use leptos::prelude::*;
use leptos_router::NavigateOptions;
use leptos_router::hooks::use_navigate;

use super::event_target_checked;
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
#[component]
pub(super) fn ExperimentalSection(
    entire_package_sync: bool,
    main_page_v2: bool,
    package_page_v2: bool,
    notification: RwSignal<Option<Notification>>,
    refetch: Trigger,
) -> impl IntoView {
    // The section owns the signals rather than each row, because the package row
    // reads the main-page row's live state and not the value the page loaded with.
    let entire_package_sync = RwSignal::new(entire_package_sync);
    let main_page_v2 = RwSignal::new(main_page_v2);
    let package_page_v2 = RwSignal::new(package_page_v2);

    view! {
        <section class="settings-section qui-experimental-settings">
            <h2 class="section-title">"Experimental"</h2>
            <dl class="settings-list">
                <ExperimentalToggle
                    label="Enable entire-package sync"
                    description="Adds a per-package choice — sync the entire package, including \
                                 files added later, instead of picking files. Off until you \
                                 choose it on a package."
                    enabled=entire_package_sync
                    flag=Flag::EntirePackageSync
                    notification=notification
                    refetch=refetch
                />
                <ExperimentalToggle
                    label="New main page"
                    description="One page for everything that needs you, over separate package \
                                 screens. Switch back at any time — nothing is lost."
                    enabled=main_page_v2
                    flag=Flag::MainPageV2
                    notification=notification
                    refetch=refetch
                    // `/` is the one route that asks which main page is on, and
                    // saving the flag alone changed nothing on screen — which
                    // looked like it needed an app restart.
                    navigate_to="/"
                />
                <ExperimentalToggle
                    label="New package page"
                    description="The package screen, being rebuilt. Unfinished — today it is a \
                                 placeholder, not the screen you know. Switch back at any time \
                                 — nothing is lost."
                    enabled=package_page_v2
                    flag=Flag::PackagePageV2
                    notification=notification
                    refetch=refetch
                    disabled_when=Signal::derive(move || !main_page_v2.get())
                    disabled_note="Turn on New main page first — the rebuilt screen is drawn in \
                                   that design and leads back to that page."
                />
            </dl>
        </section>
    }
}

/// One opt-in: its own `dt`/`dd` pair in the section's list.
///
/// `navigate_to` is for a flag whose effect is not on this screen, so saving it
/// otherwise looks like nothing happened. A row without one saves in place.
#[component]
fn ExperimentalToggle(
    label: &'static str,
    description: &'static str,
    enabled: RwSignal<bool>,
    flag: Flag,
    notification: RwSignal<Option<Notification>>,
    refetch: Trigger,
    #[prop(optional)] navigate_to: Option<&'static str>,
    /// When this row cannot be operated, and why. A disabled control with no
    /// reason beside it is a dead end, so the two arrive together.
    ///
    /// The box keeps showing what is stored while it is disabled: the flag is
    /// not cleared, so re-enabling what gates it resumes the reader's choice.
    #[prop(optional)]
    disabled_when: Option<Signal<bool>>,
    #[prop(optional)] disabled_note: Option<&'static str>,
) -> impl IntoView {
    let saving = RwSignal::new(false);
    let navigate = use_navigate();
    let blocked = move || disabled_when.is_some_and(|when| when.get());

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
        <dt>{label}</dt>
        <dd>
            <label class="checkbox-option">
                <input
                    type="checkbox"
                    prop:checked=move || enabled.get()
                    prop:disabled=move || saving.get() || blocked()
                    on:change=on_toggle
                />
                <span class="value default">
                    {description}
                    {disabled_note
                        .map(|note| {
                            view! { <Show when=blocked><span class="note">{note}</span></Show> }
                        })}
                </span>
            </label>
        </dd>
    }
}
