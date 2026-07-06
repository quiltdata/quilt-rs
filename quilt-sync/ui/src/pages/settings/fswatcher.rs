use leptos::prelude::*;

use super::event_target_checked;
use crate::commands::{
    self, FSWATCHER_SUBSCRIBER_ERROR_EVENT, FsWatcherSettingsData, SubscriberErrorEvent,
};
use crate::components::Notification;
use crate::tauri as tauri_bridge;

// ── Filesystem watcher section ──

#[component]
pub(super) fn FsWatcherSection(
    fswatcher: FsWatcherSettingsData,
    notification: RwSignal<Option<Notification>>,
    refetch: Trigger,
) -> impl IntoView {
    let enabled = RwSignal::new(fswatcher.enabled);
    let saving = RwSignal::new(false);

    // Subscriber-error listener. Only `inotify_limit` is actionable inside
    // the app (raise the sysctl limit and restart) — surface that as a
    // toast. Other kinds (e.g. `watch_lost` from a mid-session unmount) are
    // noisy and the reactor recovers via the next reconcile tick, so we
    // just log them to the browser console.
    let listener =
        tauri_bridge::listen::<SubscriberErrorEvent>(FSWATCHER_SUBSCRIBER_ERROR_EVENT, move |ev| {
            if ev.kind == "inotify_limit" {
                notification.set(Some(Notification::Error(
                    "Filesystem watcher hit the OS inotify limit. \
                     Raise it with `sudo sysctl fs.inotify.max_user_watches=524288` \
                     and restart the app."
                        .to_string(),
                )));
                return;
            }
            let ns = ev.namespace.as_deref().unwrap_or("-");
            web_sys::console::warn_1(
                &format!("fswatcher: {} [{ns}]: {}", ev.kind, ev.message).into(),
            );
        });
    on_cleanup(move || drop(listener));

    let on_toggle = move |ev: leptos::ev::Event| {
        let new_enabled = event_target_checked(&ev);
        if saving.get_untracked() {
            return;
        }
        saving.set(true);
        enabled.set(new_enabled);
        leptos::task::spawn_local(async move {
            match commands::update_fswatcher_settings(new_enabled).await {
                Ok(()) => {
                    notification.set(Some(Notification::Success(
                        "Filesystem watcher settings saved".into(),
                    )));
                    refetch.notify();
                }
                Err(e) => {
                    // Revert the optimistic toggle on error so the UI
                    // doesn't drift from on-disk state.
                    enabled.set(!new_enabled);
                    notification.set(Some(Notification::Error(e)));
                }
            }
            saving.set(false);
        });
    };

    view! {
        <section class="settings-section qui-fswatcher-settings">
            <h2 class="section-title">"Filesystem Watcher"</h2>
            <dl class="settings-list">
                <dt>"Enable filesystem watcher"</dt>
                <dd>
                    <label class="checkbox-option">
                        <input
                            type="checkbox"
                            prop:checked=move || enabled.get()
                            prop:disabled=move || saving.get()
                            on:change=on_toggle
                        />
                        <span class="value default">
                            "Refreshes local package status when files change on disk."
                        </span>
                    </label>
                </dd>
            </dl>
        </section>
    }
}
