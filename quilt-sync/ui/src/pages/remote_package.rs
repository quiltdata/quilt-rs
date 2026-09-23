use leptos::prelude::*;
use leptos_router::NavigateOptions;
use leptos_router::hooks::{use_navigate, use_query_map};

use crate::commands;
use crate::components::{Layout, Spinner};

#[component]
pub fn RemotePackage() -> impl IntoView {
    let notification = RwSignal::new(None);
    let navigate = use_navigate();
    let query = use_query_map();

    let data = LocalResource::new(move || {
        let uri = query.read().get("uri").unwrap_or_default();
        let navigate = navigate.clone();
        async move {
            let result = commands::handle_remote_package(uri).await?;

            // Navigate to the installed package page
            let ns = &result.namespace;
            let base = crate::routes::package_page_href(ns);
            let path = match &result.banner {
                Some(commands::RemoteBanner::DifferentVersion {
                    requested_hash,
                    requested_bucket,
                    requested_origin,
                    ..
                }) => {
                    let path = format!(
                        "{base}&mismatch={}&mrbucket={}",
                        urlencoding::encode(requested_hash),
                        urlencoding::encode(requested_bucket),
                    );
                    match requested_origin {
                        Some(origin) => format!(
                            "{path}&mrcatalog={}",
                            urlencoding::encode(&origin.to_string())
                        ),
                        None => path,
                    }
                }
                Some(commands::RemoteBanner::LocalOnly) => format!("{base}&localOnly=1"),
                None => base,
            };
            navigate(&path, NavigateOptions::default());
            Ok::<_, String>(result)
        }
    });

    view! {
        // `transit`: this mount is the deep link's operation, so the preview's
        // bar — with a Refresh that reloads the window — must not reach it.
        <Layout breadcrumbs=vec![] notification=notification transit=true>
            <Suspense fallback=move || {
                view! { <Spinner /> }
            }>
                {move || Suspend::new(async move {
                    match data.await {
                        Ok(_) => view! { <Spinner /> }.into_any(),
                        Err(e) => {
                            crate::error_handler::handle_or_display(&e, notification)
                        }
                    }
                })}
            </Suspense>
        </Layout>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{mount, sleep_ms};
    use crate::theme;
    use leptos_router::components::Router;
    use wasm_bindgen_test::*;

    /// The relay renders inside the same v1 `Layout` every route the preview
    /// reaches does, so it is held out by name rather than by shape. Mounted
    /// with no Tauri host, so the deep link's own read rejects and runs nothing
    /// — and that rejection is the failure path, whose shared error page draws
    /// a `Layout` of its own. Awaited, so the assertion sees that one too.
    #[wasm_bindgen_test]
    async fn the_preview_does_not_hand_the_relay_its_bar() {
        theme::set_v2(true);
        let el = mount(|| {
            view! {
                <Router>
                    <RemotePackage />
                </Router>
            }
        });
        sleep_ms(100).await;
        theme::set_v2(false);

        assert!(
            el.text_content().unwrap_or_default().contains("Error"),
            "the failure path must have drawn; markup was {}",
            el.inner_html()
        );

        assert!(
            el.query_selector("header").unwrap().is_none(),
            "a Refresh here reloads the window and re-runs the deep link; markup was {}",
            el.inner_html()
        );
        assert!(
            el.query_selector(".qui-appbar").unwrap().is_some(),
            "the relay keeps the bar it draws today; markup was {}",
            el.inner_html()
        );
    }
}
