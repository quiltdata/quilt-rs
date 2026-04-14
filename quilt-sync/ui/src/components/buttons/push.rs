use leptos::prelude::*;

use leptos::callback::UnsyncCallback;

use super::{ButtonKind, IconButton};

const KIND: ButtonKind = ButtonKind::Push;

#[component]
pub fn Push(
    on_click: impl Fn(leptos::ev::MouseEvent) + 'static,
    #[prop(optional)]
    small: bool,
    #[prop(optional, into)]
    busy: MaybeProp<bool>,
) -> impl IntoView {
    view! {
        <IconButton icon=KIND.icon() on_click=UnsyncCallback::new(on_click) small=small primary=true disabled=busy>
            {move || if busy.get().unwrap_or(false) { "Pushing\u{2026}" } else { KIND.label() }}
        </IconButton>
    }
}
