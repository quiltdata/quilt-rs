use leptos::prelude::*;
use serde::{Deserialize, Serialize};
use wasm_bindgen::JsCast;

use crate::components::layout::BreadcrumbItem;
use crate::components::{Layout, Spinner, ToolbarActions};
use crate::tauri;

// ── Data types (mirror the Tauri command response) ──

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledPackagesListData {
    pub packages: Vec<PackageItemData>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageItemData {
    pub namespace: String,
    pub status: String,
    pub origin_url: Option<String>,
    pub origin_host: Option<String>,
    pub remote_display: Option<String>,
}

// ── Installed Packages List page ──

#[component]
pub fn InstalledPackagesList() -> impl IntoView {
    let notification = RwSignal::new(String::new());

    let data = LocalResource::new(move || async {
        tauri::invoke_unit::<InstalledPackagesListData>("get_installed_packages_list_data").await
    });

    view! {
        <Suspense fallback=move || {
            view! {
                <Layout breadcrumbs=vec![] notification=notification>
                    <Spinner />
                </Layout>
            }
        }>
            {move || Suspend::new(async move {
                match data.await {
                    Ok(d) => {
                        let breadcrumbs = vec![
                            BreadcrumbItem::Current("Packages".to_string()),
                        ];
                        let show_create_popup = RwSignal::new(false);
                        let show_create_popup_for_action = show_create_popup;
                        let actions = ToolbarActions::new(move || {
                            view! {
                                <li>
                                    <button
                                        class="qui-button small"
                                        type="button"
                                        on:click=move |_| show_create_popup_for_action.set(true)
                                    >
                                        <img class="qui-icon" src="/assets/img/icons/add.svg" />
                                        <span>"Create local package"</span>
                                    </button>
                                </li>
                            }.into_any()
                        });
                        view! {
                            <Layout breadcrumbs=breadcrumbs notification=notification actions=actions>
                                <PackagesListContent
                                    packages=d.packages
                                    notification=notification
                                    show_create_popup=show_create_popup
                                />
                            </Layout>
                        }
                            .into_any()
                    }
                    Err(e) => {
                        let msg = format!("Failed to load packages list: {e}");
                        view! {
                            <Layout breadcrumbs=vec![] notification=notification>
                                <div class="qui-page-installed-packages-list">
                                    <p>{msg}</p>
                                </div>
                            </Layout>
                        }
                            .into_any()
                    }
                }
            })}
        </Suspense>
    }
}

// ── List content ──

#[component]
fn PackagesListContent(
    packages: Vec<PackageItemData>,
    notification: RwSignal<String>,
    show_create_popup: RwSignal<bool>,
) -> impl IntoView {
    let show_set_remote_popup = RwSignal::new(None::<String>);
    let show_set_origin_popup = RwSignal::new(None::<SetOriginPopupData>);

    let is_empty = packages.is_empty();

    view! {
        <div class="qui-page-installed-packages-list">
            {if is_empty {
                view! {
                    <section class="empty">
                        <h1 class="empty-title">"You don't have any packages"</h1>
                        <p class="empty-title">"You can navigate to the file in Quilt Catalog and click on GET FILE and then OPEN IN QUILTSYNC buttons to install that package with file"</p>
                        <img class="empty-img" src="/assets/img/how-to-deep-link.png" />
                    </section>
                }.into_any()
            } else {
                view! {
                    <ul class="list">
                        {packages.into_iter().map(|pkg| {
                            view! {
                                <PackageItem
                                    data=pkg
                                    notification=notification
                                    show_set_remote_popup=show_set_remote_popup
                                    show_set_origin_popup=show_set_origin_popup
                                />
                            }
                        }).collect_view()}
                    </ul>
                }.into_any()
            }}
        </div>

        // ── Popups ──
        <Show when=move || show_create_popup.get()>
            <CreatePackagePopup
                notification=notification
                on_close=move || show_create_popup.set(false)
            />
        </Show>

        <Show when=move || show_set_remote_popup.get().is_some()>
            {move || show_set_remote_popup.get().map(|ns| {
                view! {
                    <SetRemotePopup
                        namespace=ns
                        notification=notification
                        on_close=move || show_set_remote_popup.set(None)
                    />
                }
            })}
        </Show>

        <Show when=move || show_set_origin_popup.get().is_some()>
            {move || show_set_origin_popup.get().map(|data| {
                view! {
                    <SetOriginPopup
                        namespace=data.namespace
                        current_origin=data.current_origin
                        notification=notification
                        on_close=move || show_set_origin_popup.set(None)
                    />
                }
            })}
        </Show>
    }
}

// ── Package item row ──

#[component]
fn PackageItem(
    data: PackageItemData,
    notification: RwSignal<String>,
    show_set_remote_popup: RwSignal<Option<String>>,
    show_set_origin_popup: RwSignal<Option<SetOriginPopupData>>,
) -> impl IntoView {
    let is_error = data.status == "error";
    let item_class = if is_error {
        "qui-installed-package-item error"
    } else {
        "qui-installed-package-item"
    };

    let pkg_href = format!(
        "/installed-package?namespace={}&filter=unmodified",
        data.namespace
    );

    let namespace_display = data.namespace.clone();
    let remote_display = data.remote_display.clone();

    // Build menu buttons
    let menu = build_package_menu(
        &data,
        notification,
        show_set_remote_popup,
        show_set_origin_popup,
    );

    view! {
        <li class=item_class>
            <a class="link" href=pkg_href>
                <span class="item-primary">{namespace_display}</span>
                {remote_display.map(|uri| view! {
                    <span class="item-secondary">
                        <strong>"URI: "</strong>
                        {uri}
                    </span>
                })}
            </a>
            <div class="menu">
                <ul class="menu-list">
                    {menu}
                </ul>
            </div>
        </li>
    }
}

fn build_package_menu(
    data: &PackageItemData,
    notification: RwSignal<String>,
    show_set_remote_popup: RwSignal<Option<String>>,
    show_set_origin_popup: RwSignal<Option<SetOriginPopupData>>,
) -> impl IntoView {
    let namespace = data.namespace.clone();
    let status = data.status.clone();
    let origin_url = data.origin_url.clone();
    let origin_host = data.origin_host.clone();
    let has_origin = origin_url.is_some();
    let is_error = status == "error";

    // ── Open local ──
    let ns_for_folder = namespace.clone();
    let on_open_folder = move |_| {
        let ns = ns_for_folder.clone();
        leptos::task::spawn_local(async move {
            #[derive(Serialize)]
            struct Args {
                namespace: String,
            }
            match tauri::invoke::<_, String>("open_in_file_browser", &Args { namespace: ns }).await
            {
                Ok(html) => notification.set(html),
                Err(e) => notification.set(format!("<div class=\"error\">{e}</div>")),
            }
        });
    };

    // ── Open remote (catalog) ──
    let origin_for_catalog = origin_url.clone();
    let catalog_disabled = status == "local";
    let on_open_catalog = move |_| {
        if let Some(url) = origin_for_catalog.clone() {
            leptos::task::spawn_local(async move {
                #[derive(Serialize)]
                struct Args {
                    url: String,
                }
                let _ = tauri::invoke::<_, String>("open_in_web_browser", &Args { url }).await;
            });
        }
    };

    // ── Commit ──
    let commit_href = format!("/commit?namespace={}", namespace);

    // ── Sync button (Push/Pull) ──
    let sync_button = match status.as_str() {
        "ahead" => Some(SyncAction::Push(namespace.clone())),
        "behind" => Some(SyncAction::Pull(namespace.clone())),
        "local" if has_origin => Some(SyncAction::Push(namespace.clone())),
        _ => None,
    };

    // ── Merge button ──
    let merge_href = if status == "diverged" {
        Some(format!("/merge?namespace={}", namespace))
    } else {
        None
    };

    // ── Error action button ──
    let error_action = build_error_action(
        &namespace,
        &status,
        origin_host.as_deref(),
        show_set_remote_popup,
        show_set_origin_popup,
    );

    // ── Uninstall ──
    let ns_for_uninstall = namespace.clone();
    let on_uninstall = move |_| {
        let ns = ns_for_uninstall.clone();
        lock_ui();
        leptos::task::spawn_local(async move {
            #[derive(Serialize)]
            struct Args {
                namespace: String,
            }
            match tauri::invoke::<_, String>("package_uninstall", &Args { namespace: ns }).await {
                Ok(html) => {
                    notification.set(html);
                    let _ = web_sys::window().and_then(|w| w.location().reload().ok());
                }
                Err(e) => {
                    unlock_ui();
                    notification.set(format!("<div class=\"error\">{e}</div>"));
                }
            }
        });
    };

    view! {
        // Open local
        <li class="menu-item">
            <button class="qui-button small" type="button" on:click=on_open_folder>
                <img class="qui-icon" src="/assets/img/icons/folder_open.svg" />
                <span>"Open"</span>
            </button>
        </li>
        // Open remote
        {has_origin.then(|| view! {
            <li class="menu-item">
                <button
                    class="qui-button small"
                    type="button"
                    prop:disabled=catalog_disabled
                    on:click=on_open_catalog
                >
                    <img class="qui-icon" src="/assets/img/icons/open_in_browser.svg" />
                    <span>"Open in Catalog"</span>
                </button>
            </li>
        })}

        <li class="menu-item menu-divider"></li>

        // Commit (unless error)
        {(!is_error).then(|| {
            let href = commit_href.clone();
            view! {
                <li class="menu-item">
                    <a href=href>
                        <button class="qui-button small" type="button">
                            <img class="qui-icon" src="/assets/img/icons/commit.svg" />
                            <span>"Commit"</span>
                        </button>
                    </a>
                </li>
            }
        })}

        // Sync (Push/Pull)
        {sync_button.map(|action| view! {
            <li class="menu-item menu-divider"></li>
            <li class="menu-item">
                <SyncButton action=action notification=notification />
            </li>
        })}

        // Merge
        {merge_href.map(|href| view! {
            <li class="menu-item menu-divider"></li>
            <li class="menu-item">
                <a href=href>
                    <button class="qui-button primary small" type="button">
                        <img class="qui-icon" src="/assets/img/icons/merge.svg" />
                        <span>"Merge"</span>
                    </button>
                </a>
            </li>
        })}

        // Error action
        {error_action.map(|action| view! {
            <li class="menu-item menu-divider"></li>
            <li class="menu-item">
                {action}
            </li>
        })}

        <li class="menu-item menu-divider"></li>

        // Uninstall
        <li class="menu-item">
            <button class="qui-button small" type="button" on:click=on_uninstall>
                <img class="qui-icon" src="/assets/img/icons/block.svg" />
                <span>"Remove"</span>
            </button>
        </li>
    }
}

// ── Sync button (Push or Pull) ──

#[derive(Clone)]
enum SyncAction {
    Push(String),
    Pull(String),
}

#[component]
fn SyncButton(action: SyncAction, notification: RwSignal<String>) -> impl IntoView {
    let busy = RwSignal::new(false);

    let (cmd, label, busy_label, icon) = match &action {
        SyncAction::Push(_) => (
            "package_push",
            "Push",
            "Pushing\u{2026}",
            "/assets/img/icons/cloud_upload.svg",
        ),
        SyncAction::Pull(_) => (
            "package_pull",
            "Pull",
            "Pulling\u{2026}",
            "/assets/img/icons/cloud_download.svg",
        ),
    };

    let ns = match action {
        SyncAction::Push(ns) | SyncAction::Pull(ns) => ns,
    };

    let on_click = move |_| {
        if busy.get_untracked() {
            return;
        }
        busy.set(true);
        lock_ui();
        let ns = ns.clone();
        let cmd = cmd;
        leptos::task::spawn_local(async move {
            #[derive(Serialize)]
            struct Args {
                namespace: String,
            }
            match tauri::invoke::<_, String>(cmd, &Args { namespace: ns }).await {
                Ok(html) => {
                    notification.set(html);
                    let _ = web_sys::window().and_then(|w| w.location().reload().ok());
                }
                Err(e) => {
                    unlock_ui();
                    notification.set(format!("<div class=\"error\">{e}</div>"));
                    busy.set(false);
                }
            }
        });
    };

    view! {
        <button
            class="qui-button primary small"
            type="button"
            prop:disabled=move || busy.get()
            on:click=on_click
        >
            <img class="qui-icon" src=icon />
            <span>{move || if busy.get() { busy_label } else { label }}</span>
        </button>
    }
}

// ── Error action button logic ──

fn build_error_action(
    namespace: &str,
    status: &str,
    origin_host: Option<&str>,
    show_set_remote_popup: RwSignal<Option<String>>,
    show_set_origin_popup: RwSignal<Option<SetOriginPopupData>>,
) -> Option<AnyView> {
    match status {
        // Local without origin — offer to set remote
        "local" if origin_host.is_none() => {
            let ns = namespace.to_string();
            Some(
                view! {
                    <button
                        class="qui-button small"
                        type="button"
                        on:click=move |_| show_set_remote_popup.set(Some(ns.clone()))
                    >
                        <img class="qui-icon" src="/assets/img/icons/cloud_upload.svg" />
                        <span>"Set remote"</span>
                    </button>
                }
                .into_any(),
            )
        }
        // Local with origin — no error action needed (Push is in sync)
        "local" => None,
        // Has origin_host but error — offer login
        _ if origin_host.is_some() && status == "error" => {
            let host = origin_host.unwrap().to_string();
            let back_encoded = urlencoding("/installed-packages-list");
            let login_href = format!(
                "/login?host={}&back={back_encoded}",
                host
            );
            Some(
                view! {
                    <a href=login_href>
                        <button class="qui-button warning small" type="button">
                            <img class="qui-icon" src="/assets/img/icons/warning.svg" />
                            <span>"Login"</span>
                        </button>
                    </a>
                }
                .into_any(),
            )
        }
        // No origin_host — offer to set origin
        _ if origin_host.is_none() => {
            let ns = namespace.to_string();
            Some(
                view! {
                    <button
                        class="qui-button warning small"
                        type="button"
                        on:click=move |_| {
                            show_set_origin_popup.set(Some(SetOriginPopupData {
                                namespace: ns.clone(),
                                current_origin: String::new(),
                            }))
                        }
                    >
                        <img class="qui-icon" src="/assets/img/icons/warning.svg" />
                        <span>"Set origin"</span>
                    </button>
                }
                .into_any(),
            )
        }
        _ => None,
    }
}

// ── Create Package popup ──

#[component]
fn CreatePackagePopup(
    notification: RwSignal<String>,
    on_close: impl Fn() + Clone + 'static,
) -> impl IntoView {
    let namespace = RwSignal::new(String::new());
    let source = RwSignal::new(None::<String>);
    let submitting = RwSignal::new(false);

    let on_close_submit = on_close.clone();
    let on_submit = move || {
        let ns = namespace.get_untracked().trim().to_string();
        if ns.is_empty() || submitting.get_untracked() {
            return;
        }
        submitting.set(true);
        let src = source.get_untracked();
        let on_close = on_close_submit.clone();
        leptos::task::spawn_local(async move {
            #[derive(Serialize)]
            struct Args {
                namespace: String,
                source: Option<String>,
                message: Option<String>,
            }
            match tauri::invoke::<_, String>(
                "package_create",
                &Args {
                    namespace: ns,
                    source: src,
                    message: None,
                },
            )
            .await
            {
                Ok(html) => {
                    notification.set(html);
                    on_close();
                    let _ = web_sys::window().and_then(|w| w.location().reload().ok());
                }
                Err(e) => {
                    notification.set(format!("<div class=\"error\">{e}</div>"));
                    submitting.set(false);
                }
            }
        });
    };

    let on_browse = move |_| {
        leptos::task::spawn_local(async move {
            match tauri::invoke_unit::<String>("open_directory_picker").await {
                Ok(path) => source.set(Some(path)),
                Err(_) => {} // user cancelled
            }
        });
    };

    let on_submit_click = {
        let on_submit = on_submit.clone();
        move |_| on_submit()
    };

    let on_close_cancel = on_close.clone();
    let on_cancel = move |_: leptos::ev::MouseEvent| on_close_cancel();

    let on_submit_key = on_submit.clone();
    let on_close_key = on_close.clone();
    let on_keydown = move |ev: leptos::ev::KeyboardEvent| {
        if ev.key() == "Enter" {
            on_submit_key();
        } else if ev.key() == "Escape" {
            on_close_key();
        }
    };

    view! {
        <div class="popup-overlay" on:click={
            let on_close = on_close.clone();
            move |_| on_close()
        }>
            <div class="popup-content" on:click=|ev| ev.stop_propagation()>
                <div class="create-package-form">
                    <label>"Namespace"</label>
                    <input
                        class="create-package-input"
                        type="text"
                        placeholder="owner/package-name"
                        prop:value=move || namespace.get()
                        on:input=move |ev| namespace.set(event_target_value(&ev))
                        on:keydown=on_keydown
                    />

                    <label>"Source directory (optional)"</label>
                    <div class="create-package-source">
                        <span class="source-path">{move || {
                            source.get().unwrap_or_else(|| "No directory selected".to_string())
                        }}</span>
                        <button class="qui-button small" type="button" on:click=on_browse>
                            <span>"Browse..."</span>
                        </button>
                    </div>

                    <div class="create-package-actions">
                        <button
                            class="qui-button primary"
                            type="button"
                            prop:disabled=move || submitting.get()
                            on:click=on_submit_click
                        >
                            <span>"Create"</span>
                        </button>
                        <button class="qui-button" type="button" on:click=on_cancel>
                            <span>"Cancel"</span>
                        </button>
                    </div>
                </div>
            </div>
        </div>
    }
}

// ── Set Remote popup ──

#[component]
fn SetRemotePopup(
    namespace: String,
    notification: RwSignal<String>,
    on_close: impl Fn() + Clone + 'static,
) -> impl IntoView {
    let origin = RwSignal::new(String::new());
    let bucket = RwSignal::new(String::new());
    let host_error = RwSignal::new(false);
    let bucket_error = RwSignal::new(false);
    let submitting = RwSignal::new(false);

    let ns = namespace.clone();
    let on_close_submit = on_close.clone();
    let on_submit = move || {
        if submitting.get_untracked() {
            return;
        }
        let origin_val = origin.get_untracked().trim().to_string();
        let bucket_val = bucket.get_untracked().trim().to_string();

        let mut valid = true;
        if origin_val.is_empty() || !is_valid_hostname(&origin_val) {
            host_error.set(true);
            valid = false;
        }
        if bucket_val.is_empty() {
            bucket_error.set(true);
            valid = false;
        }
        if !valid {
            return;
        }

        submitting.set(true);
        let ns = ns.clone();
        let on_close = on_close_submit.clone();
        leptos::task::spawn_local(async move {
            #[derive(Serialize)]
            struct Args {
                namespace: String,
                origin: String,
                bucket: String,
            }
            match tauri::invoke::<_, String>(
                "set_remote",
                &Args {
                    namespace: ns,
                    origin: origin_val,
                    bucket: bucket_val,
                },
            )
            .await
            {
                Ok(html) => {
                    notification.set(html);
                    on_close();
                    let _ = web_sys::window().and_then(|w| w.location().reload().ok());
                }
                Err(e) => {
                    notification.set(format!("<div class=\"error\">{e}</div>"));
                    submitting.set(false);
                }
            }
        });
    };

    let on_submit_click = {
        let on_submit = on_submit.clone();
        move |_| on_submit()
    };

    let on_close_cancel = on_close.clone();
    let on_cancel = move |_: leptos::ev::MouseEvent| on_close_cancel();

    // Enter on host → focus bucket; Enter on bucket → submit
    let on_submit_bucket = on_submit.clone();
    let on_close_key_host = on_close.clone();
    let on_host_keydown = move |ev: leptos::ev::KeyboardEvent| {
        if ev.key() == "Enter" {
            // Focus the bucket input
            if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
                if let Some(el) = doc.get_element_by_id("set-remote-bucket") {
                    if let Ok(input) = el.dyn_into::<web_sys::HtmlElement>() {
                        let _ = input.focus();
                    }
                }
            }
        } else if ev.key() == "Escape" {
            on_close_key_host();
        }
    };

    let on_close_key_bucket = on_close.clone();
    let on_bucket_keydown = move |ev: leptos::ev::KeyboardEvent| {
        if ev.key() == "Enter" {
            on_submit_bucket();
        } else if ev.key() == "Escape" {
            on_close_key_bucket();
        }
    };

    view! {
        <div class="popup-overlay" on:click={
            let on_close = on_close.clone();
            move |_| on_close()
        }>
            <div class="popup-content" on:click=|ev| ev.stop_propagation()>
                <div class="set-remote-form">
                    <label>"Host"</label>
                    <div class="set-remote-input-group">
                        <input
                            class="set-remote-input"
                            class:error=move || host_error.get()
                            type="text"
                            placeholder="open.quiltdata.com"
                            prop:value=move || origin.get()
                            on:input=move |ev| {
                                origin.set(event_target_value(&ev));
                                host_error.set(false);
                            }
                            on:keydown=on_host_keydown
                        />
                        <span
                            class="set-remote-hint"
                            class:visible=move || host_error.get()
                        >
                            "Enter a valid hostname"
                        </span>
                    </div>

                    <label>"Bucket"</label>
                    <div class="set-remote-input-group">
                        <input
                            id="set-remote-bucket"
                            class="set-remote-input"
                            class:error=move || bucket_error.get()
                            type="text"
                            placeholder="my-s3-bucket"
                            prop:value=move || bucket.get()
                            on:input=move |ev| {
                                bucket.set(event_target_value(&ev));
                                bucket_error.set(false);
                            }
                            on:keydown=on_bucket_keydown
                        />
                        <span
                            class="set-remote-hint"
                            class:visible=move || bucket_error.get()
                        >
                            "Enter an S3 bucket name"
                        </span>
                    </div>

                    <div class="set-remote-actions">
                        <button
                            class="qui-button primary"
                            type="button"
                            prop:disabled=move || submitting.get()
                            on:click=on_submit_click
                        >
                            <span>"Save"</span>
                        </button>
                        <button class="qui-button" type="button" on:click=on_cancel>
                            <span>"Cancel"</span>
                        </button>
                    </div>
                </div>
            </div>
        </div>
    }
}

// ── Set Origin popup (reused from installed_package.rs pattern) ──

#[derive(Clone, Debug)]
struct SetOriginPopupData {
    namespace: String,
    current_origin: String,
}

#[component]
fn SetOriginPopup(
    namespace: String,
    current_origin: String,
    notification: RwSignal<String>,
    on_close: impl Fn() + Clone + 'static,
) -> impl IntoView {
    let origin = RwSignal::new(current_origin);
    let show_error = RwSignal::new(false);
    let submitting = RwSignal::new(false);

    let ns = namespace.clone();
    let on_close_submit = on_close.clone();
    let on_submit = move || {
        let value = origin.get_untracked().trim().to_string();
        if value.is_empty() || submitting.get_untracked() {
            return;
        }
        if !is_valid_hostname(&value) {
            show_error.set(true);
            return;
        }
        submitting.set(true);
        let ns = ns.clone();
        let on_close = on_close_submit.clone();
        leptos::task::spawn_local(async move {
            #[derive(Serialize)]
            struct Args {
                namespace: String,
                origin: String,
            }
            match tauri::invoke::<_, String>(
                "set_origin",
                &Args {
                    namespace: ns,
                    origin: value,
                },
            )
            .await
            {
                Ok(html) => {
                    notification.set(html);
                    on_close();
                    let _ = web_sys::window().and_then(|w| w.location().reload().ok());
                }
                Err(e) => {
                    notification.set(format!("<div class=\"error\">{e}</div>"));
                    submitting.set(false);
                }
            }
        });
    };

    let on_submit_click = {
        let on_submit = on_submit.clone();
        move |_| on_submit()
    };

    let on_close_cancel = on_close.clone();
    let on_cancel = move |_: leptos::ev::MouseEvent| on_close_cancel();

    let on_submit_key = on_submit.clone();
    let on_close_key = on_close.clone();
    let on_keydown = move |ev: leptos::ev::KeyboardEvent| {
        if ev.key() == "Enter" {
            on_submit_key();
        } else if ev.key() == "Escape" {
            on_close_key();
        }
    };

    view! {
        <div class="popup-overlay" on:click={
            let on_close = on_close.clone();
            move |_| on_close()
        }>
            <div class="popup-content" on:click=|ev| ev.stop_propagation()>
                <div class="origin-form">
                    <label>"Catalog origin"</label>
                    <div class="origin-input-group">
                        <input
                            class="origin-input"
                            class:error=move || show_error.get()
                            type="text"
                            placeholder="open.quilt.bio"
                            prop:value=move || origin.get()
                            on:input=move |ev| {
                                origin.set(event_target_value(&ev));
                                show_error.set(false);
                            }
                            on:keydown=on_keydown
                        />
                        <span
                            class="origin-hint"
                            class:visible=move || show_error.get()
                        >
                            "Enter a valid hostname, e.g. open.quilt.bio"
                        </span>
                    </div>
                    <div class="origin-form-actions">
                        <button
                            class="qui-button primary"
                            type="button"
                            prop:disabled=move || submitting.get()
                            on:click=on_submit_click
                        >
                            <span>"Submit"</span>
                        </button>
                        <button class="qui-button" type="button" on:click=on_cancel>
                            <span>"Cancel"</span>
                        </button>
                    </div>
                </div>
            </div>
        </div>
    }
}

// ── Helpers ──

fn is_valid_hostname(value: &str) -> bool {
    let re = js_sys::RegExp::new(
        r"^[a-zA-Z0-9]([a-zA-Z0-9-]*[a-zA-Z0-9])?(\.[a-zA-Z0-9]([a-zA-Z0-9-]*[a-zA-Z0-9])?)+$",
        "",
    );
    re.test(value)
}

fn lock_ui() {
    if let Some(el) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.get_element_by_id("layout"))
    {
        let _ = el.set_attribute("disabled", "disabled");
    }
}

fn unlock_ui() {
    if let Some(el) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.get_element_by_id("layout"))
    {
        let _ = el.remove_attribute("disabled");
    }
}

fn urlencoding(s: &str) -> String {
    js_sys::encode_uri_component(s)
        .as_string()
        .unwrap_or_default()
}
