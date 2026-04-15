use leptos::prelude::*;

use super::IconLink;

#[component]
pub fn ReLogin(href: String) -> impl IntoView {
    view! {
        <IconLink href=href small=true>
            "Re-login"
        </IconLink>
    }
}
