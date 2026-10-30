use leptos::prelude::*;
use leptos_router::NavigateOptions;
use leptos_router::hooks::use_navigate;

use super::event_target_checked;
use crate::commands::{self, Design};
use crate::components::Notification;

// ── Experimental section ──

/// Which setting a row owns.
///
/// *New design* sits here, where readers know it from the preview, but it
/// writes the design setting rather than an experiment.
#[derive(Clone, Copy)]
enum Flag {
    EntirePackageSync,
    NewDesign,
}

impl Flag {
    /// Save `value` to the setting this row owns, and only that one.
    async fn save(self, value: bool) -> Result<(), String> {
        match self {
            Self::EntirePackageSync => {
                commands::update_experimental_settings(Some(value), None).await
            }
            Self::NewDesign => {
                let design = if value { Design::V2 } else { Design::V1 };
                commands::update_design_settings(design).await
            }
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
/// Every row here is a reader's to find, so work with nothing to show them yet
/// gets no row at all. A page of the redesign gets none of its own either: it
/// arrives under *New design*, which is on unless the reader turned it off,
/// rather than as one more row to find.
#[component]
pub(super) fn ExperimentalSection(
    entire_package_sync: bool,
    new_design: bool,
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
                // `navigate_to="/"`: `/` is the route that asks which design
                // generation is on, and saving the flag alone changed nothing on
                // screen — which looked like it needed an app restart.
                <ExperimentalToggle
                    label="New design"
                    description="The redesigned QuiltSync."
                    enabled=new_design
                    flag=Flag::NewDesign
                    notification=notification
                    refetch=refetch
                    navigate_to="/"
                />
            </dl>
        </section>
    }
}

/// One opt-in: its own `dt`/`dd` pair in the section's list.
///
/// A bare pair, with nothing around it: the section's list is a two-column grid,
/// and a wrapper would take the row out of it.
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
            match flag.save(new_enabled).await {
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{element_saying, mount};
    use wasm_bindgen_test::*;

    /// The section over its stored answers.
    ///
    /// Inside a `Router` because every row asks for `use_navigate`. The
    /// notification signal and the trigger are the section's own required
    /// wiring; no assertion here reads either.
    fn mount_section(preview: bool) -> web_sys::Element {
        mount(move || {
            view! {
                <leptos_router::components::Router>
                    <ExperimentalSection
                        entire_package_sync=false
                        new_design=preview
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
        let root = mount_section(false);
        element_saying(&root, "New design");
        let words = root.text_content().unwrap_or_default();
        assert!(
            !words.contains("New main page"),
            "the old page-named label is gone"
        );
        assert!(
            !words.contains("Unfinished package page"),
            "and the rebuilt package screen is not offered at all"
        );
    }
}
