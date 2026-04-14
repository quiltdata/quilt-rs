use leptos::prelude::*;

use crate::commands;
use crate::components::Notification;

use super::IconButton;

#[component]
pub fn PushButton(
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
            match commands::package_push(ns).await {
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

    view! {
        <IconButton icon="/assets/img/icons/cloud_upload.svg" on_click=on_click small=small primary=true disabled=busy>
            <span>{move || if busy.get() { "Pushing\u{2026}" } else { "Push" }}</span>
        </IconButton>
    }
}
