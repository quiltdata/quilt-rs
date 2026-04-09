use leptos::prelude::*;

mod components;
mod pages;
mod tauri;

fn main() {
    console_error_panic_hook::set_once();

    let pathname = web_sys::window()
        .and_then(|w| w.location().pathname().ok())
        .unwrap_or_default();

    if pathname.ends_with("setup.html") {
        mount_to_body(pages::Setup);
    } else if pathname.ends_with("login.html") {
        mount_to_body(pages::Login);
    } else if pathname.ends_with("login-error.html") {
        mount_to_body(pages::Error);
    } else if pathname.ends_with("merge.html") {
        mount_to_body(pages::Merge);
    } else if pathname.ends_with("installed-package.html") {
        mount_to_body(pages::InstalledPackage);
    } else {
        mount_to_body(pages::Settings);
    }
}
