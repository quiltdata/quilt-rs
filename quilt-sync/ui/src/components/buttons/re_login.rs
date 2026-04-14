use leptos::prelude::*;

use super::IconButton;

#[component]
pub fn ReLogin(href: String) -> impl IntoView {
    view! {
        <IconButton href=href small=true>
            "Re-login"
        </IconButton>
    }
}
