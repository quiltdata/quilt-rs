use leptos::prelude::*;
use leptos_router::NavigateOptions;
use leptos_router::hooks::use_navigate;

use crate::commands;
use crate::components::{Layout, Spinner};

#[component]
pub fn RemotePackage() -> impl IntoView {
    let notification = RwSignal::new(None);
    let navigate = use_navigate();
    let query = crate::routes::use_query();

    let data = LocalResource::new(move || {
        let uri = query.read().get("uri").unwrap_or_default();
        let navigate = navigate.clone();
        async move {
            let result = commands::handle_remote_package(uri).await?;

            let path = package_target(&result);
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
                            crate::error_handler::handle_or_display_in_shell(&e, notification)
                        }
                    }
                })}
            </Suspense>
        </Layout>
    }
}

/// The package page a handled deep link lands on, keeping what the link asked
/// for that the package does not have.
pub(crate) fn package_target(result: &commands::RemotePackageResult) -> String {
    let base = crate::routes::package_page_href(&result.namespace);
    let outcome = match &result.banner {
        Some(commands::RemoteBanner::DifferentVersion {
            requested_hash,
            requested_bucket,
            requested_origin,
            ..
        }) => Some(crate::routes::DeepLinkOutcome::Mismatch(
            crate::routes::RevisionMismatch {
                hash: requested_hash.clone(),
                bucket: requested_bucket.clone(),
                catalog: requested_origin.as_ref().map(ToString::to_string),
            },
        )),
        Some(commands::RemoteBanner::LocalOnly) => Some(crate::routes::DeepLinkOutcome::LocalOnly),
        None => None,
    };
    crate::routes::keeping_outcome(base, outcome.as_ref())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{mount, sleep_ms};
    use crate::theme;
    use leptos_router::components::Router;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::*;

    /// With no engine the read rejects, so this also covers the error
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
