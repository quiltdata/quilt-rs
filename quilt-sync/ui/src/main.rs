use leptos::prelude::*;
use leptos_router::components::{Redirect, Route, Router, Routes};
use leptos_router::path;

mod commands;
mod components;
mod error_handler;
mod pages;
mod tauri;

fn main() {
    console_error_panic_hook::set_once();

    let pathname = web_sys::window()
        .and_then(|w| w.location().pathname().ok())
        .unwrap_or_default();

    // If loaded from an old .html page shell, redirect to clean URL
    // before mounting the router (avoids fallback/navigate complexity).
    if pathname.contains(".html") {
        if let Some(clean_url) = legacy_to_clean_url(&pathname) {
            if let Some(window) = web_sys::window() {
                let _ = window.location().replace(&clean_url);
            }
            return;
        }
    }

    mount_to_body(App);
}

/// Convert old `.html#fragment` URL to clean `/path?query` URL.
/// Returns None if the pathname doesn't match any known page.
fn legacy_to_clean_url(pathname: &str) -> Option<String> {
    let page = pathname
        .rsplit('/')
        .next()?
        .trim_end_matches(".html");

    let clean_page = match page {
        "commit" => "/commit",
        "installed-package" => "/installed-package",
        "installed-packages-list" => "/installed-packages-list",
        "login" => "/login",
        "login-error" => "/error",
        "merge" => "/merge",
        "remote-package" => "/remote-package",
        "settings" => "/settings",
        "setup" => "/setup",
        _ => return None,
    };

    // Read hash fragment (old-style params) and query string
    let hash = web_sys::window()
        .and_then(|w| w.location().hash().ok())
        .unwrap_or_default();
    let fragment = hash.trim_start_matches('#');

    let search = web_sys::window()
        .and_then(|w| w.location().search().ok())
        .unwrap_or_default();
    let query = search.trim_start_matches('?');

    let params = if !fragment.is_empty() {
        fragment
    } else if !query.is_empty() {
        query
    } else {
        ""
    };

    if params.is_empty() {
        Some(clean_page.to_string())
    } else {
        Some(format!("{clean_page}?{params}"))
    }
}

#[component]
fn App() -> impl IntoView {
    view! {
        <Router>
            <Routes fallback=|| view! { <pages::Error /> }>
                <Route path=path!("/") view=|| view! {
                    <Redirect path="/installed-packages-list" />
                } />
                <Route path=path!("/commit") view=pages::Commit />
                <Route path=path!("/installed-package") view=pages::InstalledPackage />
                <Route path=path!("/installed-packages-list") view=pages::InstalledPackagesList />
                <Route path=path!("/login") view=pages::Login />
                <Route path=path!("/error") view=pages::Error />
                <Route path=path!("/merge") view=pages::Merge />
                <Route path=path!("/remote-package") view=pages::RemotePackage />
                <Route path=path!("/settings") view=pages::Settings />
                <Route path=path!("/setup") view=pages::Setup />
            </Routes>
        </Router>
    }
}
