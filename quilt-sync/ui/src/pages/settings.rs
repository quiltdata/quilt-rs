use leptos::prelude::*;

use crate::commands::{self, ChangelogEntry, PublishSettingsData, SettingsData};
use crate::components::buttons;
use crate::components::layout::{BreadcrumbItem, BreadcrumbLink};
use crate::components::{Layout, Notification, Spinner};

// ── Settings page ──

#[component]
pub fn Settings() -> impl IntoView {
    let notification = RwSignal::new(None);
    let refetch = Trigger::new();

    let data = LocalResource::new(move || {
        refetch.track();
        async { commands::get_settings_data().await }
    });

    let breadcrumbs = vec![
        BreadcrumbItem::Link(BreadcrumbLink {
            href: "/installed-packages-list".to_string(),
            title: String::new(),
        }),
        BreadcrumbItem::Current("Settings".to_string()),
    ];

    // Layout wraps Suspense here (not the other way around) because
    // breadcrumbs are static and can render immediately while data loads.
    // Pages with data-dependent breadcrumbs use Suspense outside Layout.
    view! {
        <Layout breadcrumbs=breadcrumbs notification=notification>
            <Suspense fallback=move || {
                view! { <Spinner /> }
            }>
                {move || Suspend::new(async move {
                    match data.await {
                        Ok(d) => {
                            view! { <SettingsContent data=d notification=notification refetch=refetch /> }.into_any()
                        }
                        Err(e) => {
                            crate::error_handler::handle_or_display(&e, notification)
                        }
                    }
                })}
            </Suspense>
        </Layout>
    }
}

// ── Main content (rendered after data loads) ──

#[component]
fn SettingsContent(
    data: SettingsData,
    notification: RwSignal<Option<Notification>>,
    refetch: Trigger,
) -> impl IntoView {
    let zip_path = RwSignal::new(None::<String>);

    view! {
        <div class="qui-page-settings container">
            <GeneralSection
                version=data.version.clone()
                home_dir=data.home_dir
                data_dir=data.data_dir
                changelog=data.changelog
                notification=notification
            />
            <PublishSection publish=data.publish notification=notification refetch=refetch />
            <AccountSection auth_hosts=data.auth_hosts notification=notification refetch=refetch />
            <DiagnosticsSection
                version=data.version
                os=data.os
                log_level=data.log_level
                logs_dir=data.logs_dir
                logs_dir_is_temporary=data.logs_dir_is_temporary
                notification=notification
                zip_path=zip_path
            />
        </div>
    }
}

// ── General section ──

#[component]
fn GeneralSection(
    version: String,
    home_dir: Option<String>,
    data_dir: String,
    changelog: Vec<ChangelogEntry>,
    notification: RwSignal<Option<Notification>>,
) -> impl IntoView {
    let home_display = home_dir.unwrap_or_else(|| "Not set".to_string());
    let home_title = home_display.clone();
    let data_title = data_dir.clone();
    let show_release_notes = RwSignal::new(false);

    let on_open_home = move |_| {
        leptos::task::spawn_local(async move {
            match commands::open_home_dir().await {
                Ok(msg) => notification.set(Some(Notification::Success(msg))),
                Err(e) => notification.set(Some(Notification::Error(e))),
            }
        });
    };

    let on_open_data = move |_| {
        leptos::task::spawn_local(async move {
            match commands::open_data_dir().await {
                Ok(msg) => notification.set(Some(Notification::Success(msg))),
                Err(e) => notification.set(Some(Notification::Error(e))),
            }
        });
    };

    view! {
        <section class="settings-section">
            <h2 class="section-title">"General"</h2>
            <dl class="settings-list">
                <dt>"Version"</dt>
                <dd>
                    <span>{version}</span>
                    <buttons::ReleaseNotes on_click=move |_| show_release_notes.set(true) />
                </dd>

                <dt>"Home directory"</dt>
                <dd>
                    <span class="path" title=home_title>{home_display}</span>
                    <buttons::OpenInFileBrowser on_click=on_open_home small=true link=true />
                </dd>

                <dt>"Data directory"</dt>
                <dd>
                    <span class="path" title=data_title>{data_dir}</span>
                    <buttons::OpenInFileBrowser on_click=on_open_data small=true link=true />
                </dd>
            </dl>
        </section>

        <Show when=move || show_release_notes.get()>
            <ReleaseNotesPopup
                changelog=changelog.clone()
                on_close=move || show_release_notes.set(false)
            />
        </Show>
    }
}

// ── Publish section ──

fn render_publish_preview(template: &str) -> String {
    if template.trim().is_empty() {
        return "Auto-generated summary of changes".to_string();
    }
    let now = js_sys::Date::new_0();
    let date = format!(
        "{:04}-{:02}-{:02}",
        now.get_full_year(),
        now.get_month() + 1,
        now.get_date()
    );
    let time = format!("{:02}:{:02}", now.get_hours(), now.get_minutes());
    let datetime = format!("{date} {time}");
    template
        .replace("{date}", &date)
        .replace("{time}", &time)
        .replace("{datetime}", &datetime)
        .replace("{namespace}", "example/package")
        .replace("{changes}", "3 files modified")
}

#[component]
fn PublishSection(
    publish: PublishSettingsData,
    notification: RwSignal<Option<Notification>>,
    refetch: Trigger,
) -> impl IntoView {
    let show_popup = RwSignal::new(false);

    let template_display = match publish.message_template.trim() {
        "" => "Default — auto-generated summary of changes".to_string(),
        t => t.to_string(),
    };
    let template_is_default = publish.message_template.trim().is_empty();

    let workflow_display = match publish.default_workflow.trim() {
        "" => "Default — bucket's workflow".to_string(),
        w => w.to_string(),
    };
    let workflow_is_default = publish.default_workflow.trim().is_empty();

    let metadata_display = match publish.default_metadata.trim() {
        "" => "Default — none".to_string(),
        m => m.to_string(),
    };
    let metadata_is_default = publish.default_metadata.trim().is_empty();

    let current = publish.clone();

    view! {
        <section class="settings-section qui-publish-settings">
            <h2 class="section-title">"Publish"</h2>
            <dl class="settings-list">
                <dt>"Message template"</dt>
                <dd>
                    <span
                        class="value"
                        class:default=template_is_default
                    >{template_display}</span>
                </dd>

                <dt>"Default workflow"</dt>
                <dd>
                    <span
                        class="value"
                        class:default=workflow_is_default
                    >{workflow_display}</span>
                </dd>

                <dt>"Default metadata"</dt>
                <dd>
                    <span
                        class="value"
                        class:default=metadata_is_default
                    >{metadata_display}</span>
                </dd>
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
            <PublishSettingsPopup
                current=current.clone()
                notification=notification
                refetch=refetch
                on_close=move || show_popup.set(false)
            />
        </Show>
    }
}

#[component]
fn PublishSettingsPopup(
    current: PublishSettingsData,
    notification: RwSignal<Option<Notification>>,
    refetch: Trigger,
    on_close: impl Fn() + Clone + 'static,
) -> impl IntoView {
    let message_template = RwSignal::new(current.message_template.clone());
    let workflow_override = RwSignal::new(current.default_workflow.clone());
    let metadata = RwSignal::new(current.default_metadata.clone());
    let use_bucket_default = RwSignal::new(current.default_workflow.trim().is_empty());
    let metadata_error = RwSignal::new(None::<String>);
    let saving = RwSignal::new(false);

    let on_close_save = on_close.clone();
    let on_save = move |_: leptos::ev::MouseEvent| {
        if saving.get_untracked() {
            return;
        }
        let template = message_template.get_untracked();
        let wf = if use_bucket_default.get_untracked() {
            String::new()
        } else {
            workflow_override.get_untracked()
        };
        let meta = metadata.get_untracked();
        if !meta.trim().is_empty() {
            if let Err(err) = serde_json::from_str::<serde_json::Value>(&meta) {
                metadata_error.set(Some(format!("Invalid JSON: {err}")));
                return;
            }
        }
        metadata_error.set(None);
        saving.set(true);
        let on_close = on_close_save.clone();
        leptos::task::spawn_local(async move {
            match commands::update_publish_settings(template, wf, meta).await {
                Ok(()) => {
                    notification.set(Some(Notification::Success(
                        "Publish settings saved".into(),
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
        message_template.set(String::new());
        workflow_override.set(String::new());
        use_bucket_default.set(true);
        metadata.set(String::new());
        metadata_error.set(None);
    };

    let on_close_cancel = on_close.clone();
    let on_cancel = move |_: leptos::ev::MouseEvent| on_close_cancel();

    view! {
        <div class="popup-overlay" on:click={
            let on_close = on_close.clone();
            move |_| on_close()
        }>
            <div class="popup-content publish-settings-form" on:click=|ev| ev.stop_propagation()>
                <h2 class="section-title">"Edit publish defaults"</h2>

                <div class="field">
                    <label for="publish-message-template">"Message template"</label>
                    <input
                        class="input"
                        id="publish-message-template"
                        placeholder="Auto-publish {date} ({changes})"
                        prop:value=move || message_template.get()
                        on:input=move |ev| message_template.set(event_target_value(&ev))
                    />
                    <p class="field-description">
                        "Placeholders: "
                        <code>"{date}"</code>" "
                        <code>"{time}"</code>" "
                        <code>"{datetime}"</code>" "
                        <code>"{namespace}"</code>" "
                        <code>"{changes}"</code>
                    </p>
                    <p class="field-description">
                        "Preview: "
                        <em>{move || render_publish_preview(&message_template.get())}</em>
                    </p>
                </div>

                <div class="field">
                    <label>"Default workflow"</label>
                    <label class="radio-option">
                        <input
                            type="radio"
                            name="publish-workflow-mode"
                            prop:checked=move || use_bucket_default.get()
                            on:change=move |_| use_bucket_default.set(true)
                        />
                        "Use the bucket's default workflow"
                    </label>
                    <label class="radio-option">
                        <input
                            type="radio"
                            name="publish-workflow-mode"
                            prop:checked=move || !use_bucket_default.get()
                            on:change=move |_| use_bucket_default.set(false)
                        />
                        "Override"
                    </label>
                    <Show when=move || !use_bucket_default.get()>
                        <input
                            class="input"
                            id="publish-workflow-override"
                            placeholder="workflow-id"
                            prop:value=move || workflow_override.get()
                            on:input=move |ev| workflow_override.set(event_target_value(&ev))
                        />
                    </Show>
                </div>

                <div class="field">
                    <label for="publish-default-metadata">"Default metadata"</label>
                    <textarea
                        class="textarea"
                        id="publish-default-metadata"
                        placeholder="{ \"source\": \"desktop\" }"
                        prop:value=move || metadata.get()
                        on:input=move |ev| metadata.set(event_target_value(&ev))
                    ></textarea>
                    <Show when=move || metadata_error.get().is_some()>
                        <span class="error">
                            {move || metadata_error.get().unwrap_or_default()}
                        </span>
                    </Show>
                </div>

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

// ── Account section ──

#[component]
fn AccountSection(
    auth_hosts: Vec<String>,
    notification: RwSignal<Option<Notification>>,
    refetch: Trigger,
) -> impl IntoView {
    view! {
        <section class="settings-section">
            <h2 class="section-title">"Auth"</h2>
            {if auth_hosts.is_empty() {
                view! { <p class="empty-state">"No authenticated hosts"</p> }.into_any()
            } else {
                view! {
                    <dl class="settings-list">
                        {auth_hosts
                            .into_iter()
                            .map(|host| {
                                view! { <AuthHostRow host=host notification=notification refetch=refetch /> }
                            })
                            .collect_view()}
                    </dl>
                }
                    .into_any()
            }}
        </section>
    }
}

#[component]
fn AuthHostRow(
    host: String,
    notification: RwSignal<Option<Notification>>,
    refetch: Trigger,
) -> impl IntoView {
    let host_display = host.clone();
    let host_for_logout = host.clone();
    let back_encoded = urlencoding::encode("/settings");
    let login_href = format!(
        "/login?host={}&back={back_encoded}",
        urlencoding::encode(&host)
    );

    view! {
        <dt>{host_display}</dt>
        <dd>
            <buttons::ReLogin href=login_href />
            <div class="qui-popover">
                <buttons::Logout
                    on_click=move |_| {
                        let host = host_for_logout.clone();
                        leptos::task::spawn_local(async move {
                            match commands::erase_auth(host).await {
                                Ok(msg) => notification.set(Some(Notification::Success(msg))),
                                Err(e) => {
                                    notification
                                        .set(Some(Notification::Error(e)))
                                }
                            }
                            refetch.notify();
                        });
                    }
                    small=true
                />
                <div class="popover-wrapper">
                    <div class="popover">
                        "This will erase stored credentials for this host. You will need to log in again."
                    </div>
                </div>
            </div>
        </dd>
    }
}

// ── Diagnostics section ──

#[component]
fn DiagnosticsSection(
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
                                            .set(Some(Notification::Error(e)))
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
                                            .set(Some(Notification::Error(e)))
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

// ── Release notes popup ──

#[component]
fn ReleaseNotesPopup(
    changelog: Vec<ChangelogEntry>,
    on_close: impl Fn() + 'static,
) -> impl IntoView {
    view! {
        <div class="popup-overlay" on:click=move |_| on_close()>
            <div
                class="popup-content release-notes"
                on:click=|ev| ev.stop_propagation()
            >
                <h2 class="section-title">"Release Notes"</h2>
                {changelog
                    .into_iter()
                    .map(|entry| {
                        view! {
                            <div class="release-notes-entry">
                                <h3>{entry.version}</h3>
                                <p>{entry.date}</p>
                                <pre>{entry.body}</pre>
                            </div>
                        }
                    })
                    .collect_view()}
            </div>
        </div>
    }
}
