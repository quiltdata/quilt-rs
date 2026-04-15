use leptos::prelude::*;

use crate::commands;
use crate::components::buttons;
use crate::components::Notification;
use crate::util::is_valid_hostname;

#[derive(Clone, Debug)]
pub struct SetOriginPopupData {
    pub namespace: String,
    pub current_origin: String,
}

#[component]
pub fn SetOriginPopup(
    namespace: String,
    current_origin: String,
    notification: RwSignal<Option<Notification>>,
    refetch: Trigger,
    on_close: impl Fn() + Clone + 'static,
) -> impl IntoView {
    let origin = RwSignal::new(current_origin);
    let show_error = RwSignal::new(false);
    let submitting = RwSignal::new(false);

    let ns = namespace.clone();

    let on_close_submit = on_close.clone();
    let on_submit = move || {
        let value = origin.get_untracked().trim().to_string();
        if value.is_empty() || submitting.get_untracked() {
            return;
        }
        if !is_valid_hostname(&value) {
            show_error.set(true);
            return;
        }
        submitting.set(true);
        let ns = ns.clone();
        let on_close = on_close_submit.clone();
        leptos::task::spawn_local(async move {
            match commands::set_origin(ns, value).await {
                Ok(msg) => {
                    notification.set(Some(Notification::Success(msg)));
                    on_close();
                    refetch.notify();
                }
                Err(e) => {
                    notification.set(Some(Notification::Error(e)));
                    submitting.set(false);
                }
            }
        });
    };

    let on_submit_click = {
        let on_submit = on_submit.clone();
        move |_| on_submit()
    };

    let on_close_cancel = on_close.clone();
    let on_cancel = move |_: leptos::ev::MouseEvent| on_close_cancel();

    let on_submit_key = on_submit.clone();
    let on_close_key = on_close.clone();
    let on_keydown = move |ev: leptos::ev::KeyboardEvent| {
        if ev.key() == "Enter" {
            on_submit_key();
        } else if ev.key() == "Escape" {
            on_close_key();
        }
    };

    view! {
        <div class="popup-overlay" on:click={
            let on_close = on_close.clone();
            move |_| on_close()
        }>
            <div class="popup-content" on:click=|ev| ev.stop_propagation()>
                <div class="origin-form">
                    <label>"Catalog origin"</label>
                    <div class="origin-input-group">
                        <input
                            class="origin-input"
                            class:error=move || show_error.get()
                            type="text"
                            placeholder="open.quilt.bio"
                            prop:value=move || origin.get()
                            on:input=move |ev| {
                                origin.set(event_target_value(&ev));
                                show_error.set(false);
                            }
                            on:keydown=on_keydown
                        />
                        <span
                            class="origin-hint"
                            class:visible=move || show_error.get()
                        >
                            "Enter a valid hostname, e.g. open.quilt.bio"
                        </span>
                    </div>
                    <div class="origin-form-actions">
                        <buttons::FormPrimary on_click=on_submit_click disabled=submitting>
                            "Submit"
                        </buttons::FormPrimary>
                        <buttons::FormSecondary on_click=on_cancel />
                    </div>
                </div>
            </div>
        </div>
    }
}
