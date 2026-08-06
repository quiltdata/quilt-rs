use leptos::prelude::*;

use super::event_target_checked;
use crate::commands;
use crate::components::Notification;

// ── Experimental section ──

/// Opt-ins for behaviour still being designed.
///
/// The row here is a gate on a *control*, not on behaviour: ticking it reveals
/// the sync-scope choice on the package screen and downloads nothing by itself,
/// which is why the label names the capability rather than an outcome. Unticking
/// it stops the scope being honoured but leaves each package's choice written,
/// so ticking it again resumes them.
#[component]
pub(super) fn ExperimentalSection(
    entire_package_sync: bool,
    notification: RwSignal<Option<Notification>>,
    refetch: Trigger,
) -> impl IntoView {
    let enabled = RwSignal::new(entire_package_sync);
    let saving = RwSignal::new(false);

    let on_toggle = move |ev: leptos::ev::Event| {
        let new_enabled = event_target_checked(&ev);
        if saving.get_untracked() {
            return;
        }
        saving.set(true);
        enabled.set(new_enabled);
        leptos::task::spawn_local(async move {
            match commands::update_experimental_settings(new_enabled).await {
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
        <section class="settings-section qui-experimental-settings">
            <h2 class="section-title">"Experimental"</h2>
            <dl class="settings-list">
                <dt>"Enable entire-package sync"</dt>
                <dd>
                    <label class="checkbox-option">
                        <input
                            type="checkbox"
                            prop:checked=move || enabled.get()
                            prop:disabled=move || saving.get()
                            on:change=on_toggle
                        />
                        <span class="value default">
                            "Adds a per-package choice — sync the entire package, including \
                             files added later, instead of picking files. Off until you \
                             choose it on a package."
                        </span>
                    </label>
                </dd>
            </dl>
        </section>
    }
}
