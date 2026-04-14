use leptos::callback::UnsyncCallback;
use leptos::prelude::*;

use super::IconButton;

#[component]
pub fn EmailSupport(
    on_click: impl Fn(leptos::ev::MouseEvent) + 'static,
    #[prop(optional, into)]
    disabled: MaybeProp<bool>,
) -> impl IntoView {
    view! {
        <IconButton on_click=UnsyncCallback::new(on_click) disabled=disabled>
            "Email Support"
        </IconButton>
    }
}
