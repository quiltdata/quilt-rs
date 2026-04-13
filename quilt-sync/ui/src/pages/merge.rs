use leptos::prelude::*;
use leptos_router::hooks::{use_navigate, use_query_map};

use crate::commands::{self, MergeData};
use crate::components::layout::{BreadcrumbItem, BreadcrumbLink};
use crate::components::{Layout, Notification, Spinner, ToolbarActions};

// ── Merge page ──

#[component]
pub fn Merge() -> impl IntoView {
    let notification = RwSignal::new(None);
    let ui_locked = RwSignal::new(false);

    let query = use_query_map();
    let data = LocalResource::new(move || {
        let namespace = query.read().get("namespace").unwrap_or_default();
        async move { commands::get_merge_data(namespace).await }
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
                        let ns = d.namespace.clone();
                        let pkg_href = format!("/installed-package?namespace={ns}&filter=unmodified");
                        let breadcrumbs = vec![
                            BreadcrumbItem::Link(BreadcrumbLink {
                                href: "/installed-packages-list".to_string(),
                                title: String::new(),
                            }),
                            BreadcrumbItem::Link(BreadcrumbLink {
                                href: pkg_href,
                                title: ns.clone(),
                            }),
                            BreadcrumbItem::Current("Merge".to_string()),
                        ];
                        let actions = build_toolbar_actions(&d, notification, ui_locked);
                        view! {
                            <Layout breadcrumbs=breadcrumbs notification=notification actions=actions ui_locked=ui_locked>
                                <MergeContent data=d notification=notification ui_locked=ui_locked />
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

// ── Main content ──

#[component]
fn MergeContent(
    data: MergeData,
    notification: RwSignal<Option<Notification>>,
    ui_locked: RwSignal<bool>,
) -> impl IntoView {
    let namespace = data.namespace.clone();
    let navigate = use_navigate();

    let ns_for_certify = namespace.clone();
    let navigate_for_certify = navigate.clone();
    let on_certify = move |_| {
        let ns = ns_for_certify.clone();
        let navigate = navigate_for_certify.clone();
        ui_locked.set(true);
        leptos::task::spawn_local(async move {
            match commands::certify_latest(ns.clone()).await {
                Ok(msg) => {
                    notification.set(Some(Notification::Success(msg)));
                    navigate(
                        &format!("/installed-package?namespace={ns}&filter=unmodified"),
                        Default::default(),
                    );
                }
                Err(e) => {
                    ui_locked.set(false);
                    notification.set(Some(Notification::Error(e)));
                }
            }
        });
    };

    let ns_for_reset = namespace.clone();
    let navigate_for_reset = navigate.clone();
    let on_reset = move |_| {
        let ns = ns_for_reset.clone();
        let navigate = navigate_for_reset.clone();
        ui_locked.set(true);
        leptos::task::spawn_local(async move {
            match commands::reset_local(ns.clone()).await {
                Ok(msg) => {
                    notification.set(Some(Notification::Success(msg)));
                    navigate(
                        &format!("/installed-package?namespace={ns}&filter=unmodified"),
                        Default::default(),
                    );
                }
                Err(e) => {
                    ui_locked.set(false);
                    notification.set(Some(Notification::Error(e)));
                }
            }
        });
    };

    view! {
        <div class="qui-page-merge container">
            <div class="root">
                <div class="field">
                    <p class="description">
                        "Certify your latest commit as Quilt "
                        <code>"latest"</code>
                        ". This will update local and remote "
                        <code>"latest"</code>
                        " with your latest commit."
                    </p>
                    <button class="qui-button" type="button" on:click=on_certify>
                        <span>"Certify latest"</span>
                    </button>
                </div>

                <div class="field">
                    <p class="description">
                        "Erase local commits and make local "
                        <code>"latest"</code>
                        " the same as remote."
                    </p>
                    <button class="qui-button" type="button" on:click=on_reset>
                        <span>"Reset local"</span>
                    </button>
                </div>
            </div>
        </div>
    }
}

// ── Toolbar actions ──

fn build_toolbar_actions(
    data: &MergeData,
    notification: RwSignal<Option<Notification>>,
    ui_locked: RwSignal<bool>,
) -> ToolbarActions {
    let namespace = data.namespace.clone();
    let origin_url = data.origin_url.clone();

    ToolbarActions::new(move || {
        let navigate = use_navigate();

        let ns_for_folder = namespace.clone();
        let on_open_folder = move |_| {
            let ns = ns_for_folder.clone();
            leptos::task::spawn_local(async move {
                match commands::open_in_file_browser(ns).await {
                    Ok(msg) => notification.set(Some(Notification::Success(msg))),
                    Err(e) => notification.set(Some(Notification::Error(e))),
                }
            });
        };

        let origin_for_catalog = origin_url.clone();
        let on_open_catalog = move |_| {
            if let Some(url) = origin_for_catalog.clone() {
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

        let has_catalog = origin_url.is_some();

        view! {
            <li>
                <button class="qui-button small" type="button" on:click=on_open_folder>
                    <img class="qui-icon" src="/assets/img/icons/folder_open.svg" />
                    <span>"Open"</span>
                </button>
            </li>
            {has_catalog.then(|| view! {
                <li>
                    <button class="qui-button small" type="button" on:click=on_open_catalog>
                        <img class="qui-icon" src="/assets/img/icons/open_in_browser.svg" />
                        <span>"Open in Catalog"</span>
                    </button>
                </li>
            })}
            <li>
                <button class="qui-button small" type="button" on:click=on_uninstall>
                    <img class="qui-icon" src="/assets/img/icons/block.svg" />
                    <span>"Remove"</span>
                </button>
            </li>
        }
        .into_any()
    })
}
