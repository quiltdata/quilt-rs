use leptos::prelude::*;
use leptos_router::hooks::use_navigate;

use crate::commands::{self, InstalledPackageData};
use crate::components::buttons;
use crate::components::{Notification, ToolbarActions};
use crate::util;

// ── Toolbar actions (rendered into Layout's toolbar) ──

pub(super) fn build_toolbar_actions(
    data: &InstalledPackageData,
    notification: RwSignal<Option<Notification>>,
    ui_locked: RwSignal<bool>,
    show_set_remote_popup: RwSignal<bool>,
) -> ToolbarActions {
    let namespace = data.namespace.clone();
    let origin_url = data.uri.as_ref().and_then(util::catalog_url);
    let has_catalog = origin_url.is_some();
    let catalog_disabled = data.status == "local";

    let remote_configured = data
        .uri
        .as_ref()
        .is_some_and(|u| u.catalog.is_some() && !u.bucket.is_empty());
    let is_error = data.status == "error";
    let (remote_label, remote_warning) = if !remote_configured {
        ("Set remote", true)
    } else if data.remote_locked {
        ("Show remote", false)
    } else {
        ("Change remote", is_error)
    };

    ToolbarActions::new(move || {
        let navigate = use_navigate();

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

        let url_for_catalog = origin_url.clone();
        let on_open_catalog = move |_| {
            if let Some(url) = url_for_catalog.clone() {
                leptos::task::spawn_local(async move {
                    let _ = commands::open_in_web_browser(url).await;
                });
            }
        };

        let ns_for_uninstall = namespace.clone();
        let on_uninstall = move |_| {
            let ns = ns_for_uninstall.clone();
            let navigate = navigate.clone();
            ui_locked.set(true);
            leptos::task::spawn_local(async move {
                match commands::package_uninstall(ns).await {
                    Ok(msg) => {
                        notification.set(Some(Notification::Success(msg)));
                        navigate("/installed-packages-list", Default::default());
                    }
                    Err(e) => {
                        ui_locked.set(false);
                        notification.set(Some(Notification::Error(e)));
                    }
                }
            });
        };

        let on_set_remote = move |_| show_set_remote_popup.set(true);

        view! {
            <li>
                <buttons::OpenInFileBrowser on_click=on_open_file_browser />
            </li>
            {if has_catalog {
                view! {
                    <li>
                        <buttons::OpenInCatalog on_click=on_open_catalog disabled=catalog_disabled />
                    </li>
                }.into_any()
            } else {
                ().into_any()
            }}
            <li>
                <buttons::SetRemote
                    on_click=on_set_remote
                    warning=remote_warning
                    label=remote_label
                />
            </li>
            <li>
                <buttons::Remove on_click=on_uninstall />
            </li>
        }
        .into_any()
    })
}
