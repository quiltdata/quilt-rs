//! The v2 package page. Behind `ExperimentalSettings.package_page_v2`.
//!
//! A placeholder, and deliberately one: this landing is the *seam* — a flag, a
//! route that reads it, and a page that proves the route parameter arrives. The
//! page the design describes is built on top of this, region by region.

use leptos::prelude::*;
use leptos_router::NavigateOptions;
use leptos_router::hooks::use_navigate;
use leptos_router::hooks::use_query_map;

use crate::kit::{Button, PageLayout, icons};

/// What `/installed-package` renders for a reader with *New package page* on.
#[component]
pub fn InstalledPackageV2() -> impl IntoView {
    let query = use_query_map();
    // The address is the only input the page has yet. Read reactively, because
    // one route serves every package and a link from another page swaps the
    // parameter without remounting.
    let namespace = move || query.read().get("namespace").unwrap_or_default();
    let navigate = use_navigate();

    // `heading` is not reactive and one route serves every package, so it names
    // the page rather than the package; the package's own name is on screen.
    view! {
        <PageLayout
            heading="Package"
            actions=view! {
                // The one control the placeholder genuinely needs: it is reached
                // by a switch in Settings, so it has to offer the way back to it.
                // The logo only reaches `/`.
                <Button
                    leading_visual=icons::gear()
                    on_click=move |_| navigate("/settings", NavigateOptions::default())
                >
                    "Settings"
                </Button>
            }
                .into_any()
        >
            <h2>"It works!"</h2>
            <p>{namespace}</p>
        </PageLayout>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{element_saying, mount, sleep_ms};
    use leptos_router::components::{Route, Router, Routes};
    use leptos_router::path;
    use wasm_bindgen_test::*;

    /// Put the browser on an address before the router reads one. Same origin,
    /// so the history write is allowed; the runner's own page is whatever it is.
    fn go_to(address: &str) {
        web_sys::window()
            .unwrap()
            .history()
            .unwrap()
            .replace_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some(address))
            .unwrap();
    }

    /// The one thing the placeholder is for: proving the route parameter
    /// arrives. A page that drew a fixed string would pass a weaker test and
    /// tell the next unit nothing.
    /// The one thing the placeholder is for: proving the route parameter
    /// arrives. A page drawing a fixed string would pass a weaker test and tell
    /// the next unit nothing.
    ///
    /// Async because the router resolves a location one tick after the mount —
    /// queried synchronously the container is still a comment marker.
    #[wasm_bindgen_test]
    async fn the_page_names_the_package_the_address_asked_for() {
        go_to("/installed-package?namespace=team%2Fdataset");
        let el = mount(|| {
            view! {
                <Router>
                    <Routes fallback=|| view! { "no route" }>
                        <Route path=path!("/installed-package") view=InstalledPackageV2 />
                    </Routes>
                </Router>
            }
        });
        sleep_ms(50).await;

        element_saying(&el, "It works!");
        element_saying(&el, "team/dataset");
    }

    /// v2's frame, not v1's. `data-v2-page` is what the stylesheet keys the
    /// palette and `color-scheme` on, so without it the page is drawn in v1's
    /// fixed light chrome whatever the desktop is set to.
    #[wasm_bindgen_test]
    async fn the_page_is_drawn_in_the_v2_frame() {
        go_to("/installed-package?namespace=team%2Fdataset");
        let el = mount(|| {
            view! {
                <Router>
                    <Routes fallback=|| view! { "no route" }>
                        <Route path=path!("/installed-package") view=InstalledPackageV2 />
                    </Routes>
                </Router>
            }
        });
        sleep_ms(50).await;

        assert!(
            el.query_selector("[data-v2-page]").unwrap().is_some(),
            "the placeholder sits in the v2 page frame; markup was {}",
            el.inner_html()
        );
    }
}
