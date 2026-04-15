use leptos::prelude::*;

use super::IconLink;

#[component]
pub fn LoginLink(href: String) -> impl IntoView {
    view! {
        <IconLink href=href>
            "Login"
        </IconLink>
    }
}
