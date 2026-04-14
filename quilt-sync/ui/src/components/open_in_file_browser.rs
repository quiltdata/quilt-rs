use leptos::prelude::*;

use crate::commands;
use crate::components::Notification;

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

    let class = if small { "qui-button small" } else { "qui-button" };

    view! {
        <button class=class type="button" on:click=on_click>
            <img class="qui-icon" src="/assets/img/icons/folder_open.svg" />
            <span>"Open"</span>
        </button>
    }
}
