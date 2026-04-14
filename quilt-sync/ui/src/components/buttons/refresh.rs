use leptos::callback::UnsyncCallback;
use leptos::prelude::*;

use super::{ButtonKind, IconButton};

const KIND: ButtonKind = ButtonKind::Refresh;

#[component]
pub fn Refresh(
    on_click: impl Fn(leptos::ev::MouseEvent) + 'static,
) -> impl IntoView {
    view! {
        <IconButton icon=KIND.icon() on_click=UnsyncCallback::new(on_click) link=true>
            {KIND.label()}
        </IconButton>
    }
}
