use leptos::callback::UnsyncCallback;
use leptos::prelude::*;

use super::{ButtonKind, IconButton};

const KIND: ButtonKind = ButtonKind::Save;

#[component]
pub fn Save(
    on_click: impl Fn(leptos::ev::MouseEvent) + 'static,
    #[prop(optional, into)] busy: MaybeProp<bool>,
) -> impl IntoView {
    view! {
        <IconButton icon=KIND.icon() on_click=UnsyncCallback::new(on_click) primary=true large=true disabled=busy>
            {move || if busy.get().unwrap_or(false) { "Saving\u{2026}" } else { KIND.label() }}
        </IconButton>
    }
}
