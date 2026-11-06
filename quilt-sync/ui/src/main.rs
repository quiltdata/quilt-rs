use leptos::prelude::*;
use leptos_router::components::{Route, Router, Routes};
use leptos_router::path;

#[cfg(test)]
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

// The modules themselves live in the library beside this file — see `src/lib.rs`
// for why. Imported rather than declared, so this binary and the gallery share
// one compilation of them and neither can call an item dead that the other uses.
use quilt_sync_ui::components;
use quilt_sync_ui::pages;
use quilt_sync_ui::panic_report;
use quilt_ui_kit as kit;

fn main() {
    console_error_panic_hook::set_once();
    // After the console hook, so it chains onto it rather than being replaced by it.
    panic_report::install();
    // Before the mount, so the first paint is in the right palette. The frame
    // before this one belongs to `index.html` — Rust cannot run early enough for
    // it.
    quilt_ui_kit::theme::follow_os();
    // The launch marker comes off before anything is drawn: it darkens the bare
    // canvas, which only an empty frame may have.
    quilt_ui_kit::theme::stop_booting();
    // Before the router reads the address: a link's token leaves it here.
    quilt_sync_ui::engine::start();
    if quilt_sync_ui::engine::linked() {
        mount_to_body(App);
    } else {
        mount_to_body(components::NoLink);
    }
}

#[component]
fn App() -> impl IntoView {
    // Before the singletons, so the appbar's activity line and its producer
    // below both find it. Without it the line draws nothing.
    provide_context(kit::Activities::new());
    view! {
        <components::ConnectionBanner />
        <components::UpdateChecker />
        <components::ToastStack />
        <components::AutopullActivityFeed />
        <components::QuitPrompt />
        <Router>
            <components::FollowEngine />
            <DocumentTitle />
            <PageRoutes />
        </Router>
    }
}

/// Every page, by address. Each page draws its own skeleton while its first
/// read is out, so the route has no loading frame of its own.
#[component]
fn PageRoutes() -> impl IntoView {
    view! {
        <Routes fallback=pages::NotFoundV2>
            <Route path=path!("/") view=pages::MainPage />
            <Route path=path!("/commit") view=pages::CommitV2 />
            <Route path=path!("/installed-package") view=pages::InstalledPackageV2 />
            <Route path=path!("/login") view=pages::LoginV2 />
            <Route path=path!("/error") view=pages::ErrorV2 />
            <Route path=path!("/remote-package") view=pages::RemotePackageV2 />
            <Route path=path!("/settings") view=pages::SettingsV2 />
            <Route path=path!("/setup") view=pages::SetupV2 />
        </Routes>
    }
}

/// The browser tab names the page; see [`quilt_sync_ui::routes::page_title`].
#[component]
fn DocumentTitle() -> impl IntoView {
    let location = leptos_router::hooks::use_location();
    Effect::new(move |_| {
        let query = quilt_sync_ui::routes::Query::parse(&location.search.get());
        document().set_title(&quilt_sync_ui::routes::page_title(
            &location.pathname.get(),
            &query,
        ));
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::*;

    /// The routes mounted at `address`, torn down when this is dropped: a
    /// `Router` left mounted keeps listening for clicks on the window, and the
    /// first one registered takes them from every later test's.
    struct Mounted {
        el: web_sys::Element,
        owner: Owner,
        handle: Option<Box<dyn std::any::Any>>,
    }

    impl Mounted {
        fn at(address: &str) -> Self {
            let window = web_sys::window().unwrap();
            window
                .history()
                .unwrap()
                .replace_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some(address))
                .unwrap();
            let doc = window.document().unwrap();
            let host: web_sys::HtmlElement = doc.create_element("div").unwrap().dyn_into().unwrap();
            doc.body().unwrap().append_child(&host).unwrap();
            let owner = Owner::new();
            let handle = owner.with(|| {
                leptos::mount::mount_to(host.clone(), || {
                    view! {
                        <Router>
                            <PageRoutes />
                        </Router>
                    }
                })
            });
            Self {
                el: host.into(),
                owner,
                handle: Some(Box::new(handle)),
            }
        }
    }

    impl Drop for Mounted {
        fn drop(&mut self) {
            drop(self.handle.take());
            self.owner.cleanup();
            self.el.remove();
        }
    }

    /// The first paint of every route is its page's own frame, with the
    /// regions still to be read marked busy: no loading frame of the route's
    /// own comes first.
    #[wasm_bindgen_test]
    fn every_route_first_paints_its_page_s_skeleton() {
        for (address, reads) in [
            ("/", true),
            ("/installed-package?namespace=team%2Fdataset", true),
            ("/commit?namespace=team%2Fdataset", true),
            ("/settings", true),
            ("/login?host=quilt.test&back=%2F", true),
            ("/setup", true),
            ("/error?host=quilt.test&error=nope", true),
            (
                "/remote-package?uri=quilt%2Bs3%3A%2F%2Fb%23package%3Dteam%2Fdataset",
                false,
            ),
            ("/no-such-page", false),
        ] {
            let mounted = Mounted::at(address);
            let el = &mounted.el;
            assert!(
                el.query_selector("[data-v2-page]").unwrap().is_some(),
                "{address}: the page's frame, appbar and all; markup was {}",
                el.inner_html()
            );
            if reads {
                assert!(
                    el.query_selector("[data-v2-page] [aria-busy=true]")
                        .unwrap()
                        .is_some(),
                    "{address}: a skeleton region that says it is busy; markup was {}",
                    el.inner_html()
                );
            }
        }
    }

    /// The pages the classic design drew alone are gone, and their addresses
    /// are no longer pages.
    #[wasm_bindgen_test]
    fn the_classic_addresses_are_not_found() {
        for address in [
            "/main",
            "/installed-packages-list",
            "/merge?namespace=team%2Fdataset",
        ] {
            let mounted = Mounted::at(address);
            assert!(
                mounted
                    .el
                    .text_content()
                    .unwrap_or_default()
                    .contains("Page not found"),
                "{address}: markup was {}",
                mounted.el.inner_html()
            );
        }
    }
}
