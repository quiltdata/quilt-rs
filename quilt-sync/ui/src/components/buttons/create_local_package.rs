use leptos::prelude::*;

use leptos::callback::UnsyncCallback;

use super::{ButtonKind, IconButton};

const KIND: ButtonKind = ButtonKind::CreateLocalPackage;

#[component]
pub fn CreateLocalPackage(
    on_click: impl Fn(leptos::ev::MouseEvent) + 'static,
    #[prop(optional)] small: bool,
) -> impl IntoView {
    view! {
        <IconButton icon=KIND.icon() on_click=UnsyncCallback::new(on_click) small=small>
            {KIND.label()}
        </IconButton>
    }
}
