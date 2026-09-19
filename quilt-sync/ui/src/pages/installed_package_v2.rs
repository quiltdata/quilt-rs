//! The v2 package page. Behind `ExperimentalSettings.package_page_v2`.
//!
//! A placeholder, and deliberately one: this landing is the *seam* — a flag, a
//! route that reads it, and a page that proves the route parameter arrives. The
//! page the design describes is built on top of this, region by region.

use leptos::prelude::*;
use leptos_router::hooks::use_query_map;

use crate::components::Layout;

/// What `/installed-package` renders for a reader with *New package page* on.
#[component]
pub fn InstalledPackageV2() -> impl IntoView {
    let query = use_query_map();
    // The address is the only input the page has yet. Read reactively, because
    // one route serves every package and a link from another page swaps the
    // parameter without remounting.
    let namespace = move || query.read().get("namespace").unwrap_or_default();

    view! {
        <Layout breadcrumbs=vec![] notification=RwSignal::new(None)>
            <div class="container">
                <h1 class="title">"It works!"</h1>
                <p class="message">{namespace}</p>
            </div>
        </Layout>
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
}
