use leptos::prelude::*;

use super::event_target_checked;
use crate::commands::{self, AutosyncSettingsData};
use crate::components::Notification;
use crate::components::buttons;

// ── Autosync section ──

#[component]
pub(super) fn AutosyncSection(
    autosync: AutosyncSettingsData,
    notification: RwSignal<Option<Notification>>,
    refetch: Trigger,
) -> impl IntoView {
    let show_popup = RwSignal::new(false);
    let pull_display = if autosync.pull_enabled { "On" } else { "Off" };
    let push_display = if autosync.push_enabled { "On" } else { "Off" };
    let pull_interval = autosync.pull_interval_secs;
    let idle_timeout = autosync.idle_timeout_secs;
    let close_to_tray_display = if autosync.close_to_tray { "On" } else { "Off" };
    let current = autosync;

    view! {
        <section class="settings-section qui-autosync-settings">
            <h2 class="section-title">"Autosync"</h2>
            <p class="section-description">
                "Autosync periodically refreshes installed remote packages and, if you opt in, publishes mapped folders that have local changes or a pending commit. The two directions are toggled independently — many users want background pulls without unattended pushes."
            </p>
            <dl class="settings-list">
                <dt>"Pull (remote → local)"</dt>
                <dd><span class="value">{pull_display}</span></dd>

                <dt>"Push (local → remote)"</dt>
                <dd><span class="value">{push_display}</span></dd>

                <dt>"Pull interval"</dt>
                <dd><span class="value">{format!("{pull_interval} s")}</span></dd>

                <dt>"Wait after last edit before publishing"</dt>
                <dd><span class="value">{format!("{idle_timeout} s")}</span></dd>

                <dt>"Close to tray"</dt>
                <dd><span class="value">{close_to_tray_display}</span></dd>
            </dl>
            <div class="settings-actions">
                <button
                    type="button"
                    class="qui-button"
                    on:click=move |_| show_popup.set(true)
                >
                    <span>"Edit"</span>
                </button>
            </div>
        </section>

        <Show when=move || show_popup.get()>
            <AutosyncSettingsPopup
                current=current.clone()
                notification=notification
                refetch=refetch
                on_close=move || show_popup.set(false)
            />
        </Show>
    }
}

#[component]
fn AutosyncSettingsPopup(
    current: AutosyncSettingsData,
    notification: RwSignal<Option<Notification>>,
    refetch: Trigger,
    on_close: impl Fn() + Clone + 'static,
) -> impl IntoView {
    let pull_enabled = RwSignal::new(current.pull_enabled);
    let push_enabled = RwSignal::new(current.push_enabled);
    let pull_interval_secs = RwSignal::new(current.pull_interval_secs.to_string());
    let idle_timeout_secs = RwSignal::new(current.idle_timeout_secs.to_string());
    let close_to_tray = RwSignal::new(current.close_to_tray);
    let parse_error = RwSignal::new(None::<String>);
    let saving = RwSignal::new(false);

    let on_close_save = on_close.clone();
    let on_save = move |_: leptos::ev::MouseEvent| {
        if saving.get_untracked() {
            return;
        }
        let pull_interval = match pull_interval_secs.get_untracked().trim().parse::<u64>() {
            Ok(n) if n >= 1 => n,
            _ => {
                parse_error.set(Some("Pull interval must be a positive integer".to_string()));
                return;
            }
        };
        let idle_timeout = match idle_timeout_secs.get_untracked().trim().parse::<u64>() {
            Ok(n) if n >= 1 => n,
            _ => {
                parse_error.set(Some(
                    "Wait after last edit must be a positive integer".to_string(),
                ));
                return;
            }
        };
        parse_error.set(None);
        saving.set(true);
        let on_close = on_close_save.clone();
        let settings = AutosyncSettingsData {
            pull_enabled: pull_enabled.get_untracked(),
            push_enabled: push_enabled.get_untracked(),
            pull_interval_secs: pull_interval,
            idle_timeout_secs: idle_timeout,
            close_to_tray: close_to_tray.get_untracked(),
        };
        leptos::task::spawn_local(async move {
            match commands::update_autosync_settings(settings).await {
                Ok(()) => {
                    notification.set(Some(Notification::Success(
                        "Autosync settings saved".into(),
                    )));
                    on_close();
                    refetch.notify();
                }
                Err(e) => notification.set(Some(Notification::Error(e))),
            }
            saving.set(false);
        });
    };

    let on_reset = move |_: leptos::ev::MouseEvent| {
        let defaults = AutosyncSettingsData::default();
        pull_enabled.set(defaults.pull_enabled);
        push_enabled.set(defaults.push_enabled);
        pull_interval_secs.set(defaults.pull_interval_secs.to_string());
        idle_timeout_secs.set(defaults.idle_timeout_secs.to_string());
        close_to_tray.set(defaults.close_to_tray);
        parse_error.set(None);
    };

    let on_close_cancel = on_close.clone();
    let on_cancel = move |_: leptos::ev::MouseEvent| on_close_cancel();

    view! {
        <div class="popup-overlay" on:click={
            let on_close = on_close.clone();
            move |_| on_close()
        }>
            <div class="popup-content autosync-settings-form" on:click=|ev| ev.stop_propagation()>
                <h2 class="section-title">"Edit autosync settings"</h2>

                <div class="field">
                    <label class="checkbox-option">
                        <input
                            type="checkbox"
                            prop:checked=move || pull_enabled.get()
                            on:change=move |ev| pull_enabled.set(event_target_checked(&ev))
                        />
                        "Auto-pull updates from the remote"
                    </label>
                    <p class="field-description">
                        "When a tracked package's remote moves ahead and your working tree has no local changes, pull automatically. Cheap and idempotent."
                    </p>
                </div>

                <div class="field">
                    <label class="checkbox-option">
                        <input
                            type="checkbox"
                            prop:checked=move || push_enabled.get()
                            on:change=move |ev| push_enabled.set(event_target_checked(&ev))
                        />
                        "Auto-publish local changes"
                    </label>
                    <p class="field-description">
                        "When you save files in a mapped folder, commit and push automatically once the working tree is quiet. Uses the publish-settings template / metadata / workflow above. Refuses on diverged or foreign-remote conflicts."
                    </p>
                </div>

                <div class="field">
                    <label for="autosync-pull-interval-secs">"Pull interval (seconds)"</label>
                    <input
                        class="input"
                        id="autosync-pull-interval-secs"
                        type="number"
                        min="1"
                        prop:value=move || pull_interval_secs.get()
                        on:input=move |ev| pull_interval_secs.set(event_target_value(&ev))
                    />
                </div>

                <div class="field">
                    <label for="autosync-idle-timeout-secs">"Wait after last edit before publishing (seconds)"</label>
                    <input
                        class="input"
                        id="autosync-idle-timeout-secs"
                        type="number"
                        min="1"
                        prop:value=move || idle_timeout_secs.get()
                        on:input=move |ev| idle_timeout_secs.set(event_target_value(&ev))
                    />
                </div>

                <div class="field">
                    <label class="checkbox-option">
                        <input
                            type="checkbox"
                            prop:checked=move || close_to_tray.get()
                            on:change=move |ev| close_to_tray.set(event_target_checked(&ev))
                        />
                        "Close to tray (keep autosync running when the window is closed)"
                    </label>
                    <p class="field-description">
                        "Leave off on systems without a working tray (e.g. stock GNOME without a tray extension)."
                    </p>
                </div>

                <Show when=move || parse_error.get().is_some()>
                    <span class="error">{move || parse_error.get().unwrap_or_default()}</span>
                </Show>

                <div class="popup-actions">
                    <buttons::FormPrimary on_click=on_save disabled=saving>
                        "Save"
                    </buttons::FormPrimary>
                    <buttons::FormSecondary on_click=on_cancel />
                    <button type="button" class="qui-button link" on:click=on_reset>
                        <span>"Reset to defaults"</span>
                    </button>
                </div>
            </div>
        </div>
    }
}
