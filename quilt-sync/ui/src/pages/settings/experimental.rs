use leptos::prelude::*;

use super::event_target_checked;
use crate::commands;
use crate::components::Notification;

// ── Experimental section ──

/// Opt-ins for behaviour still being designed.
///
/// Each row gates a *control*, never behaviour already running: ticking one
/// reveals something, and unticking it leaves whatever was written behind it
/// written, so nothing a reader chose is destroyed by undoing a setting.
///
/// Every row here is a reader's to find, so work with nothing to show them yet
/// gets no row at all. The design is not one of them: it has a section of its
/// own.
#[component]
pub(super) fn ExperimentalSection(
    entire_package_sync: bool,
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
                    notification=notification
                    refetch=refetch
                />
            </dl>
        </section>
    }
}

/// One opt-in: its own `dt`/`dd` pair in the section's list.
///
/// A bare pair, with nothing around it: the section's list is a two-column grid,
/// and a wrapper would take the row out of it.
#[component]
fn ExperimentalToggle(
    label: &'static str,
    description: &'static str,
    /// What is stored. The row owns the live signal over it.
    enabled: bool,
    notification: RwSignal<Option<Notification>>,
    refetch: Trigger,
) -> impl IntoView {
    let enabled = RwSignal::new(enabled);
    let saving = RwSignal::new(false);

    let on_toggle = move |ev: leptos::ev::Event| {
        let new_enabled = event_target_checked(&ev);
        if saving.get_untracked() {
            return;
        }
        saving.set(true);
        enabled.set(new_enabled);
        leptos::task::spawn_local(async move {
            match commands::update_experimental_settings(Some(new_enabled)).await {
                Ok(()) => {
                    notification.set(Some(Notification::Success(
                        "Experimental settings saved".into(),
                    )));
                    refetch.notify();
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
    use crate::test_support::mount;
    use wasm_bindgen_test::*;

    /// The design row has left: it is a section of its own.
    #[wasm_bindgen_test]
    fn only_entire_package_sync_is_experimental() {
        let root = mount(|| {
            view! {
                <ExperimentalSection
                    entire_package_sync=false
                    notification=RwSignal::new(None)
                    refetch=Trigger::new()
                />
            }
        });
        let words = root.text_content().unwrap_or_default();
        assert!(words.contains("Enable entire-package sync"));
        assert!(!words.contains("design"), "{words}");
    }
}
