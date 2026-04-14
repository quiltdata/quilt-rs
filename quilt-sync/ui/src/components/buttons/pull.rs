use leptos::prelude::*;

use super::{ButtonKind, IconButton};

const KIND: ButtonKind = ButtonKind::Pull;

#[component]
pub fn Pull(
    on_click: impl Fn(leptos::ev::MouseEvent) + 'static,
    #[prop(optional)]
    small: bool,
    #[prop(optional, into)]
    busy: MaybeProp<bool>,
) -> impl IntoView {
    view! {
        <IconButton icon=KIND.icon() on_click=on_click small=small primary=true disabled=busy>
            {move || if busy.get().unwrap_or(false) { "Pulling\u{2026}" } else { KIND.label() }}
        </IconButton>
    }
}
