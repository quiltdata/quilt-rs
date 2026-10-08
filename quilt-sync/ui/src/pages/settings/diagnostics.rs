use leptos::prelude::*;

use crate::commands::{self, LogEnv};
use crate::components::Notification;
use crate::components::buttons;
use crate::kit::{Naming, Select};
use crate::pages::settings_v2::{LOG_LEVELS, log_env_hint, log_level_label};

/// The saved log level. Disabled while a valid `QUILT_LOG` replaces it.
#[component]
fn LogLevelField(
    log_level: String,
    log_env: LogEnv,
    notification: RwSignal<Option<Notification>>,
) -> impl IntoView {
    let level = RwSignal::new(log_level_label(log_level));
    let overridden = matches!(log_env, LogEnv::Overrides(_));
    // Disabled while a save is in flight, so saves land in the order they were chosen.
    let saving = RwSignal::new(false);
    let env_hint = log_env_hint(&log_env);
    Effect::watch(
        move || level.get(),
        move |label, _, _| {
            let label = label.to_lowercase();
            saving.set(true);
            leptos::task::spawn_local(async move {
                match commands::update_log_settings(label).await {
                    Ok(()) => notification.set(Some(Notification::Success(
                        "Log level saved; it applies after a restart".into(),
                    ))),
                    Err(e) => notification.set(Some(Notification::Error(e))),
                }
                saving.set(false);
            });
        },
        false,
    );

    view! {
        <Select
            naming=Naming::Hidden("Log level".to_string())
            options=LOG_LEVELS.iter().map(ToString::to_string).collect()
            selected=level
            disabled=Signal::derive(move || overridden || saving.get())
        />
        <span class="value default">"Applies after a restart."</span>
        {env_hint.map(|hint| view! { <span class="value default">{hint}</span> })}
    }
}

// ── Diagnostics section ──

#[component]
pub(super) fn DiagnosticsSection(
    version: String,
    os: String,
    log_level: String,
    log_env: LogEnv,
    logs_dir: String,
    logs_dir_is_temporary: bool,
    notification: RwSignal<Option<Notification>>,
    zip_path: RwSignal<Option<String>>,
) -> impl IntoView {
    let collecting = RwSignal::new(false);
    let logs_title = logs_dir.clone();

    view! {
        <section class="settings-section">
            <h2 class="section-title">"Diagnostics"</h2>
            <dl class="settings-list">
                <dt>"Log level"</dt>
                <dd>
                    <LogLevelField log_level=log_level log_env=log_env notification=notification />
                </dd>

                <dt>"Logs directory"</dt>
                <dd>
                    <span class="path" title=logs_title>{logs_dir}</span>
                    <buttons::OpenLogsDir
                        on_click=move |_| {
                            leptos::task::spawn_local(async move {
                                match commands::debug_logs().await {
                                    Ok(msg) => notification.set(Some(Notification::Success(msg))),
                                    Err(e) => {
                                        notification
                                            .set(Some(Notification::Error(e)));
                                    }
                                }
                            });
                        }
                        is_temporary=logs_dir_is_temporary
                    />
                </dd>
            </dl>

            <div class="settings-actions" id="diagnostic-actions">
                // Collect Logs
                <buttons::CollectLogs
                    on_click=move |_| {
                        collecting.set(true);
                        leptos::task::spawn_local(async move {
                            match commands::collect_diagnostic_logs().await {
                                Ok(path) => zip_path.set(Some(path)),
                                Err(e) => {
                                    web_sys::console::error_1(
                                        &format!("Failed to collect logs: {e}").into(),
                                    );
                                }
                            }
                            collecting.set(false);
                        });
                    }
                    busy=collecting
                />

                <span class="actions-divider">"then"</span>

                // Send to Sentry
                <buttons::SendToSentry
                    on_click=move |_| {
                        if let Some(path) = zip_path.get_untracked() {
                            leptos::task::spawn_local(async move {
                                match commands::send_crash_report(path).await {
                                    Ok(msg) => notification.set(Some(Notification::Success(msg))),
                                    Err(e) => {
                                        notification
                                            .set(Some(Notification::Error(e)));
                                    }
                                }
                            });
                        }
                    }
                    disabled=Signal::derive(move || zip_path.get().is_none())
                />

                <span class="actions-divider">"or"</span>

                // Email Support
                <EmailSupportButton version=version os=os zip_path=zip_path />

                // Collected logs result
                <Show when=move || zip_path.get().is_some()>
                    <div class="collect-logs-result">
                        <span class="zip-path-label">"Logs collected:"</span>
                        <code>{move || zip_path.get().unwrap_or_default()}</code>
                        <buttons::Reveal
                            on_click=move |_| {
                                if let Some(path) = zip_path.get_untracked() {
                                    leptos::task::spawn_local(async move {
                                        let sep = path.rfind('/').or_else(|| path.rfind('\\'));
                                        let dir = match sep {
                                            Some(i) if i > 0 => path[..i].to_string(),
                                            _ => path,
                                        };
                                        let _ = commands::open_in_web_browser(dir).await;
                                    });
                                }
                            }
                            small=true
                            link=true
                        />
                    </div>
                </Show>

                <p class="crash-report-description">
                    "Sends app version, OS, directory paths, authenticated host names, log files, OAuth client IDs, and this installation's anonymous ID."
                </p>
            </div>
        </section>
    }
}

#[component]
fn EmailSupportButton(
    version: String,
    os: String,
    zip_path: RwSignal<Option<String>>,
) -> impl IntoView {
    view! {
        <buttons::EmailSupport
            on_click=move |_| {
                if let Some(path) = zip_path.get_untracked() {
                    let version = version.clone();
                    let os = os.clone();
                    leptos::task::spawn_local(async move {
                        let subject_raw = format!("Quilt issue report (v{version}, {os})");
                        let body_raw = format!(
                            "Please describe the issue:\n...\n\nDiagnostic logs saved to:\n{path}\nPlease attach this file to this email."
                        );
                        let mailto = format!(
                            "mailto:support@quilt.bio?subject={}&body={}",
                            urlencoding::encode(&subject_raw),
                            urlencoding::encode(&body_raw),
                        );
                        let _ = commands::open_in_web_browser(mailto).await;
                    });
                }
            }
            disabled=Signal::derive(move || zip_path.get().is_none())
        />
    }
}
