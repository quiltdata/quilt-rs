use leptos::prelude::*;

use leptos::callback::UnsyncCallback;

use super::{ButtonKind, IconButton};

const KIND: ButtonKind = ButtonKind::Publish;

#[component]
pub fn Publish(
    on_click: impl Fn(leptos::ev::MouseEvent) + 'static,
    #[prop(optional)] small: bool,
    #[prop(optional, into)] busy: MaybeProp<bool>,
    #[prop(optional, into)] disabled: MaybeProp<bool>,
) -> impl IntoView {
    let is_disabled =
        Signal::derive(move || busy.get().unwrap_or(false) || disabled.get().unwrap_or(false));
    view! {
        <IconButton icon=KIND.icon() on_click=UnsyncCallback::new(on_click) small=small primary=true disabled=is_disabled>
            {move || if busy.get().unwrap_or(false) { "Committing and pushing\u{2026}" } else { KIND.label() }}
        </IconButton>
    }
}
