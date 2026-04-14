use leptos::prelude::*;

use super::{ButtonKind, IconButton};

const KIND: ButtonKind = ButtonKind::Login;

#[component]
pub fn Login(href: String, #[prop(optional)] small: bool) -> impl IntoView {
    view! {
        <IconButton href=href icon=KIND.icon() small=small warning=true>
            {KIND.label()}
        </IconButton>
    }
}
