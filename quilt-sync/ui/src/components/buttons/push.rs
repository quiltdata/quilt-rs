use leptos::prelude::*;

use crate::commands;
use crate::components::Notification;

use super::{ButtonKind, IconButton};

const KIND: ButtonKind = ButtonKind::Push;

#[component]
pub fn Push(
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
        <IconButton icon=KIND.icon() on_click=on_click small=small primary=true disabled=busy>
            {move || if busy.get() { "Pushing\u{2026}" } else { KIND.label() }}
        </IconButton>
    }
}
