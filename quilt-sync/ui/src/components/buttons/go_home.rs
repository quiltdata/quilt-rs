use leptos::prelude::*;

use super::IconButton;

#[component]
pub fn GoHome() -> impl IntoView {
    view! {
        <IconButton href="/installed-packages-list".to_string() primary=true>
            "Go home"
        </IconButton>
    }
}
