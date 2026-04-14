use leptos::callback::UnsyncCallback;
use leptos::prelude::*;

use super::IconButton;

#[component]
pub fn CollectLogs(
    on_click: impl Fn(leptos::ev::MouseEvent) + 'static,
    #[prop(optional, into)]
    busy: MaybeProp<bool>,
) -> impl IntoView {
    view! {
        <IconButton on_click=UnsyncCallback::new(on_click) disabled=busy>
            {move || if busy.get().unwrap_or(false) { "Collecting\u{2026}" } else { "Collect Logs" }}
        </IconButton>
    }
}
