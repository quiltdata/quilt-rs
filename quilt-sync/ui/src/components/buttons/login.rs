use leptos::prelude::*;

use super::{ButtonKind, IconLink};

const KIND: ButtonKind = ButtonKind::Login;

#[component]
pub fn Login(href: String, #[prop(optional)] small: bool) -> impl IntoView {
    view! {
        <IconLink href=href icon=KIND.icon() small=small warning=true>
            {KIND.label()}
        </IconLink>
    }
}
