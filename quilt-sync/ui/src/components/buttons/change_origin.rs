use leptos::callback::UnsyncCallback;
use leptos::prelude::*;

use super::IconButton;

#[component]
pub fn ChangeOrigin(
    on_click: impl Fn(leptos::ev::MouseEvent) + 'static,
    #[prop(optional)]
    small: bool,
) -> impl IntoView {
    view! {
        <IconButton on_click=UnsyncCallback::new(on_click) small=small>
            "Change origin"
        </IconButton>
    }
}
