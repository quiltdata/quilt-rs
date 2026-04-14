use leptos::prelude::*;

use crate::commands;
use crate::components::Notification;

use super::{ButtonKind, IconButton};

const KIND: ButtonKind = ButtonKind::OpenInFileBrowser;

#[component]
pub fn OpenInFileBrowser(
    namespace: String,
    notification: RwSignal<Option<Notification>>,
    #[prop(optional)]
    small: bool,
) -> impl IntoView {
    let on_click = move |_| {
        let ns = namespace.clone();
        leptos::task::spawn_local(async move {
            match commands::open_in_file_browser(ns).await {
                Ok(msg) => notification.set(Some(Notification::Success(msg))),
                Err(e) => notification.set(Some(Notification::Error(e))),
            }
        });
    };

    view! {
        <IconButton icon=KIND.icon() on_click=on_click small=small>
            {KIND.label()}
        </IconButton>
    }
}
