use leptos::prelude::*;
use leptos_router::NavigateOptions;
use leptos_router::hooks::use_navigate;

use super::event_target_checked;
use crate::build_profile::BuildProfile;
use crate::commands;
use crate::components::Notification;

// ── Experimental section ──

/// Which opt-in a row owns.
///
/// The command takes one `Option` per flag and applies only what it is sent, so
/// a row has to name its own slot and leave the rest `None`: a save must never
/// reset a flag the row does not own.
#[derive(Clone, Copy)]
enum Flag {
    EntirePackageSync,
    MainPageV2,
    PackagePageV2,
}

impl Flag {
    fn only(self, value: bool) -> (Option<bool>, Option<bool>, Option<bool>) {
        let value = Some(value);
        match self {
            Self::EntirePackageSync => (value, None, None),
            Self::MainPageV2 => (None, value, None),
            Self::PackagePageV2 => (None, None, value),
        }
    }
}

/// Opt-ins for behaviour still being designed.
///
/// Each row gates a *control* or a *page*, never behaviour already running:
/// ticking one reveals something, and unticking it leaves whatever was written
/// behind it written, so nothing a reader chose is destroyed by undoing a
/// setting.
///
/// The rows are independent of each other. The construction gate is drawn only
/// under a development profile — absent rather than disabled, because a disabled
/// row still tells a reader the unfinished page exists. Hiding it is safe only
/// because `main.rs`'s `package_page_v2` refuses a stored value in a release
/// build too, so the row's absence leaves no live flag behind it.
#[component]
pub(super) fn ExperimentalSection(
    entire_package_sync: bool,
    main_page_v2: bool,
    package_page_v2: bool,
    /// Which build this is. A parameter rather than a `cfg!` here, so a test can
    /// draw the section as a release build draws it — see `build_profile.rs`.
    profile: BuildProfile,
    notification: RwSignal<Option<Notification>>,
    refetch: Trigger,
) -> impl IntoView {
    view! {
        <section class="settings-section qui-experimental-settings">
            <h2 class="section-title">"Experimental"</h2>
            <dl class="settings-list">
                <ExperimentalToggle
                    label="Enable entire-package sync"
                    description="Adds a per-package option to sync everything in a package, \
                                 instead of picking files."
                    enabled=entire_package_sync
                    flag=Flag::EntirePackageSync
                    notification=notification
                    refetch=refetch
                />
                // Both design rows below carry `navigate_to="/"`. `/` is the route
                // that asks which design generation is on, and saving the flag
                // alone changed nothing on screen — which looked like it needed an
                // app restart. The construction gate needs it for the same reason:
                // it implies the generation, so it changes what `/` renders too
                // (`main.rs`'s `effective_design`).
                <ExperimentalToggle
                    label="New design preview"
                    description="The redesigned QuiltSync, wherever it is ready."
                    enabled=main_page_v2
                    flag=Flag::MainPageV2
                    notification=notification
                    refetch=refetch
                    navigate_to="/"
                />
                {profile
                    .allows_construction_gates()
                    .then(|| {
                        view! {
                            <ExperimentalToggle
                                label="Unfinished package page"
                                description="The rebuilt package screen, still a placeholder. \
                                             Also turns on New design preview."
                                enabled=package_page_v2
                                flag=Flag::PackagePageV2
                                notification=notification
                                refetch=refetch
                                navigate_to="/"
                            />
                        }
                    })}
            </dl>
        </section>
    }
}

/// One opt-in: its own `dt`/`dd` pair in the section's list.
///
/// The pair is wrapped, because a row is a thing a test has to be able to reach
/// from any part of it — `data-settings-row` is that handle, and the wrapper is
/// `display: contents` in `pages/settings.css` so the `dt` and `dd` stay items of
/// the section's grid.
///
/// `navigate_to` is for a flag whose effect is not on this screen, so saving it
/// otherwise looks like nothing happened. A row without one saves in place.
#[component]
fn ExperimentalToggle(
    label: &'static str,
    description: &'static str,
    /// What is stored. The row owns the live signal over it: no row reads
    /// another's state, so there is nothing for the section to hold.
    enabled: bool,
    flag: Flag,
    notification: RwSignal<Option<Notification>>,
    refetch: Trigger,
    #[prop(optional)] navigate_to: Option<&'static str>,
) -> impl IntoView {
    let enabled = RwSignal::new(enabled);
    let saving = RwSignal::new(false);
    let navigate = use_navigate();

    let on_toggle = move |ev: leptos::ev::Event| {
        let new_enabled = event_target_checked(&ev);
        if saving.get_untracked() {
            return;
        }
        saving.set(true);
        enabled.set(new_enabled);
        let navigate = navigate.clone();
        leptos::task::spawn_local(async move {
            let (entire_package_sync, main_page_v2, package_page_v2) = flag.only(new_enabled);
            match commands::update_experimental_settings(
                entire_package_sync,
                main_page_v2,
                package_page_v2,
            )
            .await
            {
                Ok(()) => {
                    notification.set(Some(Notification::Success(
                        "Experimental settings saved".into(),
                    )));
                    refetch.notify();
                    if let Some(route) = navigate_to {
                        navigate(route, NavigateOptions::default());
                    }
                }
                Err(e) => {
                    // Revert the optimistic toggle so the UI doesn't drift
                    // from on-disk state.
                    enabled.set(!new_enabled);
                    notification.set(Some(Notification::Error(e)));
                }
            }
            saving.set(false);
        });
    };

    view! {
        <div data-settings-row>
            <dt>{label}</dt>
            <dd>
                <label class="checkbox-option">
                    <input
                        type="checkbox"
                        prop:checked=move || enabled.get()
                        prop:disabled=move || saving.get()
                        on:change=on_toggle
                    />
                    <span class="value default">{description}</span>
                </label>
            </dd>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{element_saying, mount};
    use wasm_bindgen_test::*;

    /// The section as a given build draws it, over the two stored answers.
    ///
    /// Inside a `Router` because every row asks for `use_navigate`. The
    /// notification signal and the trigger are the section's own required
    /// wiring; no assertion here reads either.
    fn mount_section(profile: BuildProfile, preview: bool, construction: bool) -> web_sys::Element {
        mount(move || {
            view! {
                <leptos_router::components::Router>
                    <ExperimentalSection
                        entire_package_sync=false
                        main_page_v2=preview
                        package_page_v2=construction
                        profile=profile
                        notification=RwSignal::new(None)
                        refetch=Trigger::new()
                    />
                </leptos_router::components::Router>
            }
        })
    }

    /// The reader sees one design switch, named for what it selects rather than
    /// for the page that introduced it.
    #[wasm_bindgen_test]
    fn the_reader_row_is_named_for_the_design_not_the_page() {
        let root = mount_section(BuildProfile::Release, false, false);
        element_saying(&root, "New design preview");
        assert!(
            !root
                .text_content()
                .unwrap_or_default()
                .contains("New main page"),
            "the old page-named label is gone"
        );
    }

    /// Absent, not disabled. A disabled row still tells a reader the unfinished
    /// page exists and invites the question; the construction gate is not theirs
    /// to know about.
    #[wasm_bindgen_test]
    fn a_release_build_does_not_draw_the_construction_row() {
        let root = mount_section(BuildProfile::Release, false, true);
        assert!(
            !root
                .text_content()
                .unwrap_or_default()
                .contains("Unfinished package page"),
            "a release build draws no construction gate, even with the value stored"
        );
    }

    /// And a development build draws it free of the other row: the dependency
    /// that disabled it encoded the nesting this change inverts.
    #[wasm_bindgen_test]
    fn a_development_build_draws_it_independent_of_the_preview() {
        let root = mount_section(BuildProfile::Development, false, false);
        let row = element_saying(&root, "Unfinished package page");
        let input = row
            .closest("[data-settings-row]")
            .unwrap()
            .expect("the row wraps its control")
            .query_selector("input")
            .unwrap()
            .expect("the row carries a checkbox");
        assert!(!input.has_attribute("disabled"), "with the preview off");
    }
}
