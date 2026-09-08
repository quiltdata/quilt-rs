use leptos::prelude::*;

use super::IconLink;

#[component]
pub fn GoHome() -> impl IntoView {
    view! {
        <IconLink href="/".to_string() primary=true>
            "Go home"
        </IconLink>
    }
}
