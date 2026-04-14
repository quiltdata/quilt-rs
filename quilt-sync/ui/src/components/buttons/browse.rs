use leptos::callback::UnsyncCallback;
use leptos::prelude::*;

use super::IconButton;

#[component]
pub fn Browse(
    on_click: impl Fn(leptos::ev::MouseEvent) + 'static,
    #[prop(optional)]
    small: bool,
    #[prop(optional, into)]
    disabled: MaybeProp<bool>,
) -> impl IntoView {
    view! {
        <IconButton on_click=UnsyncCallback::new(on_click) small=small disabled=disabled>
            "Browse"
        </IconButton>
    }
}
