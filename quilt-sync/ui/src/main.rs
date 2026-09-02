use leptos::prelude::*;
use leptos_router::components::{Redirect, Route, Router, Routes};
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
                <Route path=path!("/") view=|| view! { <Landing /> } />
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

/// Sends `/` to whichever main page is switched on.
///
/// A fetch, so there is one frame with nothing on it. That is deliberate over
/// guessing: landing on v1 and then jumping to v2 is worse than a blank frame,
/// and any failure lands on v1, which is the page that has always worked.
#[component]
fn Landing() -> impl IntoView {
    let settings = LocalResource::new(|| async move { commands::get_settings_data().await });

    view! {
        <Suspense fallback=|| view! { <div></div> }>
            {move || Suspend::new(async move {
                let settings = settings.await;
                let target = landing_target(settings.as_ref().map_err(String::as_str));
                view! { <Redirect path=target /> }
            })}
        </Suspense>
    }
}

/// Which page `/` sends the user to, given the settings fetch's outcome.
///
/// `Err` — the fetch failed — falls back to v1, same as the flag being off:
/// v1 is the page that has always worked.
fn landing_target(settings: Result<&commands::SettingsData, &str>) -> &'static str {
    let to_v2 = settings.is_ok_and(|data| data.experimental.main_page_v2);
    if to_v2 {
        "/main"
    } else {
        "/installed-packages-list"
    }
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
    fn flag_on_goes_to_v2() {
        let settings = settings_stub(true);
        assert_eq!(landing_target(Ok(&settings)), "/main");
    }

    #[wasm_bindgen_test]
    fn flag_off_goes_to_v1() {
        let settings = settings_stub(false);
        assert_eq!(landing_target(Ok(&settings)), "/installed-packages-list");
    }

    #[wasm_bindgen_test]
    fn fetch_error_falls_back_to_v1() {
        assert_eq!(landing_target(Err("boom")), "/installed-packages-list");
    }
}
