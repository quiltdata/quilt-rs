use leptos::prelude::*;

use crate::commands;
use crate::components::Notification;

#[component]
pub fn PullButton(
    namespace: String,
    notification: RwSignal<Option<Notification>>,
    ui_locked: RwSignal<bool>,
    refetch: Trigger,
    #[prop(optional)]
    small: bool,
) -> impl IntoView {
    let busy = RwSignal::new(false);

    let on_click = move |_| {
        if busy.get_untracked() {
            return;
        }
        busy.set(true);
        ui_locked.set(true);
        let ns = namespace.clone();
        leptos::task::spawn_local(async move {
            match commands::package_pull(ns).await {
                Ok(msg) => {
                    ui_locked.set(false);
                    notification.set(Some(Notification::Success(msg)));
                    refetch.notify();
                }
                Err(e) => {
                    ui_locked.set(false);
                    notification.set(Some(Notification::Error(e)));
                    busy.set(false);
                }
            }
        });
    };

    let class = if small {
        "qui-button primary small"
    } else {
        "qui-button primary"
    };

    view! {
        <button class=class type="button" prop:disabled=move || busy.get() on:click=on_click>
            <img class="qui-icon" src="/assets/img/icons/cloud_download.svg" />
            <span>{move || if busy.get() { "Pulling\u{2026}" } else { "Pull" }}</span>
        </button>
    }
}
