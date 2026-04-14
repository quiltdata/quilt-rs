use leptos::prelude::*;

use super::{ButtonKind, IconButton};

const KIND: ButtonKind = ButtonKind::SetRemote;

#[component]
pub fn SetRemote(
    on_click: impl Fn(leptos::ev::MouseEvent) + 'static,
    #[prop(optional)]
    small: bool,
) -> impl IntoView {
    view! {
        <IconButton icon=KIND.icon() on_click=on_click small=small>
            {KIND.label()}
        </IconButton>
    }
}
