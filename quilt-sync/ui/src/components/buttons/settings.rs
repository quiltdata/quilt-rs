use leptos::prelude::*;

use super::{ButtonKind, IconButton};

const KIND: ButtonKind = ButtonKind::Settings;

#[component]
pub fn Settings() -> impl IntoView {
    view! {
        <IconButton icon=KIND.icon() href="/settings".to_string() link=true>
            {KIND.label()}
        </IconButton>
    }
}
