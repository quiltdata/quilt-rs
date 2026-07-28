use leptos::prelude::*;

use super::{ButtonKind, IconLink};

const KIND: ButtonKind = ButtonKind::SwitchRole;

/// Route to the per-host role switcher in Settings. Shown on a row the
/// active role cannot reach — and only when the user holds another role,
/// so it is never a dead end.
#[component]
pub fn SwitchRole(#[prop(optional)] small: bool) -> impl IntoView {
    view! {
        <IconLink href="/settings".to_string() icon=KIND.icon() small=small warning=true>
            {KIND.label()}
        </IconLink>
    }
}
