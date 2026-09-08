use leptos::prelude::*;
use leptos_router::components::{Route, Router, Routes};
use leptos_router::path;

#[cfg(test)]
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

mod commands;
mod components;
mod error_handler;
// `kit` ships components ahead of the pages that use them; later tasks in this
// plan wire in the rest, so most are dead here until then.
#[allow(dead_code, unused_imports)]
mod kit;
mod pages;
mod panic_report;
mod tauri;
mod util;

fn main() {
    console_error_panic_hook::set_once();
    // After the console hook, so it chains onto it rather than being replaced by it.
    panic_report::install();
    mount_to_body(App);
}

#[component]
fn App() -> impl IntoView {
    view! {
        <components::UpdateChecker />
        <components::ToastStack />
        <Router>
            <Routes fallback=|| view! { <pages::NotFound /> }>
                <Route path=path!("/") view=|| view! { <Home /> } />
                <Route path=path!("/commit") view=pages::Commit />
                <Route path=path!("/installed-package") view=pages::InstalledPackage />
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
        <Suspense fallback=loading>
            {move || Suspend::new(async move {
                let settings = settings.await;
                if wants_v2(settings.as_ref().map_err(String::as_str)) {
                    view! { <pages::MainPage /> }.into_any()
                } else {
                    view! { <pages::InstalledPackagesList /> }.into_any()
                }
            })}
        </Suspense>
    }
}

/// What `/` shows while it works out which page it is.
///
/// A spinner, which `kit::Spinner`'s own doc reserves for two jobs — this is the
/// second, "filling a region that cannot be skeletonised because its contents are
/// not a list of rows". A skeleton is the right loading state for a list, because
/// it holds the shape the content will take; here not even the page is decided
/// yet, so there is no shape to hold.
///
/// No appbar around it. Drawing one page's chrome and then swapping it for the
/// other's is the flicker this route exists to avoid.
fn loading() -> AnyView {
    view! { <kit::Spinner variant=kit::SpinnerVariant::Region aria_label="Loading QuiltSync" /> }
        .into_any()
}

/// Whether `/` renders v2, given the settings fetch's outcome.
///
/// `Err` — the fetch failed — falls back to v1, same as the flag being off:
/// v1 is the page that has always worked.
fn wants_v2(settings: Result<&commands::SettingsData, &str>) -> bool {
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
        let doc = web_sys::window().unwrap().document().unwrap();
        let host: web_sys::HtmlElement = doc.create_element("div").unwrap().dyn_into().unwrap();
        doc.body().unwrap().append_child(&host).unwrap();
        leptos::mount::mount_to(host.clone(), loading).forget();

        let el: web_sys::Element = host.into();
        let status = el
            .query_selector("[role=status]")
            .unwrap()
            .expect("the loading frame is a live region");
        assert_eq!(
            status.text_content().unwrap().trim(),
            "Loading QuiltSync",
            "and it says what it is waiting on"
        );
    }

    #[wasm_bindgen_test]
    fn flag_on_renders_v2() {
        let settings = settings_stub(true);
        assert!(wants_v2(Ok(&settings)));
    }

    #[wasm_bindgen_test]
    fn flag_off_renders_v1() {
        let settings = settings_stub(false);
        assert!(!wants_v2(Ok(&settings)));
    }

    #[wasm_bindgen_test]
    fn fetch_error_falls_back_to_v1() {
        // The flag is unknowable, so the answer is the page that has always
        // worked — not a guess at what the reader chose.
        assert!(!wants_v2(Err("boom")));
    }
}
