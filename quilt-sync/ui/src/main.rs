use leptos::prelude::*;
use leptos_router::components::{Route, Router, Routes};
use leptos_router::path;

#[cfg(test)]
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

// The modules themselves live in the library beside this file — see `src/lib.rs`
// for why. Imported rather than declared, so this binary and the gallery share
// one compilation of them and neither can call an item dead that the other uses.
use quilt_sync_ui::build_profile::BuildProfile;
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
    // Resolved once here rather than inside the gate, so the gate takes a value
    // a test can supply — see `build_profile.rs`.
    let profile = BuildProfile::resolve();

    view! {
        <Suspense fallback=home_loading>
            {move || Suspend::new(async move {
                let settings = settings.await;
                let v2 = effective_design(settings.as_ref().map_err(String::as_str), profile);
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
    let profile = BuildProfile::resolve();

    view! {
        <Suspense fallback=|| loading("Loading package")>
            {move || Suspend::new(async move {
                let settings = settings.await;
                if package_page_v2(settings.as_ref().map_err(String::as_str), profile) {
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
/// and that marker records the *effective design generation* — see
/// [`effective_design`]. It may only be drawn where it predicts what is coming,
/// which is here and nowhere else: `/` renders v2 on exactly the value the marker
/// is set from, so the ground it paints is the ground the page then keeps.
///
/// `/installed-package` gets [`loading`] bare for that reason. The generation says
/// nothing about which package page is coming — [`package_page_v2`] is the
/// narrower question — so a frame painted from the marker is wrong whenever the
/// two disagree, and a reader in the preview with the unfinished page off would
/// have been shown a dark ground before v1's light chrome, having opted into
/// nothing.
fn home_loading() -> AnyView {
    view! {
        <div data-home-frame>{loading("Loading QuiltSync")}</div>
    }
    .into_any()
}

/// Whether the redesigned application generation is effective, given the settings
/// fetch's outcome and the build.
///
/// `Err` — the fetch failed — falls back to v1, same as the preference being off:
/// v1 is the generation that has always worked.
///
/// The construction gate is an OR, not an AND: a developer turning on the
/// unfinished page gets the application generation it is built inside, without
/// the stored reader answer being rewritten. Turning the gate off resumes that
/// answer.
fn effective_design(
    settings: Result<&commands::SettingsData, &str>,
    profile: BuildProfile,
) -> bool {
    settings.is_ok_and(|data| data.experimental.main_page_v2) || package_page_v2(settings, profile)
}

/// Whether `/installed-package` renders v2.
///
/// The build is `ANDed` here and not only in Settings: a disabled row over a live
/// flag is a flag with no way to turn it off, and a release build must not honour
/// a value a development build left behind.
fn package_page_v2(settings: Result<&commands::SettingsData, &str>, profile: BuildProfile) -> bool {
    profile.allows_construction_gates()
        && settings.is_ok_and(|data| data.experimental.package_page_v2)
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
        assert!(effective_design(Ok(&settings), BuildProfile::Development));
    }

    #[wasm_bindgen_test]
    fn flag_off_renders_v1() {
        let settings = settings_stub(false, false);
        assert!(!effective_design(Ok(&settings), BuildProfile::Development));
        assert!(!package_page_v2(Ok(&settings), BuildProfile::Development));
    }

    #[wasm_bindgen_test]
    fn package_flag_on_renders_v2() {
        let settings = settings_stub(true, true);
        assert!(package_page_v2(Ok(&settings), BuildProfile::Development));
        assert!(effective_design(Ok(&settings), BuildProfile::Development));
    }

    #[wasm_bindgen_test]
    fn package_flag_off_renders_v1() {
        let settings = settings_stub(true, false);
        assert!(!package_page_v2(Ok(&settings), BuildProfile::Development));
    }

    /// The gate implies the generation rather than depending on it — the
    /// inversion this change turns on. A developer opens the unfinished page
    /// without first opting into the preview, and gets the design it is drawn in.
    #[wasm_bindgen_test]
    fn the_construction_gate_turns_the_design_on_without_the_stored_answer() {
        let settings = settings_stub(false, true);
        assert!(effective_design(Ok(&settings), BuildProfile::Development));
        assert!(package_page_v2(Ok(&settings), BuildProfile::Development));
    }

    /// And leaves it alone: the stored answer is still off, so turning the gate
    /// off returns the reader to v1 rather than to whatever the gate implied.
    #[wasm_bindgen_test]
    fn the_construction_gate_does_not_rewrite_the_stored_answer() {
        let settings = settings_stub(false, false);
        assert!(!effective_design(Ok(&settings), BuildProfile::Development));
    }

    /// A value a development build persisted is inert in a release build — both
    /// for the page and for the generation. This is the claim Settings alone
    /// cannot make.
    #[wasm_bindgen_test]
    fn a_release_build_ignores_a_stored_construction_value() {
        let settings = settings_stub(false, true);
        assert!(!package_page_v2(Ok(&settings), BuildProfile::Release));
        assert!(!effective_design(Ok(&settings), BuildProfile::Release));
    }

    /// The reader's own preference is untouched by the build: a release reader
    /// who opted in still gets the preview.
    #[wasm_bindgen_test]
    fn a_release_build_still_honours_the_reader_preference() {
        let settings = settings_stub(true, true);
        assert!(effective_design(Ok(&settings), BuildProfile::Release));
        assert!(!package_page_v2(Ok(&settings), BuildProfile::Release));
    }

    /// A failed read is v1, unchanged from `wants_main_page_v2`'s rule.
    #[wasm_bindgen_test]
    fn a_failed_settings_read_is_v1() {
        assert!(!effective_design(Err("nope"), BuildProfile::Development));
        assert!(!package_page_v2(Err("nope"), BuildProfile::Development));
    }
}
