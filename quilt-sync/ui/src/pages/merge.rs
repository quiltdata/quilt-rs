use leptos::prelude::*;
use leptos_router::NavigateOptions;
use leptos_router::hooks::use_navigate;

use quilt_ui_kit::{ConfirmDialog, Submit};

use super::installed_package_v2::resolve::share_consequence;
use crate::commands::{self, MergeData};
use crate::components::buttons;
use crate::components::layout::{BreadcrumbItem, BreadcrumbLink};
use crate::components::{Layout, Notification, Spinner, ToolbarActions};
use crate::util;

// ── Merge page ──

#[component]
pub fn Merge() -> impl IntoView {
    let notification = RwSignal::new(None);
    let ui_locked = RwSignal::new(false);

    let query = crate::routes::use_query();
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
                        let pkg_href = crate::routes::package_page_href(&ns);
                        let breadcrumbs = vec![
                            BreadcrumbItem::Link(BreadcrumbLink {
                                href: "/".to_string(),
                                title: String::new(),
                            }),
                            BreadcrumbItem::Link(BreadcrumbLink {
                                href: pkg_href,
                                title: ns.to_string(),
                            }),
                            BreadcrumbItem::Current("Resolve conflict".to_string()),
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
    let uri = data.uri.clone();
    let navigate = use_navigate();

    let ns_for_certify = namespace.clone();
    let uri_for_certify = uri.clone();
    let navigate_for_certify = navigate.clone();
    let certify = move || {
        let ns = ns_for_certify.clone();
        let uri = uri_for_certify.clone();
        let navigate = navigate_for_certify.clone();
        async move {
            let msg = commands::certify_latest(ns.to_string(), uri).await?;
            notification.set(Some(Notification::Success(msg)));
            navigate(
                &crate::routes::package_page_href(&ns),
                NavigateOptions::default(),
            );
            Ok::<(), String>(())
        }
    };

    // The confirmation's sentence, read at the press: `latest` may have moved
    // since the page loaded.
    let promote_open = RwSignal::new(false);
    let promote_says = RwSignal::new(None::<String>);
    let ns_for_loss = namespace.clone();
    let certify_at_once = certify.clone();
    let on_certify = move |_| {
        let ns = ns_for_loss.clone();
        let certify = certify_at_once.clone();
        ui_locked.set(true);
        leptos::task::spawn_local(async move {
            let answer = async {
                let loss = commands::get_latest_loss(ns.to_string()).await?;
                match share_consequence(loss) {
                    None => certify().await,
                    Some(sentence) => {
                        ui_locked.set(false);
                        promote_says.set(Some(sentence));
                        promote_open.set(true);
                        Ok(())
                    }
                }
            }
            .await;
            if let Err(e) = answer {
                ui_locked.set(false);
                notification.set(Some(Notification::Error(e)));
            }
        });
    };
    let promote_confirmation = move || {
        promote_says.get().map(|consequence| {
            view! {
                <ConfirmDialog
                    open=promote_open
                    title="Promote your revision to latest"
                    consequence=consequence
                    confirm=Submit::new("Promote my revision", certify.clone())
                />
            }
        })
    };

    let ns_for_reset = namespace.clone();
    let uri_for_reset = uri.clone();
    let navigate_for_reset = navigate.clone();
    let on_reset = move |_| {
        let ns = ns_for_reset.clone();
        let uri = uri_for_reset.clone();
        let navigate = navigate_for_reset.clone();
        ui_locked.set(true);
        leptos::task::spawn_local(async move {
            match commands::reset_local(ns.to_string(), uri).await {
                Ok(msg) => {
                    notification.set(Some(Notification::Success(msg)));
                    navigate(
                        &crate::routes::package_page_href(&ns),
                        NavigateOptions::default(),
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
                <p class="intro">
                    "Quilt doesn't merge file contents — there is no per-file three-way merge. Pick which side's revision becomes the new shared "
                    <code>"latest"</code>
                    "; the other side is discarded."
                </p>

                <div class="field">
                    <p class="description">
                        "Make your "
                        <abbr title="the hash of your most recent local commit, or the last commit you pushed">
                            "latest revision"
                        </abbr>
                        " the shared "
                        <code>"latest"</code>
                        ". If it hasn't been pushed yet, it is uploaded first. The remote pointer then moves to your revision; other users see it on their next pull. Local files are unchanged."
                    </p>
                    <buttons::CertifyLatest on_click=on_certify />
                </div>

                <div class="field">
                    <p class="description">
                        "Replace your local commits and local files with the remote "
                        <code>"latest"</code>
                        ". Any uncommitted edits in the working tree are lost."
                    </p>
                    <buttons::ResetLocal on_click=on_reset />
                </div>
            </div>
            {promote_confirmation}
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
    let uri = data.uri.clone();
    let origin_url = data.uri.as_ref().and_then(util::catalog_url);

    ToolbarActions::new(move || {
        let navigate = use_navigate();

        let ns_for_open = namespace.clone();
        let uri_for_open = uri.clone();
        let on_open_file_browser = move |_| {
            let ns = ns_for_open.clone();
            let uri = uri_for_open.clone();
            leptos::task::spawn_local(async move {
                match commands::open_in_file_browser(ns.to_string(), uri).await {
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
        let uri_for_uninstall = uri.clone();
        let on_uninstall = move |_| {
            let ns = ns_for_uninstall.clone();
            let uri = uri_for_uninstall.clone();
            let navigate = navigate.clone();
            ui_locked.set(true);
            leptos::task::spawn_local(async move {
                // v1 keeps the downloaded files: only the v2 confirmation
                // offers to delete them.
                match commands::package_uninstall(ns.to_string(), uri, false).await {
                    Ok(msg) => {
                        notification.set(Some(Notification::Success(msg)));
                        navigate("/", NavigateOptions::default());
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
                <buttons::OpenInFileBrowser on_click=on_open_file_browser small=true />
            </li>
            {has_catalog.then(|| view! {
                <li>
                    <buttons::OpenInCatalog on_click=on_open_catalog small=true />
                </li>
            })}
            <li>
                <buttons::Remove on_click=on_uninstall small=true />
            </li>
        }
        .into_any()
    })
}

#[cfg(test)]
mod tests {
    use leptos::prelude::*;
    use leptos_router::components::Router;
    use quilt_ui_kit::test_support::{
        button_saying, element_saying, mount, sleep_ms, unmount_earlier,
    };
    use serde_json::{Value, json};
    use wasm_bindgen_test::*;

    use super::MergeContent;
    use crate::commands::MergeData;
    use crate::engine::fake::FakeEngine;

    async fn merge_page() -> web_sys::Element {
        unmount_earlier();
        let el = mount(move || {
            view! {
                <Router>
                    <MergeContent
                        data=MergeData {
                            namespace: ("team", "dataset").into(),
                            uri: None,
                        }
                        notification=RwSignal::new(None)
                        ui_locked=RwSignal::new(false)
                    />
                </Router>
            }
        });
        sleep_ms(0).await;
        el
    }

    fn asked(engine: &FakeEngine, cmd: &str) -> Option<Value> {
        engine.requests().into_iter().find(|r| r["cmd"] == cmd)
    }

    /// Press *Promote my revision* and answer what `latest` would lose.
    async fn promote(engine: &FakeEngine, el: &web_sys::Element, loss: Value) {
        button_saying(el, "Promote my revision").click();
        sleep_ms(0).await;
        // The first ask opened the socket; it waits for the hello.
        engine.hello();
        sleep_ms(0).await;
        let ask = asked(engine, "get_latest_loss").expect("the loss is asked");
        assert_eq!(ask["args"]["namespace"], "team/dataset");
        engine.reply(&ask["id"], &loss);
        sleep_ms(0).await;
    }

    fn open_dialog(el: &web_sys::Element) -> Option<web_sys::Element> {
        el.query_selector("dialog[open]").unwrap()
    }

    #[wasm_bindgen_test]
    async fn promoting_what_latest_holds_runs_without_asking() {
        let engine = FakeEngine::install();
        let el = merge_page().await;
        promote(&engine, &el, json!({"entries": 0, "metadata": false})).await;

        assert!(open_dialog(&el).is_none(), "no confirmation");
        assert!(asked(&engine, "certify_latest").is_some(), "it certifies");
        unmount_earlier();
    }

    #[wasm_bindgen_test]
    async fn promoting_over_published_files_asks_first_and_counts_them() {
        let engine = FakeEngine::install();
        let el = merge_page().await;
        promote(&engine, &el, json!({"entries": 2, "metadata": false})).await;

        let dialog = open_dialog(&el).expect("the confirmation is open");
        element_saying(
            &dialog,
            "Everyone who syncs loses the published version of 2 files.",
        );
        assert!(
            asked(&engine, "certify_latest").is_none(),
            "nothing ran yet"
        );

        button_saying(&dialog, "Promote my revision").click();
        sleep_ms(0).await;
        assert!(
            asked(&engine, "certify_latest").is_some(),
            "confirming certifies"
        );
        unmount_earlier();
    }

    #[wasm_bindgen_test]
    async fn promoting_over_other_published_metadata_asks_first() {
        let engine = FakeEngine::install();
        let el = merge_page().await;
        promote(&engine, &el, json!({"entries": 0, "metadata": true})).await;

        let dialog = open_dialog(&el).expect("the confirmation is open");
        element_saying(&dialog, "The published metadata is replaced with yours.");
        assert!(
            asked(&engine, "certify_latest").is_none(),
            "nothing ran yet"
        );
        unmount_earlier();
    }
}
