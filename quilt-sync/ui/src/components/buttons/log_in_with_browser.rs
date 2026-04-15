use leptos::callback::UnsyncCallback;
use leptos::prelude::*;

use super::{ButtonKind, IconButton};

const KIND: ButtonKind = ButtonKind::LogInWithBrowser;

#[component]
pub fn LogInWithBrowser(
    on_click: impl Fn(leptos::ev::MouseEvent) + 'static,
    #[prop(optional, into)] disabled: MaybeProp<bool>,
) -> impl IntoView {
    view! {
        <IconButton icon=KIND.icon() on_click=UnsyncCallback::new(on_click) primary=true large=true disabled=disabled>
            {KIND.label()}
        </IconButton>
    }
}
