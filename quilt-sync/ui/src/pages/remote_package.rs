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
            // Replace, so Back from the package page cannot land here and re-run it.
            navigate(
                &path,
                NavigateOptions {
                    replace: true,
                    ..NavigateOptions::default()
                },
            );
            Ok::<_, String>(result)
        }
    });

    view! {
        // Mounting runs the deep link, so nothing here may reload the window.
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
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::*;

    /// With no Tauri host the read rejects, so this also covers the error
    /// page drawn inside the relay's shell.
    #[wasm_bindgen_test]
    async fn the_relay_draws_the_preview_bar_once_with_refresh_disabled() {
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
        assert_eq!(
            el.query_selector_all("header").unwrap().length(),
            1,
            "one redesigned bar, and no second shell; markup was {}",
            el.inner_html()
        );
        let refresh = el
            .query_selector("header button")
            .unwrap()
            .expect("the bar draws Refresh")
            .dyn_into::<web_sys::HtmlButtonElement>()
            .unwrap();
        assert!(
            refresh.disabled(),
            "a Refresh here reloads the window and re-runs the deep link; markup was {}",
            el.inner_html()
        );
    }
}
