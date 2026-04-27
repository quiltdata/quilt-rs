use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use leptos::prelude::*;

use crate::commands::{self, PackageItemData};
use crate::components::buttons;
use crate::components::layout::BreadcrumbItem;
use crate::components::{
    Layout, Notification, SetRemotePopup, SetRemotePopupData, Spinner, ToolbarActions,
};
use crate::util;
use crate::util::make_action;

// ── Installed Packages List page ──

#[component]
pub fn InstalledPackagesList() -> impl IntoView {
    let notification = RwSignal::new(None);
    let ui_locked = RwSignal::new(false);
    let refetch = Trigger::new();

    let data = LocalResource::new(move || {
        refetch.track();
        async { commands::get_installed_packages_list_data().await }
    });

    view! {
        <Suspense fallback=move || {
            view! {
                <Layout breadcrumbs=vec![] notification=notification ui_locked=ui_locked>
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
                                    <buttons::CreateLocalPackage
                                        on_click=move |_| show_create_popup_for_action.set(true)
                                        small=true
                                    />
                                </li>
                            }.into_any()
                        });
                        view! {
                            <Layout breadcrumbs=breadcrumbs notification=notification actions=actions ui_locked=ui_locked>
                                <PackagesListContent
                                    packages=d.packages
                                    notification=notification
                                    ui_locked=ui_locked
                                    refetch=refetch
                                    show_create_popup=show_create_popup
                                />
                            </Layout>
                        }
                            .into_any()
                    }
                    Err(e) => {
                        crate::error_handler::handle_or_display(&e, notification)
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
    notification: RwSignal<Option<Notification>>,
    ui_locked: RwSignal<bool>,
    refetch: Trigger,
    show_create_popup: RwSignal<bool>,
) -> impl IntoView {
    let show_set_remote_popup = RwSignal::new(None::<SetRemotePopupData>);

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
                                    ui_locked=ui_locked
                                    refetch=refetch
                                    show_set_remote_popup=show_set_remote_popup
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
                refetch=refetch
                on_close=move || show_create_popup.set(false)
            />
        </Show>

        <Show when=move || show_set_remote_popup.get().is_some()>
            {move || show_set_remote_popup.get().map(|data| {
                view! {
                    <SetRemotePopup
                        namespace=data.namespace
                        current_host=data.current_host
                        current_bucket=data.current_bucket
                        notification=notification
                        refetch=refetch
                        on_close=move || show_set_remote_popup.set(None)
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
    notification: RwSignal<Option<Notification>>,
    ui_locked: RwSignal<bool>,
    refetch: Trigger,
    show_set_remote_popup: RwSignal<Option<SetRemotePopupData>>,
) -> impl IntoView {
    let status = RwSignal::new(data.status.clone());
    let has_changes = RwSignal::new(data.has_changes);
    let refreshing = RwSignal::new(true);
    let refresh_error = RwSignal::new(None::<String>);

    let cancelled = Arc::new(AtomicBool::new(false));
    let cancelled_flag = cancelled.clone();
    on_cleanup(move || cancelled.store(true, Ordering::Relaxed));

    let ns = data.namespace.clone();
    leptos::task::spawn_local(async move {
        let result = commands::refresh_package_status(ns).await;
        if cancelled_flag.load(Ordering::Relaxed) {
            return;
        }
        match result {
            Ok(fresh) => {
                status.set(fresh.status);
                has_changes.set(fresh.has_changes);
            }
            Err(err) => refresh_error.set(Some(err)),
        }
        refreshing.set(false);
    });

    let pkg_href = format!(
        "/installed-package?namespace={}&filter=unmodified",
        data.namespace
    );

    let namespace_display = data.namespace.clone();
    let remote_display = data.remote_display.clone();

    // Build menu buttons
    let menu = build_package_menu(
        &data,
        status,
        has_changes,
        refreshing,
        notification,
        ui_locked,
        refetch,
        show_set_remote_popup,
    );

    view! {
        <li class=move || if status.get() == "error" {
            "qui-installed-package-item error"
        } else {
            "qui-installed-package-item"
        }>
            <a class="link" href=pkg_href>
                <span class="item-primary">{namespace_display}</span>
                {remote_display.map(|uri| view! {
                    <span class="item-secondary">
                        <strong>"URI: "</strong>
                        {uri}
                    </span>
                })}
            </a>
            <Show when=move || refreshing.get()>
                <div class="q-spinner-inline" />
            </Show>
            <Show when=move || refresh_error.get().is_some()>
                <img class="refresh-warning-icon" src="/assets/img/icons/warning.svg" />
            </Show>
            <div class=move || if refreshing.get() { "menu refreshing" } else { "menu" }>
                <ul class="menu-list">
                    {menu}
                </ul>
            </div>
            <Show when=move || refreshing.get()>
                <div class="status-tooltip-wrapper">
                    <div class="status-tooltip">
                        "Syncing with remote and scanning local files for changes\u{2026}"
                    </div>
                </div>
            </Show>
            <Show when=move || refresh_error.get().is_some()>
                <div class="status-tooltip-wrapper">
                    <div class="status-tooltip error">
                        {move || refresh_error.get().unwrap_or_default()}
                    </div>
                </div>
            </Show>
        </li>
    }
}

#[allow(clippy::too_many_arguments)]
fn build_package_menu(
    data: &PackageItemData,
    status: RwSignal<String>,
    has_changes: RwSignal<bool>,
    refreshing: RwSignal<bool>,
    notification: RwSignal<Option<Notification>>,
    ui_locked: RwSignal<bool>,
    refetch: Trigger,
    show_set_remote_popup: RwSignal<Option<SetRemotePopupData>>,
) -> impl IntoView {
    let namespace = data.namespace.clone();
    let origin_url = data.uri.as_ref().and_then(util::catalog_url);
    let origin_host = data.uri.as_ref().and_then(util::host_str);
    let current_host = origin_host.clone();
    let current_bucket = data.uri.as_ref().and_then(util::bucket_str);
    let has_origin = origin_url.is_some();
    let remote_configured = current_host.is_some() && current_bucket.is_some();

    // ── Open in file browser ──
    let ns_for_open = namespace.clone();
    let on_open_file_browser = move |_| {
        let ns = ns_for_open.clone();
        leptos::task::spawn_local(async move {
            match commands::open_in_file_browser(ns).await {
                Ok(msg) => notification.set(Some(Notification::Success(msg))),
                Err(e) => notification.set(Some(Notification::Error(e))),
            }
        });
    };

    // ── Open in catalog ──
    let url_for_catalog = origin_url.clone();
    let on_open_catalog = move |_| {
        if let Some(url) = url_for_catalog.clone() {
            leptos::task::spawn_local(async move {
                let _ = commands::open_in_web_browser(url).await;
            });
        }
    };

    // ── Uninstall ──
    let ns_for_uninstall = namespace.clone();
    let on_uninstall = move |_| {
        let ns = ns_for_uninstall.clone();
        ui_locked.set(true);
        leptos::task::spawn_local(async move {
            match commands::package_uninstall(ns).await {
                Ok(msg) => {
                    ui_locked.set(false);
                    notification.set(Some(Notification::Success(msg)));
                    refetch.notify();
                }
                Err(e) => {
                    ui_locked.set(false);
                    notification.set(Some(Notification::Error(e)));
                }
            }
        });
    };

    // ── Sync actions (Publish/Pull) ──
    // Stored in StoredValue so they can be used inside Show children (which are Fn).
    let ns_for_publish = namespace.clone();
    let (publish_busy, on_publish) = make_action(
        move || {
            let ns = ns_for_publish.clone();
            async move { commands::package_publish(ns).await }
        },
        notification,
        Some(ui_locked),
        move || refetch.notify(),
    );
    let on_publish = StoredValue::new(on_publish);

    let ns_for_pull = namespace.clone();
    let (pull_busy, on_pull) = make_action(
        move || {
            let ns = ns_for_pull.clone();
            async move { commands::package_pull(ns).await }
        },
        notification,
        Some(ui_locked),
        move || refetch.notify(),
    );
    let on_pull = StoredValue::new(on_pull);

    let ns_for_merge = namespace.clone();

    // ── Error action (static views, shown/hidden reactively) ──
    let ns_for_set_remote = namespace.clone();
    let current_host_for_popup = current_host.clone();
    let current_bucket_for_popup = current_bucket.clone();
    let login_href = origin_host.as_ref().map(|host| {
        let back_encoded = urlencoding::encode("/installed-packages-list");
        format!("/login?host={}&back={back_encoded}", host)
    });

    view! {
        // Open local
        <li class="menu-item">
            <buttons::OpenInFileBrowser on_click=on_open_file_browser small=true />
        </li>
        // Open remote
        {has_origin.then(|| view! {
            <li class="menu-item">
                <buttons::OpenInCatalog
                    on_click=on_open_catalog
                    small=true
                    disabled=Signal::derive(move || status.get() == "local")
                />
            </li>
        })}

        <li class="menu-item menu-divider"></li>

        // Publish: commit (if needed) + push in one click.
        // Gated on having a remote origin, and on there being something to ship
        // (either uncommitted changes or a pending commit).
        <Show when=move || {
            let s = status.get();
            let publishable_status = s == "ahead" || (s == "local" && has_origin);
            let up_to_date_with_changes = s == "up_to_date" && has_changes.get() && has_origin;
            publishable_status || up_to_date_with_changes
        }>
            <li class="menu-item">
                <buttons::Publish
                    on_click=move |ev| on_publish.with_value(|f| f(ev))
                    small=true
                    busy=publish_busy
                    disabled=refreshing
                />
            </li>
        </Show>

        // Pull (behind)
        <Show when=move || status.get() == "behind">
            <li class="menu-item menu-divider"></li>
            <li class="menu-item">
                <div class="qui-popover">
                    <buttons::Pull on_click=move |ev| on_pull.with_value(|f| f(ev)) small=true busy=pull_busy disabled=has_changes />
                    <Show when=move || has_changes.get()>
                        <div class="popover-wrapper">
                            <div class="popover">
                                "Commit or discard local changes before pulling"
                            </div>
                        </div>
                    </Show>
                </div>
            </li>
        </Show>

        // Merge (diverged)
        <Show when=move || status.get() == "diverged">
            <li class="menu-item menu-divider"></li>
            <li class="menu-item">
                <buttons::Merge namespace=ns_for_merge.clone() small=true />
            </li>
        </Show>

        // Error actions
        // Host or bucket missing → Set Remote (warning palette, always visible)
        {(!remote_configured).then(|| view! {
            <li class="menu-item menu-divider"></li>
            <li class="menu-item">
                <buttons::SetRemote
                    on_click=move |_| show_set_remote_popup.set(Some(SetRemotePopupData {
                        namespace: ns_for_set_remote.clone(),
                        current_host: current_host_for_popup.clone(),
                        current_bucket: current_bucket_for_popup.clone(),
                    }))
                    small=true
                    warning=true
                />
            </li>
        })}
        // Has origin but error → Login (reactive on status)
        {login_href.map(|href| view! {
            <Show when=move || status.get() == "error">
                <li class="menu-item menu-divider"></li>
                <li class="menu-item">
                    <buttons::Login href=href.clone() small=true />
                </li>
            </Show>
        })}

        <li class="menu-item menu-divider"></li>

        // Uninstall
        <li class="menu-item">
            <buttons::Remove on_click=on_uninstall small=true />
        </li>
    }
}

// ── Create Package popup ──

#[component]
fn CreatePackagePopup(
    notification: RwSignal<Option<Notification>>,
    refetch: Trigger,
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
            match commands::package_create(ns, src, None).await {
                Ok(msg) => {
                    notification.set(Some(Notification::Success(msg)));
                    on_close();
                    refetch.notify();
                }
                Err(e) => {
                    notification.set(Some(Notification::Error(e)));
                    submitting.set(false);
                }
            }
        });
    };

    let on_browse = move |_| {
        leptos::task::spawn_local(async move {
            if let Ok(path) = commands::open_directory_picker().await {
                source.set(Some(path));
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
                        <buttons::Browse on_click=on_browse small=true />
                    </div>

                    <div class="create-package-actions">
                        <buttons::FormPrimary on_click=on_submit_click disabled=submitting>
                            "Create"
                        </buttons::FormPrimary>
                        <buttons::FormSecondary on_click=on_cancel />
                    </div>
                </div>
            </div>
        </div>
    }
}
