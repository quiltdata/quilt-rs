use leptos::prelude::*;

use crate::commands::{self, ChangelogEntry, SettingsData};
use crate::components::layout::{BreadcrumbItem, BreadcrumbLink};
use crate::components::{Layout, Notification, Spinner};
use crate::util::urlencoding;

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
                    <button
                        class="qui-button link"
                        type="button"
                        on:click=move |_| show_release_notes.set(true)
                    >
                        <span>"Release notes"</span>
                    </button>
                </dd>

                <dt>"Home directory"</dt>
                <dd>
                    <span class="path" title=home_title>{home_display}</span>
                    <button
                        class="qui-button link small"
                        type="button"
                        on:click=on_open_home
                    >
                        <img class="qui-icon" src="/assets/img/icons/folder_open.svg" />
                        <span>"Open"</span>
                    </button>
                </dd>

                <dt>"Data directory"</dt>
                <dd>
                    <span class="path" title=data_title>{data_dir}</span>
                    <button
                        class="qui-button link small"
                        type="button"
                        on:click=on_open_data
                    >
                        <img class="qui-icon" src="/assets/img/icons/folder_open.svg" />
                        <span>"Open"</span>
                    </button>
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
    let back_encoded = urlencoding("/settings");
    let login_href = format!("/login?host={}&back={back_encoded}", urlencoding(&host));

    view! {
        <dt>{host_display}</dt>
        <dd>
            <a class="qui-button small" href=login_href>
                <span>"Re-login"</span>
            </a>
            <div class="logout-popover">
                <button
                    class="qui-button small"
                    type="button"
                    on:click=move |_| {
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
                >
                    <img class="qui-icon" src="/assets/img/icons/warning.svg" />
                    <span>"Logout"</span>
                </button>
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
    let logs_icon = if logs_dir_is_temporary {
        "/assets/img/icons/warning.svg"
    } else {
        "/assets/img/icons/folder_open.svg"
    };
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
                    <button
                        class="qui-button link small"
                        type="button"
                        on:click=move |_| {
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
                    >
                        <img class="qui-icon" src=logs_icon />
                        <span>"Open"</span>
                    </button>
                </dd>
            </dl>

            <div class="settings-actions" id="diagnostic-actions">
                // Collect Logs
                <button
                    class="qui-button"
                    type="button"
                    prop:disabled=move || collecting.get()
                    on:click=move |_| {
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
                >
                    <span>
                        {move || {
                            if collecting.get() { "Collecting\u{2026}" } else { "Collect Logs" }
                        }}
                    </span>
                </button>

                <span class="actions-divider">"then"</span>

                // Send to Sentry
                <button
                    class="qui-button"
                    type="button"
                    prop:disabled=move || zip_path.get().is_none()
                    on:click=move |_| {
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
                >
                    <span>"Send to Sentry"</span>
                </button>

                <span class="actions-divider">"or"</span>

                // Email Support
                <EmailSupportButton version=version os=os zip_path=zip_path />

                // Collected logs result
                <Show when=move || zip_path.get().is_some()>
                    <div class="collect-logs-result">
                        <span class="zip-path-label">"Logs collected:"</span>
                        <code>{move || zip_path.get().unwrap_or_default()}</code>
                        <button
                            class="qui-button link small"
                            type="button"
                            on:click=move |_| {
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
                        >
                            <img class="qui-icon" src="/assets/img/icons/folder_open.svg" />
                            <span>"Reveal"</span>
                        </button>
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
        <button
            class="qui-button"
            type="button"
            prop:disabled=move || zip_path.get().is_none()
            on:click=move |_| {
                if let Some(path) = zip_path.get_untracked() {
                    let version = version.clone();
                    let os = os.clone();
                    leptos::task::spawn_local(async move {
                        let subject =
                            urlencoding(&format!("Quilt issue report (v{version}, {os})"));
                        let body = urlencoding(&format!(
                            "Please describe the issue:\n...\n\nDiagnostic logs saved to:\n{path}\nPlease attach this file to this email."
                        ));
                        let mailto =
                            format!("mailto:support@quilt.bio?subject={subject}&body={body}");
                        let _ = commands::open_in_web_browser(mailto).await;
                    });
                }
            }
        >
            <span>"Email Support"</span>
        </button>
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
