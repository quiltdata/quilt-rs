use leptos::prelude::*;

use super::IconButton;

#[component]
pub fn LoginLink(
    href: String,
) -> impl IntoView {
    view! {
        <IconButton href=href>
            "Login"
        </IconButton>
    }
}
