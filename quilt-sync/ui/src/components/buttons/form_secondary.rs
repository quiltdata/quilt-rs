use leptos::callback::UnsyncCallback;
use leptos::prelude::*;

use super::IconButton;

#[component]
pub fn FormSecondary(
    on_click: impl Fn(leptos::ev::MouseEvent) + 'static,
    #[prop(optional)]
    children: Option<Children>,
) -> impl IntoView {
    view! {
        <IconButton on_click=UnsyncCallback::new(on_click)>
            {match children {
                Some(c) => c().into_any(),
                None => "Cancel".into_any(),
            }}
        </IconButton>
    }
}
