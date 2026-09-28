use leptos::prelude::*;
use leptos_router::components::{Route, Router, Routes};
use leptos_router::path;
use std::future::Future;
use std::pin::Pin;

#[cfg(test)]
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

// The modules themselves live in the library beside this file — see `src/lib.rs`
// for why. Imported rather than declared, so this binary and the gallery share
// one compilation of them and neither can call an item dead that the other uses.
use quilt_sync_ui::commands;
use quilt_sync_ui::components;
use quilt_sync_ui::kit;
use quilt_sync_ui::pages;
use quilt_sync_ui::panic_report;

fn main() {
    console_error_panic_hook::set_once();
    // After the console hook, so it chains onto it rather than being replaced by it.
    panic_report::install();
    // Before the mount, so the first paint is in the right palette. The frame
    // before this one belongs to `index.html` — Rust cannot run early enough for
    // it.
    quilt_sync_ui::theme::follow_os();
    // The launch marker comes off before anything is drawn: it darkens the bare
    // canvas, which only an empty frame may have.
    quilt_sync_ui::theme::stop_booting();
    mount_to_body(App);
}

#[component]
fn App() -> impl IntoView {
    // Before the singletons, so the appbar's activity line and its producer
    // below both find it. Without it the line draws nothing.
    provide_context(kit::Activities::new());
    view! {
        <components::UpdateChecker />
        <components::ToastStack />
        <components::AutopullActivityFeed />
        <components::QuitPrompt />
        <Router>
            <Routes fallback=|| view! { <pages::NotFound /> }>
                <Route path=path!("/") view=|| view! { <Home /> } />
                <Route path=path!("/commit") view=pages::Commit />
                <Route path=path!("/installed-package") view=|| view! { <PackagePage read=read_settings /> } />
                <Route path=path!("/installed-packages-list") view=pages::InstalledPackagesList />
                <Route path=path!("/login") view=pages::Login />
                <Route path=path!("/main") view=pages::MainPage />
                <Route path=path!("/error") view=pages::Error />
                <Route path=path!("/merge") view=pages::Merge />
                <Route path=path!("/remote-package") view=pages::RemotePackage />
                <Route path=path!("/settings") view=pages::Settings />
                <Route path=path!("/setup") view=pages::Setup />
            </Routes>
        </Router>
    }
}

/// `/` is the main page, and the design preview decides which one it is.
///
/// Rendered here rather than redirected to: one route means every way home —
/// the logo, a breadcrumb, a Cancel — arrives at the page the reader has switched
/// on, instead of at whichever page the link was written against. `/main` and
/// `/installed-packages-list` stay reachable on purpose, for looking at one
/// specific page while both exist.
#[component]
fn Home() -> impl IntoView {
    view! {
        <ByDesign
            read=read_settings
            v2=|| view! { <pages::MainPage /> }.into_any()
            v1=|| view! { <pages::InstalledPackagesList /> }.into_any()
            loading="Loading QuiltSync"
        />
    }
}

/// `/installed-package` is one package's screen, and the design preview decides
/// which one — the same answer, read the same way, as [`Home`].
///
/// One route, the answer read in place, for the reason [`Home`] has it: every
/// link to a package (a roster row, a queue remedy, a deep link, a post-commit
/// return) lands on the page the reader has switched on rather than on whichever
/// one the link was written against. There is no second address for either page.
///
/// `read` is a parameter so a test can answer the settings read without a Tauri
/// host; the route passes [`read_settings`].
#[component]
fn PackagePage(read: SettingsRead) -> impl IntoView {
    view! {
        <ByDesign
            read=read
            v2=|| view! { <pages::InstalledPackageV2 /> }.into_any()
            v1=|| view! { <pages::InstalledPackage /> }.into_any()
            loading="Loading package"
        />
    }
}

/// The settings read a route decides its generation from.
type SettingsRead = fn() -> Pin<Box<dyn Future<Output = Result<commands::SettingsData, String>>>>;

fn read_settings() -> Pin<Box<dyn Future<Output = Result<commands::SettingsData, String>>>> {
    Box::pin(commands::get_settings_data())
}

/// One route that is two pages: `v2` when [`design_preview`] says so, `v1`
/// otherwise, asked afresh on every visit.
///
/// A fetch, so there is one frame with nothing on it. That is deliberate over
/// guessing: showing v1 and then swapping it for v2 is worse than a blank frame.
/// Asked per visit rather than once for the session, because saving the
/// preference in Settings has to take effect on the next page the reader opens,
/// and a cached answer would need a second path to hear about that.
///
/// Recorded on the root here rather than fetched again out in `App`: this is the
/// one read of the preference the app already makes, and `/` is where every
/// session starts, so the marker is set before any other route can be reached
/// and updated whenever the reader comes back having changed it. A launch by
/// deep link starts on `/remote-package` instead, and hands over to
/// `/installed-package`, which sets it there.
#[component]
fn ByDesign(
    read: SettingsRead,
    v2: fn() -> AnyView,
    v1: fn() -> AnyView,
    /// What the loading frame's spinner announces.
    loading: &'static str,
) -> impl IntoView {
    let settings = LocalResource::new(read);

    view! {
        <Suspense fallback=move || design_loading(loading)>
            {move || Suspend::new(async move {
                let settings = settings.await;
                let on = design_preview(settings.as_ref().map_err(String::as_str));
                quilt_sync_ui::theme::set_v2(on);
                if on { v2() } else { v1() }
            })}
        </Suspense>
    }
}

/// What a [`ByDesign`] route shows while it works out which page it is.
///
/// A spinner, which `kit::Spinner`'s own doc reserves for two jobs — this is the
/// second, "filling a region that cannot be skeletonised because its contents are
/// not a list of rows". A skeleton is the right loading state for a list, because
/// it holds the shape the content will take; here not even the page is decided
/// yet, so there is no shape to hold. No appbar around it either: drawing one
/// page's chrome and then swapping it for the other's is the flicker these routes
/// exist to avoid.
///
/// The ground is these routes' alone. `data-home-frame` is styled off
/// `theme::set_v2`'s root marker (`_base.scss`), which records the reader's
/// design preview — see [`design_preview`] — and a frame may paint from that
/// marker only where it predicts what is coming. That is here and nowhere else:
/// `/` and `/installed-package` render v2 on exactly the value the marker is set
/// from, so the ground the frame paints is the ground the page then keeps.
fn design_loading(label: &'static str) -> AnyView {
    view! {
        <div data-home-frame>
            <kit::Spinner variant=kit::SpinnerVariant::Region aria_label=label />
        </div>
    }
    .into_any()
}

/// Whether the reader has opted into the redesigned application generation,
/// given the settings fetch's outcome.
///
/// `Err` — the fetch failed — falls back to v1, same as the preference being off:
/// v1 is the generation that has always worked.
fn design_preview(settings: Result<&commands::SettingsData, &str>) -> bool {
    settings.is_ok_and(|data| data.experimental.main_page_v2)
}

#[cfg(test)]
mod tests {
    use super::*;
    use commands::{
        AutosyncSettingsData, ExperimentalSettingsData, FsWatcherSettingsData, PublishSettingsData,
        SettingsData,
    };
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::*;

    fn settings_stub(main_page_v2: bool) -> SettingsData {
        SettingsData {
            version: String::new(),
            home_dir: None,
            data_dir: String::new(),
            auth_hosts: Vec::new(),
            log_level: String::new(),
            logs_dir: String::new(),
            logs_dir_is_temporary: false,
            os: String::new(),
            changelog: Vec::new(),
            publish: PublishSettingsData::default(),
            autosync: AutosyncSettingsData::default(),
            fswatcher: FsWatcherSettingsData::default(),
            experimental: ExperimentalSettingsData {
                entire_package_sync: false,
                main_page_v2,
            },
        }
    }

    #[wasm_bindgen_test]
    fn the_loading_frame_announces_itself() {
        // `/` cannot skeletonise — it does not know which page is coming — so it
        // spins, and a spinner with nothing beside it has to say what it is
        // waiting on. `kit::Spinner`'s own doc: a `Region` spinner "always passes
        // something", rendered as off-screen text INSIDE the live region rather
        // than as a label on it, or the announcement may never fire.
        //
        // This pins that the frame is not empty and that it speaks. It cannot
        // pin what it looks like: no stylesheet is loaded here.
        let el = Mounted::new(|| design_loading("Loading QuiltSync"));
        // Inside the frame `_base.scss` grounds for a v2 reader — the two are
        // pinned together because the frame is what a stylesheet can see.
        let status = el
            .query_selector("[data-home-frame] [role=status]")
            .unwrap()
            .expect("the loading frame is a live region inside the grounded frame");
        assert_eq!(
            status.text_content().unwrap().trim(),
            "Loading QuiltSync",
            "and it says what it is waiting on"
        );
    }

    fn settings_on() -> Pin<Box<dyn Future<Output = Result<SettingsData, String>>>> {
        Box::pin(async { Ok(settings_stub(true)) })
    }

    fn settings_off() -> Pin<Box<dyn Future<Output = Result<SettingsData, String>>>> {
        Box::pin(async { Ok(settings_stub(false)) })
    }

    fn settings_fail() -> Pin<Box<dyn Future<Output = Result<SettingsData, String>>>> {
        Box::pin(async { Err("no settings".to_string()) })
    }

    async fn sleep_ms(ms: i32) {
        let promise = js_sys::Promise::new(&mut |resolve, _| {
            web_sys::window()
                .unwrap()
                .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms)
                .unwrap();
        });
        wasm_bindgen_futures::JsFuture::from(promise).await.unwrap();
    }

    /// A view mounted under `<body>`, torn down when this is dropped — after the
    /// test's assertions, and on a failing one too.
    ///
    /// `test_support::mount` is the library's and `#[cfg(test)]`, so this binary
    /// cannot reach it; this is the same shape, but owning its teardown: a
    /// `Router` left mounted keeps listening for clicks on the window, and the
    /// first one registered takes them from every later test's.
    struct Mounted {
        el: web_sys::Element,
        owner: Owner,
        handle: Option<Box<dyn std::any::Any>>,
    }

    impl Mounted {
        fn new<N: IntoView + 'static>(f: impl FnOnce() -> N + 'static) -> Self {
            let doc = web_sys::window().unwrap().document().unwrap();
            let host: web_sys::HtmlElement = doc.create_element("div").unwrap().dyn_into().unwrap();
            doc.body().unwrap().append_child(&host).unwrap();
            let owner = Owner::new();
            let handle = owner.with(|| leptos::mount::mount_to(host.clone(), f));
            Self {
                el: host.into(),
                owner,
                handle: Some(Box::new(handle)),
            }
        }
    }

    impl std::ops::Deref for Mounted {
        type Target = web_sys::Element;
        fn deref(&self) -> &web_sys::Element {
            &self.el
        }
    }

    impl Drop for Mounted {
        fn drop(&mut self) {
            // The route tests set the root marker. Put back here, after the
            // assertions, so the next test starts from v1's.
            quilt_sync_ui::theme::set_v2(false);
            drop(self.handle.take());
            self.owner.cleanup();
            self.el.remove();
        }
    }

    /// `/installed-package` as the router draws it, over a stubbed settings read.
    ///
    /// Neither page has a Tauri host here, so each draws its own failure — inside
    /// its own frame, which is what tells them apart: v2's `PageLayout` carries
    /// `data-v2-page`, v1's `Layout` is `#layout`. The route sets the marker;
    /// [`Mounted`]'s drop puts it back.
    async fn package_route(read: SettingsRead) -> Mounted {
        let el = Mounted::new(move || {
            view! {
                <Router>
                    <PackagePage read=read />
                </Router>
            }
        });
        sleep_ms(100).await;
        el
    }

    fn draws_v2(el: &web_sys::Element) -> bool {
        el.query_selector("[data-v2-page]").unwrap().is_some()
    }

    fn draws_v1(el: &web_sys::Element) -> bool {
        el.query_selector("#layout").unwrap().is_some()
    }

    #[wasm_bindgen_test]
    async fn the_package_route_is_v2_with_the_preview_on() {
        let el = package_route(settings_on).await;
        assert!(draws_v2(&el), "markup was {}", el.inner_html());
        assert!(
            !draws_v1(&el),
            "one page, not two; markup was {}",
            el.inner_html()
        );
    }

    #[wasm_bindgen_test]
    async fn the_package_route_is_v1_with_the_preview_off() {
        let el = package_route(settings_off).await;
        assert!(draws_v1(&el), "markup was {}", el.inner_html());
        assert!(!draws_v2(&el), "markup was {}", el.inner_html());
    }

    /// A failed read is v1 here too, as it is on `/`.
    #[wasm_bindgen_test]
    async fn the_package_route_is_v1_when_settings_cannot_be_read() {
        let el = package_route(settings_fail).await;
        assert!(draws_v1(&el), "markup was {}", el.inner_html());
        assert!(!draws_v2(&el), "markup was {}", el.inner_html());
    }

    #[wasm_bindgen_test]
    fn flag_on_renders_v2() {
        let settings = settings_stub(true);
        assert!(design_preview(Ok(&settings)));
    }

    #[wasm_bindgen_test]
    fn flag_off_renders_v1() {
        let settings = settings_stub(false);
        assert!(!design_preview(Ok(&settings)));
    }

    /// A failed read is v1: the generation that has always worked.
    #[wasm_bindgen_test]
    fn a_failed_settings_read_is_v1() {
        assert!(!design_preview(Err("nope")));
    }
}
