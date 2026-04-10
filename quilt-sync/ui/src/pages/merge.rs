use leptos::prelude::*;
use leptos_router::hooks::{use_navigate, use_query_map};

use crate::commands::{self, MergeData};
use crate::components::layout::{BreadcrumbItem, BreadcrumbLink};
use crate::components::{Layout, Spinner, ToolbarActions};

// ── Merge page ──

#[component]
pub fn Merge() -> impl IntoView {
    let notification = RwSignal::new(String::new());

    let query = use_query_map();
    let data = LocalResource::new(move || {
        let namespace = query.read().get("namespace").unwrap_or_default();
        async move {
            commands::get_merge_data(namespace).await
        }
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
                        let actions = build_toolbar_actions(&d, notification);
                        view! {
                            <Layout breadcrumbs=breadcrumbs notification=notification actions=actions>
                                <MergeContent data=d notification=notification />
                            </Layout>
                        }
                            .into_any()
                    }
                    Err(e) => {
                        let msg = format!("Failed to load merge data: {e}");
                        view! {
                            <Layout breadcrumbs=vec![] notification=notification>
                                <div class="qui-page-merge container">
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

// ── Main content ──

#[component]
fn MergeContent(data: MergeData, notification: RwSignal<String>) -> impl IntoView {
    let namespace = data.namespace.clone();
    let navigate = use_navigate();

    let ns_for_certify = namespace.clone();
    let navigate_for_certify = navigate.clone();
    let on_certify = move |_| {
        let ns = ns_for_certify.clone();
        let navigate = navigate_for_certify.clone();
        lock_ui();
        leptos::task::spawn_local(async move {
            match commands::certify_latest(ns.clone()).await {
                Ok(html) => {
                    notification.set(html);
                    navigate(
                        &format!("/installed-package?namespace={ns}&filter=unmodified"),
                        Default::default(),
                    );
                }
                Err(e) => {
                    unlock_ui();
                    notification.set(format!("<div class=\"error\">{e}</div>"));
                }
            }
        });
    };

    let ns_for_reset = namespace.clone();
    let navigate_for_reset = navigate.clone();
    let on_reset = move |_| {
        let ns = ns_for_reset.clone();
        let navigate = navigate_for_reset.clone();
        lock_ui();
        leptos::task::spawn_local(async move {
            match commands::reset_local(ns.clone()).await {
                Ok(html) => {
                    notification.set(html);
                    navigate(
                        &format!("/installed-package?namespace={ns}&filter=unmodified"),
                        Default::default(),
                    );
                }
                Err(e) => {
                    unlock_ui();
                    notification.set(format!("<div class=\"error\">{e}</div>"));
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

fn build_toolbar_actions(data: &MergeData, notification: RwSignal<String>) -> ToolbarActions {
    let namespace = data.namespace.clone();
    let origin_url = data.origin_url.clone();

    ToolbarActions::new(move || {
        let navigate = use_navigate();

        let ns_for_folder = namespace.clone();
        let on_open_folder = move |_| {
            let ns = ns_for_folder.clone();
            leptos::task::spawn_local(async move {
                match commands::open_in_file_browser(ns).await {
                    Ok(html) => notification.set(html),
                    Err(e) => notification.set(format!("<div class=\"error\">{e}</div>")),
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
            lock_ui();
            leptos::task::spawn_local(async move {
                match commands::package_uninstall(ns).await {
                    Ok(html) => {
                        notification.set(html);
                        navigate("/installed-packages-list", Default::default());
                    }
                    Err(e) => {
                        unlock_ui();
                        notification.set(format!("<div class=\"error\">{e}</div>"));
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

// ── Helpers ──

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
