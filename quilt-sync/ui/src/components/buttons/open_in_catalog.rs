use leptos::prelude::*;

use super::{ButtonKind, IconButton};

const KIND: ButtonKind = ButtonKind::OpenInCatalog;

#[component]
pub fn OpenInCatalog(
    on_click: impl Fn(leptos::ev::MouseEvent) + 'static,
    #[prop(optional)]
    small: bool,
    #[prop(optional)]
    disabled: bool,
) -> impl IntoView {
    view! {
        <IconButton icon=KIND.icon() on_click=on_click small=small disabled=disabled>
            {KIND.label()}
        </IconButton>
    }
}
