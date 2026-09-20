use leptos::prelude::*;
use leptos_router::components::{Route, Router, Routes};
use leptos_router::path;

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
    view! {
        <components::UpdateChecker />
        <components::ToastStack />
        <components::QuitPrompt />
        <Router>
            <Routes fallback=|| view! { <pages::NotFound /> }>
                <Route path=path!("/") view=|| view! { <Home /> } />
                <Route path=path!("/commit") view=pages::Commit />
                <Route path=path!("/installed-package") view=|| view! { <PackagePage /> } />
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

/// `/` is the main page, and the flag decides which one it is.
///
/// Rendered here rather than redirected to: one route means every way home —
/// the logo, a breadcrumb, a Cancel — arrives at the page the reader has switched
/// on, instead of at whichever page the link was written against. `/main` and
/// `/installed-packages-list` stay reachable on purpose, for looking at one
/// specific page while both exist.
///
/// A fetch, so there is one frame with nothing on it. That is deliberate over
/// guessing: showing v1 and then swapping it for v2 is worse than a blank frame.
#[component]
fn Home() -> impl IntoView {
    let settings = LocalResource::new(|| async move { commands::get_settings_data().await });

    view! {
        <Suspense fallback=home_loading>
            {move || Suspend::new(async move {
                let settings = settings.await;
                let v2 = wants_main_page_v2(settings.as_ref().map_err(String::as_str));
                // Recorded on the root here rather than fetched again out in
                // `App`: this is the one read of the flag the app already makes,
                // and `/` is where every session starts, so the marker is set
                // before any other route can be reached and updated whenever the
                // reader comes back having changed it.
                quilt_sync_ui::theme::set_v2(v2);
                if v2 {
                    view! { <pages::MainPage /> }.into_any()
                } else {
                    view! { <pages::InstalledPackagesList /> }.into_any()
                }
            })}
        </Suspense>
    }
}

/// `/installed-package` is one package's screen, and the flag decides which one.
///
/// The shape [`Home`] has, for the reason [`Home`] has it: one route, the flag
/// read in place, so every link to a package — a roster row, a queue remedy, a
/// deep link, a post-commit return — lands on the page the reader switched on
/// rather than on whichever one the link was written against. There is no
/// second address for either page.
#[component]
fn PackagePage() -> impl IntoView {
    let settings = LocalResource::new(|| async move { commands::get_settings_data().await });

    view! {
        <Suspense fallback=|| loading("Loading package")>
            {move || Suspend::new(async move {
                let settings = settings.await;
                if wants_package_page_v2(settings.as_ref().map_err(String::as_str)) {
                    view! { <pages::InstalledPackageV2 /> }.into_any()
                } else {
                    view! { <pages::InstalledPackage /> }.into_any()
                }
            })}
        </Suspense>
    }
}

/// What a flag-reading route shows while it works out which page it is.
///
/// A spinner, which `kit::Spinner`'s own doc reserves for two jobs — this is the
/// second, "filling a region that cannot be skeletonised because its contents are
/// not a list of rows". A skeleton is the right loading state for a list, because
/// it holds the shape the content will take; here not even the page is decided
/// yet, so there is no shape to hold.
///
/// No appbar around it. Drawing one page's chrome and then swapping it for the
/// other's is the flicker this route exists to avoid. **And no ground** — see
/// [`home_loading`] for who may paint one and why only they may.
fn loading(aria_label: &'static str) -> AnyView {
    view! { <kit::Spinner variant=kit::SpinnerVariant::Region aria_label=aria_label /> }.into_any()
}

/// `/`'s loading frame, which also paints a ground.
///
/// `data-home-frame` is styled off `theme::set_v2`'s root marker (`_base.scss`),
/// and that marker records the *main page* opt-in. It may only be drawn where it
/// predicts what is coming, which is here and nowhere else: `/` renders v2
/// exactly when the marker is set, so the ground it paints is the ground the
/// page then keeps.
///
/// `/installed-package` gets [`loading`] bare for that reason. The marker says
/// nothing about which package page is coming, so a frame painted from it is
/// wrong for whichever flag disagrees — and a reader with the main page on and
/// the package page off would have been shown a dark ground before v1's light
/// chrome, having opted into nothing.
fn home_loading() -> AnyView {
    view! {
        <div data-home-frame>{loading("Loading QuiltSync")}</div>
    }
    .into_any()
}

/// Whether `/` renders v2, given the settings fetch's outcome.
///
/// `Err` — the fetch failed — falls back to v1, same as the flag being off:
/// v1 is the page that has always worked.
fn wants_main_page_v2(settings: Result<&commands::SettingsData, &str>) -> bool {
    settings.is_ok_and(|data| data.experimental.main_page_v2)
}

/// Whether `/installed-package` renders v2, on the same terms — and only under
/// the main page's opt-in.
///
/// **Both flags**, because the v2 package page assumes a v2 app around it: it is
/// drawn in that design, and its way back leads to the main page. A flag of its
/// own still, so the unfinished page is not forced on every reader of the v2
/// main page; but it narrows that opt-in rather than standing beside it. The two
/// merge into one switch when the page is finished.
///
/// The gate is on the effect, not only on the Settings row. A disabled box over
/// a live flag is a flag with no way to turn it off — and gating the effect is
/// how `entire_package_sync`'s gate already behaves: the stored choice is left
/// written, so restoring what gates it resumes the reader's answer.
fn wants_package_page_v2(settings: Result<&commands::SettingsData, &str>) -> bool {
    settings.is_ok_and(|data| data.experimental.main_page_v2 && data.experimental.package_page_v2)
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

    fn settings_stub(main_page_v2: bool, package_page_v2: bool) -> SettingsData {
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
                package_page_v2,
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
        let doc = web_sys::window().unwrap().document().unwrap();
        let host: web_sys::HtmlElement = doc.create_element("div").unwrap().dyn_into().unwrap();
        doc.body().unwrap().append_child(&host).unwrap();
        leptos::mount::mount_to(host.clone(), home_loading).forget();

        let el: web_sys::Element = host.into();
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

    /// The ground is `/`'s alone, so the frame the package route uses must not
    /// carry one.
    ///
    /// `data-home-frame` is painted from the main-page marker, so on the package
    /// route it is wrong for whichever flag disagrees — a reader with the main
    /// page on and the package page off got a dark full-viewport ground before
    /// v1's light chrome, having opted into nothing.
    ///
    /// **What this does not pin:** that `PackagePage` names this fallback rather
    /// than `home_loading`. Reading that wiring needs the route mounted with its
    /// settings fetch still pending, and the fetch resolves to `Err` at once in
    /// this harness — the binary's tests also have no `sleep_ms`, since
    /// `test_support` belongs to the library. The wiring is held by the two
    /// functions being named apart and by `home_loading`'s doc, not by a test.
    #[wasm_bindgen_test]
    fn the_package_frame_paints_no_ground() {
        let doc = web_sys::window().unwrap().document().unwrap();
        let host: web_sys::HtmlElement = doc.create_element("div").unwrap().dyn_into().unwrap();
        doc.body().unwrap().append_child(&host).unwrap();
        leptos::mount::mount_to(host.clone(), || loading("Loading package")).forget();

        let el: web_sys::Element = host.into();
        assert!(
            el.query_selector("[role=status]").unwrap().is_some(),
            "it says what it is waiting on"
        );
        assert!(
            el.query_selector("[data-home-frame]").unwrap().is_none(),
            "and paints no ground; markup was {}",
            el.inner_html()
        );
    }

    #[wasm_bindgen_test]
    fn flag_on_renders_v2() {
        let settings = settings_stub(true, false);
        assert!(wants_main_page_v2(Ok(&settings)));
    }

    #[wasm_bindgen_test]
    fn flag_off_renders_v1() {
        let settings = settings_stub(false, false);
        assert!(!wants_main_page_v2(Ok(&settings)));
    }

    #[wasm_bindgen_test]
    fn fetch_error_falls_back_to_v1() {
        // The flag is unknowable, so the answer is the page that has always
        // worked — not a guess at what the reader chose.
        assert!(!wants_main_page_v2(Err("boom")));
    }

    #[wasm_bindgen_test]
    fn package_flag_on_renders_v2() {
        let settings = settings_stub(true, true);
        assert!(wants_package_page_v2(Ok(&settings)));
    }

    #[wasm_bindgen_test]
    fn package_flag_off_renders_v1() {
        let settings = settings_stub(true, false);
        assert!(!wants_package_page_v2(Ok(&settings)));
    }

    #[wasm_bindgen_test]
    fn package_fetch_error_falls_back_to_v1() {
        assert!(!wants_package_page_v2(Err("boom")));
    }

    /// All four rows, because the interesting one is the third: a package flag
    /// left on from before the main page was switched off does not render v2.
    /// The stored value is untouched — Settings still shows it ticked, disabled
    /// — so restoring the main page resumes it.
    #[wasm_bindgen_test]
    fn the_package_page_needs_both_flags() {
        assert!(!wants_package_page_v2(Ok(&settings_stub(false, false))));
        assert!(!wants_package_page_v2(Ok(&settings_stub(true, false))));
        assert!(!wants_package_page_v2(Ok(&settings_stub(false, true))));
        assert!(wants_package_page_v2(Ok(&settings_stub(true, true))));
    }

    /// And the main page is not gated in return — the dependency runs one way.
    #[wasm_bindgen_test]
    fn the_main_page_does_not_read_the_package_flag() {
        assert!(wants_main_page_v2(Ok(&settings_stub(true, false))));
        assert!(!wants_main_page_v2(Ok(&settings_stub(false, true))));
    }
}
