use leptos::prelude::*;

use leptos::callback::UnsyncCallback;

use super::{ButtonKind, IconButton};

const KIND: ButtonKind = ButtonKind::SetRemote;

#[component]
pub fn SetRemote(
    on_click: impl Fn(leptos::ev::MouseEvent) + 'static,
    #[prop(optional)] small: bool,
    #[prop(optional)] warning: bool,
    #[prop(optional)] label: Option<&'static str>,
) -> impl IntoView {
    let label = label.unwrap_or(KIND.label());
    view! {
        <IconButton
            icon=KIND.icon()
            on_click=UnsyncCallback::new(on_click)
            small=small
            warning=warning
        >
            {label}
        </IconButton>
    }
}
