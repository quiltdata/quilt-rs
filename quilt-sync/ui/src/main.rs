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
use quilt_sync_ui::routes::UNFINISHED_PACKAGE_PAGE;

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
        <components::AutopullActivity />
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
                let v2 = design_preview(settings.as_ref().map_err(String::as_str));
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

/// `/installed-package` is one package's screen, and [`UNFINISHED_PACKAGE_PAGE`]
/// decides which one.
///
/// One route, the answer read in place — the shape [`Home`] has, for the reason
/// [`Home`] has it: every link to a package (a roster row, a queue remedy, a deep
/// link, a post-commit return) lands on the page that is switched on rather than
/// on whichever one the link was written against. There is no second address for
/// either page.
///
/// No fetch and no loading frame, unlike [`Home`]: the answer is compiled in, so
/// there is nothing to wait for and no frame to fill while waiting.
#[component]
fn PackagePage() -> impl IntoView {
    if package_page_v2() {
        view! { <pages::InstalledPackageV2 /> }.into_any()
    } else {
        view! { <pages::InstalledPackage /> }.into_any()
    }
}

/// What `/` shows while it works out which page it is.
///
/// A spinner, which `kit::Spinner`'s own doc reserves for two jobs — this is the
/// second, "filling a region that cannot be skeletonised because its contents are
/// not a list of rows". A skeleton is the right loading state for a list, because
/// it holds the shape the content will take; here not even the page is decided
/// yet, so there is no shape to hold. No appbar around it either: drawing one
/// page's chrome and then swapping it for the other's is the flicker this route
/// exists to avoid.
///
/// The ground is `/`'s alone. `data-home-frame` is styled off `theme::set_v2`'s
/// root marker (`_base.scss`), which records the reader's design preview — see
/// [`design_preview`] — and a frame may paint from that marker only where it
/// predicts what is coming. That is here and nowhere else: `/` renders v2 on
/// exactly the value the marker is set from, so the ground it paints is the
/// ground the page then keeps.
fn home_loading() -> AnyView {
    view! {
        <div data-home-frame>
            <kit::Spinner variant=kit::SpinnerVariant::Region aria_label="Loading QuiltSync" />
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

/// Whether `/installed-package` renders v2.
///
/// The constant and nothing else. A developer working on the rebuilt screen flips
/// it and ticks *New design preview* the way any reader would — the two answers
/// are asked separately, and neither is inferred from the other.
fn package_page_v2() -> bool {
    UNFINISHED_PACKAGE_PAGE
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

    /// The one real cost of an in-code flag: it can be committed on by accident,
    /// which would ship the unfinished package page to every reader. Flip it
    /// locally to work on that screen; this is what stops it reaching a release.
    #[wasm_bindgen_test]
    fn the_unfinished_package_page_is_off() {
        assert!(
            !package_page_v2(),
            "UNFINISHED_PACKAGE_PAGE is flipped locally and never committed true"
        );
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
