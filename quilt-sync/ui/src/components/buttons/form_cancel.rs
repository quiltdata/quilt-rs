use leptos::callback::UnsyncCallback;
use leptos::prelude::*;

use super::IconButton;

#[component]
pub fn FormCancel(
    on_click: impl Fn(leptos::ev::MouseEvent) + 'static,
) -> impl IntoView {
    view! {
        <IconButton on_click=UnsyncCallback::new(on_click)>
            "Cancel"
        </IconButton>
    }
}
