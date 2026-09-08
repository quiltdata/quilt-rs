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
        <Suspense fallback=|| view! { <div></div> }>
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
