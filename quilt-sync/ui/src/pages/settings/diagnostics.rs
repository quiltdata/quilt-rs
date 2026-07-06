use leptos::prelude::*;

use crate::commands;
use crate::components::Notification;
use crate::components::buttons;

// ── Diagnostics section ──

#[component]
pub(super) fn DiagnosticsSection(
    version: String,
    os: String,
    log_level: String,
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
                <dd>{log_level}</dd>

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
                    "Sends app version, OS, directory paths, authenticated host names, log files, and OAuth client IDs."
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
