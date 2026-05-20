use leptos::prelude::*;

use crate::commands::{
    self, AutosyncSettingsData, ChangelogEntry, FSWATCHER_SUBSCRIBER_ERROR_EVENT,
    FsWatcherSettingsData, PublishSettingsData, SettingsData, SubscriberErrorEvent,
};
use crate::components::buttons;
use crate::components::layout::{BreadcrumbItem, BreadcrumbLink};
use crate::components::{Layout, Notification, Spinner};
use crate::tauri as tauri_bridge;

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
            <AutosyncSection autosync=data.autosync notification=notification refetch=refetch />
            <FsWatcherSection fswatcher=data.fswatcher notification=notification refetch=refetch />
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

/// Placeholders supported by the Publish message preview.
///
/// Kept in lockstep with `PUBLISH_PLACEHOLDERS` in
/// `quilt-sync/src-tauri/src/commit_message.rs`. When adding or renaming a
/// placeholder, update both sides and the positional values passed to
/// [`apply_placeholders`] below.
const PUBLISH_PLACEHOLDERS: &[&str] =
    &["{date}", "{time}", "{datetime}", "{namespace}", "{changes}"];

fn apply_placeholders(template: &str, values: &[&str]) -> String {
    debug_assert_eq!(PUBLISH_PLACEHOLDERS.len(), values.len());
    let mut rendered = template.to_string();
    for (placeholder, value) in PUBLISH_PLACEHOLDERS.iter().zip(values) {
        rendered = rendered.replace(placeholder, value);
    }
    rendered
}

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
    apply_placeholders(
        template,
        &[
            &date,
            &time,
            &datetime,
            "example/package",
            "3 files modified",
        ],
    )
}

#[component]
fn PublishSection(
    publish: PublishSettingsData,
    notification: RwSignal<Option<Notification>>,
    refetch: Trigger,
) -> impl IntoView {
    let show_popup = RwSignal::new(false);

    let template_is_default = publish.message_template.is_empty();
    let template_display = if template_is_default {
        "Default — auto-generated summary of changes".to_string()
    } else {
        publish.message_template.clone()
    };

    let workflow_is_default = publish.default_workflow.is_empty();
    let workflow_display = if workflow_is_default {
        "Default — bucket's workflow".to_string()
    } else {
        publish.default_workflow.clone()
    };

    let metadata_is_default = publish.default_metadata.is_empty();
    let metadata_display = if metadata_is_default {
        "Default — none".to_string()
    } else {
        publish.default_metadata.clone()
    };

    let current = publish.clone();

    view! {
        <section class="settings-section qui-publish-settings">
            <h2 class="section-title">"Commit and Push"</h2>
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
    let use_bucket_default = RwSignal::new(current.default_workflow.is_empty());
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
        if !meta.trim().is_empty()
            && let Err(err) = serde_json::from_str::<serde_json::Value>(&meta)
        {
            metadata_error.set(Some(format!("Invalid JSON: {err}")));
            return;
        }
        metadata_error.set(None);
        saving.set(true);
        let on_close = on_close_save.clone();
        leptos::task::spawn_local(async move {
            match commands::update_publish_settings(template, wf, meta).await {
                Ok(()) => {
                    notification.set(Some(Notification::Success(
                        "Commit and Push settings saved".into(),
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
                <h2 class="section-title">"Edit commit defaults"</h2>

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
                        {PUBLISH_PLACEHOLDERS
                            .iter()
                            .map(|p| view! { <code>{*p}</code>" " })
                            .collect_view()}
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

// ── Autosync section ──

#[component]
fn AutosyncSection(
    autosync: AutosyncSettingsData,
    notification: RwSignal<Option<Notification>>,
    refetch: Trigger,
) -> impl IntoView {
    let show_popup = RwSignal::new(false);
    let pull_display = if autosync.pull_enabled { "On" } else { "Off" };
    let push_display = if autosync.push_enabled { "On" } else { "Off" };
    let pull_interval = autosync.pull_interval_secs;
    let idle_timeout = autosync.idle_timeout_secs;
    let current = autosync.clone();

    view! {
        <section class="settings-section qui-autosync-settings">
            <h2 class="section-title">"Background Autosync"</h2>
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

// ── Filesystem watcher section ──

#[component]
fn FsWatcherSection(
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

fn event_target_checked(ev: &leptos::ev::Event) -> bool {
    use wasm_bindgen::JsCast;
    ev.target()
        .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok())
        .is_some_and(|el| el.checked())
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
                                        .set(Some(Notification::Error(e)));
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
