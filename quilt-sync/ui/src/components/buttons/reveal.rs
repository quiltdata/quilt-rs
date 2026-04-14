use leptos::callback::UnsyncCallback;
use leptos::prelude::*;

use super::{ButtonKind, IconButton};

const KIND: ButtonKind = ButtonKind::Reveal;

#[component]
pub fn Reveal(
    on_click: impl Fn(leptos::ev::MouseEvent) + 'static,
    #[prop(optional)] small: bool,
    #[prop(optional)] link: bool,
) -> impl IntoView {
    view! {
        <IconButton icon=KIND.icon() on_click=UnsyncCallback::new(on_click) small=small link=link>
            {KIND.label()}
        </IconButton>
    }
}
