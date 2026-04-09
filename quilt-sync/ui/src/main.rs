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
    } else {
        mount_to_body(pages::Settings);
    }
}
