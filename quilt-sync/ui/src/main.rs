use leptos::prelude::*;
use leptos_router::components::{Redirect, Route, Router, Routes};
use leptos_router::path;

mod commands;
mod components;
mod error_handler;
mod pages;
mod tauri;
mod util;

fn main() {
    console_error_panic_hook::set_once();
    mount_to_body(App);
}

#[component]
fn App() -> impl IntoView {
    view! {
        <components::UpdateChecker />
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
