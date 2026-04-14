use leptos::callback::UnsyncCallback;
use leptos::prelude::*;

use super::IconButton;

#[component]
pub fn FormPrimary(
    on_click: impl Fn(leptos::ev::MouseEvent) + 'static,
    #[prop(optional, into)] disabled: MaybeProp<bool>,
    children: Children,
) -> impl IntoView {
    view! {
        <IconButton on_click=UnsyncCallback::new(on_click) primary=true disabled=disabled>
            {children()}
        </IconButton>
    }
}
