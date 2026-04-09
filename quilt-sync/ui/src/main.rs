use leptos::prelude::*;

mod components;
mod pages;
mod tauri;

fn main() {
    console_error_panic_hook::set_once();
    mount_to_body(pages::Settings);
}
