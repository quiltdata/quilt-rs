use leptos::callback::UnsyncCallback;
use leptos::prelude::*;

use super::{ButtonKind, IconButton};

const KIND: ButtonKind = ButtonKind::SubmitLogin;

#[component]
pub fn SubmitLogin(
    on_click: impl Fn(leptos::ev::MouseEvent) + 'static,
    #[prop(optional, into)] busy: MaybeProp<bool>,
    #[prop(optional, into)] disabled: MaybeProp<bool>,
) -> impl IntoView {
    view! {
        <IconButton icon=KIND.icon() on_click=UnsyncCallback::new(on_click) primary=true large=true disabled=disabled>
            {move || if busy.get().unwrap_or(false) { "Logging in\u{2026}" } else { KIND.label() }}
        </IconButton>
    }
}
