use leptos::callback::UnsyncCallback;
use leptos::prelude::*;

use super::IconButton;

#[component]
pub fn SendToSentry(
    on_click: impl Fn(leptos::ev::MouseEvent) + 'static,
    #[prop(optional, into)] disabled: MaybeProp<bool>,
) -> impl IntoView {
    view! {
        <IconButton on_click=UnsyncCallback::new(on_click) disabled=disabled>
            "Send to Sentry"
        </IconButton>
    }
}
