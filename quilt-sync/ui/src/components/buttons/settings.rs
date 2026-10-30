use leptos::prelude::*;

use super::{ButtonKind, IconLink};

const KIND: ButtonKind = ButtonKind::Settings;

#[component]
pub fn Settings(#[prop(optional)] replace: bool) -> impl IntoView {
    view! {
        <IconLink icon=KIND.icon() href="/settings".to_string() link=true replace=replace>
            {KIND.label()}
        </IconLink>
    }
}
