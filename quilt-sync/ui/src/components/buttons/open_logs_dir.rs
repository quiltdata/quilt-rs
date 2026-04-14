use leptos::callback::UnsyncCallback;
use leptos::prelude::*;

use super::{ButtonKind, IconButton};

const KIND: ButtonKind = ButtonKind::OpenInFileBrowser;

#[component]
pub fn OpenLogsDir(
    on_click: impl Fn(leptos::ev::MouseEvent) + 'static,
    #[prop(optional)]
    is_temporary: bool,
) -> impl IntoView {
    let icon = if is_temporary {
        ButtonKind::Login.icon() // warning.svg
    } else {
        KIND.icon() // folder_open.svg
    };

    view! {
        <IconButton icon=icon on_click=UnsyncCallback::new(on_click) small=true link=true>
            {KIND.label()}
        </IconButton>
    }
}
