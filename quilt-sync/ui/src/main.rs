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
                <Route path=path!("/commit") view=|| view! { <CommitPage read=read_settings /> } />
                <Route path=path!("/installed-package") view=|| view! { <PackagePage read=read_settings /> } />
                <Route path=path!("/installed-packages-list") view=pages::InstalledPackagesList />
                <Route path=path!("/login") view=pages::Login />
                <Route path=path!("/main") view=pages::MainPage />
                <Route path=path!("/error") view=pages::Error />
                <Route path=path!("/merge") view=pages::Merge />
                <Route path=path!("/remote-package") view=pages::RemotePackage />
                <Route path=path!("/settings") view=|| view! { <SettingsPage read=read_settings /> } />
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
            skeleton=|| view! { <pages::MainPageSkeleton actions=loading_actions() /> }.into_any()
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
            skeleton=|| view! { <pages::PackagePageSkeleton actions=loading_actions() /> }.into_any()
            loading="Loading package"
        />
    }
}

/// `/commit` is the new revision's screen, and the design preview decides
/// which one, as for [`PackagePage`]: every way here is a link from a package
/// page, so the commit page is the one beside the package page the reader has.
#[component]
fn CommitPage(read: SettingsRead) -> impl IntoView {
    view! {
        <ByDesign
            read=read
            v2=|| view! { <pages::CommitV2 /> }.into_any()
            v1=|| view! { <pages::Commit /> }.into_any()
            skeleton=|| view! { <pages::CommitV2Skeleton actions=loading_actions() /> }.into_any()
            loading="Loading new revision"
        />
    }
}

/// `/settings`, decided as the other routes are: the v2 page under *New design
/// preview*, v1 otherwise. Turning the preview off saves, then goes to `/`,
/// which asks again.
#[component]
fn SettingsPage(read: SettingsRead) -> impl IntoView {
    view! {
        <ByDesign
            read=read
            v2=|| view! { <pages::SettingsV2 /> }.into_any()
            v1=|| view! { <pages::Settings /> }.into_any()
            skeleton=|| view! { <pages::SettingsV2Skeleton /> }.into_any()
            loading="Loading settings"
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
/// A fetch, so there is a frame before the answer — [`design_loading`] says what
/// it holds. Asked per visit rather than once for the session, because saving the
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
    /// `v2`'s first paint, for the loading frame when the root marker predicts
    /// `v2`.
    skeleton: fn() -> AnyView,
    /// What the loading frame announces.
    loading: &'static str,
) -> impl IntoView {
    let settings = LocalResource::new(read);

    view! {
        <Suspense fallback=move || design_loading(loading, skeleton)>
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
/// The page the root marker predicts. The marker is the answer this read gave
/// last time — `index.html` restores it before the first paint, and every
/// `ByDesign` sets it from its own answer — so it is wrong only when the
/// preference changed somewhere this app did not see, or when this read fails
/// and the route falls back to v1 under a v2 marker. For a v2 reader that makes
/// the frame `skeleton`, the v2 page's own first paint: the appbar, then the page
/// in the shape it will take. The page then draws the same skeleton and fills it,
/// so the frame is the first step of the page's own loading and nothing moves at
/// either handover.
///
/// The prediction is cheap to get wrong. A stale marker costs one v2 skeleton
/// frame before v1, and that frame is a skeleton rather than a real page, so the
/// swap replaces placeholders instead of content the reader has started to read.
/// A v1 reader gets the spinner v1's own pages load behind: v1 has no skeletons
/// to draw, and a v2 one would be the guess the marker has just ruled out.
///
/// The announcement is the frame's, not the skeleton's, so it speaks whichever
/// generation is coming and says what it is waiting on. Off-screen text inside
/// the live region, for `kit::Spinner`'s reason: a live region announces its
/// content, and one with nothing inside may never fire. It sits outside every
/// `aria-busy` region, which would hold its announcement back.
///
/// The ground is these routes' alone. `data-home-frame` is styled off
/// `theme::set_v2`'s root marker (`_base.scss`), which records the reader's
/// design preview — see [`design_preview`] — and a frame may paint from that
/// marker only where it predicts what is coming. That is here and nowhere else:
/// `/` and `/installed-package` render v2 on exactly the value the marker is set
/// from, so the ground the frame paints is the ground the page then keeps.
fn design_loading(label: &'static str, skeleton: fn() -> AnyView) -> AnyView {
    let page = if quilt_sync_ui::theme::is_v2() {
        skeleton()
    } else {
        view! { <components::Spinner /> }.into_any()
    };
    view! {
        <div data-home-frame>
            <p role="status" data-sr-only>
                {label}
            </p>
            {page}
        </div>
    }
    .into_any()
}

/// The appbar's controls in a loading frame: Refresh spinning, as the page
/// draws it while its first read is out, so the bar does not change at the
/// handover. A spinning Button is disabled, so it takes no press.
///
/// Settings stays live, unlike the frame's list toolbar. It navigates to a page
/// of its own and depends on nothing the frame holds, so a reader who presses
/// it during the wait gets exactly what they asked for; there is nothing to
/// lose.
fn loading_actions() -> AnyView {
    components::appbar::appbar_actions(|| (), Signal::stored(true))
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
            log_env: commands::LogEnv::Unset,
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

    /// The frame `/` draws while it decides, in whichever generation the root
    /// marker predicts, under a router because the v2 appbar's Settings navigates.
    fn home_frame(v2: bool) -> Mounted {
        quilt_sync_ui::theme::set_v2(v2);
        Mounted::new(|| {
            view! {
                <Router>
                    {design_loading("Loading QuiltSync", || {
                        view! { <pages::MainPageSkeleton actions=loading_actions() /> }.into_any()
                    })}
                </Router>
            }
        })
    }

    #[wasm_bindgen_test]
    fn the_loading_frame_announces_itself() {
        // The frame's skeleton or spinner says nothing on its own — the bars are
        // hidden and v1's rings are markup — so the frame has to say what it is
        // waiting on. `kit::Spinner`'s rule: off-screen text INSIDE the live
        // region rather than a label on it, or the announcement may never fire.
        //
        // This pins that the frame speaks, in both generations. It cannot pin
        // what it looks like: no stylesheet is loaded here.
        for v2 in [true, false] {
            let el = home_frame(v2);
            // Inside the frame `_base.scss` grounds for a v2 reader — the two are
            // pinned together because the frame is what a stylesheet can see.
            let status = el
                .query_selector("[data-home-frame] [role=status]")
                .unwrap()
                .expect("the loading frame is a live region inside the grounded frame");
            assert_eq!(
                status.text_content().unwrap().trim(),
                "Loading QuiltSync",
                "and it says what it is waiting on (v2: {v2})"
            );
            // `aria-busy` on an ancestor holds a live region's news back until it
            // clears, which is when there is nothing left to announce.
            assert!(
                status.closest("[aria-busy=true]").unwrap().is_none(),
                "and nothing busy around it silences it (v2: {v2})"
            );
        }
    }

    /// The marker predicts v2, so the frame is the v2 page's own first paint:
    /// its appbar and its skeleton, with the skeleton's region marked busy. Not
    /// the tiny region spinner the frame used to hold, nor v1's.
    #[wasm_bindgen_test]
    fn a_v2_reader_waits_on_the_page_s_skeleton() {
        let el = home_frame(true);
        let frame = el.query_selector("[data-home-frame]").unwrap().unwrap();
        assert!(
            frame.query_selector("[data-v2-page]").unwrap().is_some(),
            "the v2 page's frame, appbar and all; markup was {}",
            el.inner_html()
        );
        assert!(
            frame.query_selector("[aria-busy=true]").unwrap().is_some(),
            "a skeleton region that says it is busy; markup was {}",
            el.inner_html()
        );
        assert!(
            frame.query_selector(".q-spinner").unwrap().is_none(),
            "and no spinner beside it; markup was {}",
            el.inner_html()
        );
    }

    /// A v1 reader gets the spinner v1's own pages load behind, and no v2
    /// surface: a v2 skeleton is a prediction the marker has ruled out.
    #[wasm_bindgen_test]
    fn a_v1_reader_waits_on_v1_s_spinner() {
        let el = home_frame(false);
        assert!(
            el.query_selector("[data-home-frame] .q-spinner")
                .unwrap()
                .is_some(),
            "markup was {}",
            el.inner_html()
        );
        assert!(!draws_v2(&el), "markup was {}", el.inner_html());
    }

    /// `/installed-package` before its settings read answers: the package
    /// page's skeleton for a v2 reader, announced as the package it is.
    #[wasm_bindgen_test]
    async fn the_package_route_waits_on_the_package_skeleton() {
        quilt_sync_ui::theme::set_v2(true);
        let el = package_route(settings_pending).await;
        let status = el
            .query_selector("[data-home-frame] [role=status]")
            .unwrap()
            .expect("the frame, still up: the read never answers");
        assert_eq!(status.text_content().unwrap().trim(), "Loading package");
        assert!(
            el.query_selector("[data-home-frame] [data-v2-page] [aria-label=Files]")
                .unwrap()
                .is_some(),
            "the package page's panes, not the main page's list; markup was {}",
            el.inner_html()
        );
    }

    fn settings_on() -> Pin<Box<dyn Future<Output = Result<SettingsData, String>>>> {
        Box::pin(async { Ok(settings_stub(true)) })
    }

    fn settings_off() -> Pin<Box<dyn Future<Output = Result<SettingsData, String>>>> {
        Box::pin(async { Ok(settings_stub(false)) })
    }

    fn settings_pending() -> Pin<Box<dyn Future<Output = Result<SettingsData, String>>>> {
        Box::pin(std::future::pending())
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

    /// `/commit` as the router draws it, over a stubbed settings read.
    async fn commit_route(read: SettingsRead) -> Mounted {
        let el = Mounted::new(move || {
            view! {
                <Router>
                    <CommitPage read=read />
                </Router>
            }
        });
        sleep_ms(100).await;
        el
    }

    #[wasm_bindgen_test]
    async fn the_commit_route_is_v2_with_the_preview_on() {
        let el = commit_route(settings_on).await;
        assert!(draws_v2(&el), "markup was {}", el.inner_html());
        assert!(!draws_v1(&el), "markup was {}", el.inner_html());
    }

    #[wasm_bindgen_test]
    async fn the_commit_route_is_v1_with_the_preview_off() {
        let el = commit_route(settings_off).await;
        assert!(draws_v1(&el), "markup was {}", el.inner_html());
        assert!(!draws_v2(&el), "markup was {}", el.inner_html());
    }

    #[wasm_bindgen_test]
    async fn the_commit_route_is_v1_when_settings_cannot_be_read() {
        let el = commit_route(settings_fail).await;
        assert!(draws_v1(&el), "markup was {}", el.inner_html());
    }

    /// `/settings` as the router draws it, over a stubbed settings read.
    async fn settings_route(read: SettingsRead) -> Mounted {
        let el = Mounted::new(move || {
            view! {
                <Router>
                    <SettingsPage read=read />
                </Router>
            }
        });
        sleep_ms(100).await;
        el
    }

    #[wasm_bindgen_test]
    async fn the_settings_route_is_v2_with_the_preview_on() {
        let el = settings_route(settings_on).await;
        assert!(draws_v2(&el), "markup was {}", el.inner_html());
        assert!(!draws_v1(&el), "markup was {}", el.inner_html());
    }

    #[wasm_bindgen_test]
    async fn the_settings_route_is_v1_with_the_preview_off() {
        let el = settings_route(settings_off).await;
        assert!(draws_v1(&el), "markup was {}", el.inner_html());
        assert!(!draws_v2(&el), "markup was {}", el.inner_html());
    }

    #[wasm_bindgen_test]
    async fn the_settings_route_waits_on_the_settings_skeleton() {
        quilt_sync_ui::theme::set_v2(true);
        let el = settings_route(settings_pending).await;
        let status = el
            .query_selector("[data-home-frame] [role=status]")
            .unwrap()
            .expect("the frame, still up: the read never answers");
        assert_eq!(status.text_content().unwrap().trim(), "Loading settings");
        assert!(
            el.query_selector("[data-home-frame] [data-v2-page] [aria-busy=true]")
                .unwrap()
                .is_some(),
            "the settings page's skeleton; markup was {}",
            el.inner_html()
        );
    }

    #[wasm_bindgen_test]
    async fn the_commit_route_waits_on_the_commit_skeleton() {
        quilt_sync_ui::theme::set_v2(true);
        let el = commit_route(settings_pending).await;
        let status = el
            .query_selector("[data-home-frame] [role=status]")
            .unwrap()
            .expect("the frame, still up: the read never answers");
        assert_eq!(
            status.text_content().unwrap().trim(),
            "Loading new revision"
        );
        assert!(
            el.query_selector("[data-home-frame] [data-v2-page] [aria-busy=true]")
                .unwrap()
                .is_some(),
            "the commit page's skeleton; markup was {}",
            el.inner_html()
        );
    }

    #[test]
    fn flag_on_renders_v2() {
        let settings = settings_stub(true);
        assert!(design_preview(Ok(&settings)));
    }

    #[test]
    fn flag_off_renders_v1() {
        let settings = settings_stub(false);
        assert!(!design_preview(Ok(&settings)));
    }

    /// A failed read is v1: the generation that has always worked.
    #[test]
    fn a_failed_settings_read_is_v1() {
        assert!(!design_preview(Err("nope")));
    }
}
