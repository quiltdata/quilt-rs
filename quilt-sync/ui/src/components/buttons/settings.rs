use leptos::prelude::*;

use super::{ButtonKind, IconLink};

const KIND: ButtonKind = ButtonKind::Settings;

#[component]
pub fn Settings() -> impl IntoView {
    view! {
        <IconLink icon=KIND.icon() href="/settings".to_string() link=true>
            {KIND.label()}
        </IconLink>
    }
}
