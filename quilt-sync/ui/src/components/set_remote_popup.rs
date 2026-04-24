use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::commands;
use crate::components::buttons;
use crate::components::Notification;
use crate::util::is_valid_hostname;

#[derive(Clone, Debug)]
pub struct SetRemotePopupData {
    pub namespace: String,
    pub current_host: Option<String>,
    pub current_bucket: Option<String>,
}

#[component]
pub fn SetRemotePopup(
    namespace: String,
    current_host: Option<String>,
    current_bucket: Option<String>,
    /// When true, renders the popup as a read-only "Show remote" view:
    /// disabled inputs, no Save button, secondary button becomes "Close".
    /// Used for pushed packages where the remote is pinned to lineage
    /// (see `InstalledPackage::set_remote` in quilt-rs).
    #[prop(optional)]
    locked: bool,
    notification: RwSignal<Option<Notification>>,
    refetch: Trigger,
    on_close: impl Fn() + Clone + 'static,
) -> impl IntoView {
    let origin = RwSignal::new(current_host.unwrap_or_default());
    let bucket = RwSignal::new(current_bucket.unwrap_or_default());
    let host_error = RwSignal::new(false);
    let bucket_error = RwSignal::new(false);
    let submitting = RwSignal::new(false);

    let ns = namespace.clone();
    let on_close_submit = on_close.clone();
    let on_submit = move || {
        if submitting.get_untracked() {
            return;
        }
        let origin_val = origin.get_untracked().trim().to_string();
        let bucket_val = bucket.get_untracked().trim().to_string();

        let mut valid = true;
        if origin_val.is_empty() || !is_valid_hostname(&origin_val) {
            host_error.set(true);
            valid = false;
        }
        if bucket_val.is_empty() {
            bucket_error.set(true);
            valid = false;
        }
        if !valid {
            return;
        }

        submitting.set(true);
        let ns = ns.clone();
        let on_close = on_close_submit.clone();
        leptos::task::spawn_local(async move {
            match commands::set_remote(ns, origin_val, bucket_val).await {
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

    // Enter on host → focus bucket; Enter on bucket → submit
    let on_submit_bucket = on_submit.clone();
    let on_close_key_host = on_close.clone();
    let on_host_keydown = move |ev: leptos::ev::KeyboardEvent| {
        if ev.key() == "Enter" {
            // Focus the bucket input
            if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
                if let Some(el) = doc.get_element_by_id("set-remote-bucket") {
                    if let Ok(input) = el.dyn_into::<web_sys::HtmlElement>() {
                        let _ = input.focus();
                    }
                }
            }
        } else if ev.key() == "Escape" {
            on_close_key_host();
        }
    };

    let on_close_key_bucket = on_close.clone();
    let on_bucket_keydown = move |ev: leptos::ev::KeyboardEvent| {
        if ev.key() == "Enter" {
            on_submit_bucket();
        } else if ev.key() == "Escape" {
            on_close_key_bucket();
        }
    };

    view! {
        <div class="popup-overlay" on:click={
            let on_close = on_close.clone();
            move |_| on_close()
        }>
            <div class="popup-content" on:click=|ev| ev.stop_propagation()>
                <div class="set-remote-form">
                    <h2 class="section-title">
                        {if locked { "Show remote" } else { "Set remote" }}
                    </h2>
                    <div class="set-remote-fields">
                        <div class="set-remote-field">
                            <label>"Host"</label>
                            <div class="set-remote-input-group">
                                <input
                                    class="set-remote-input"
                                    class:error=move || host_error.get()
                                    type="text"
                                    placeholder="open.quiltdata.com"
                                    prop:disabled=locked
                                    prop:value=move || origin.get()
                                    on:input=move |ev| {
                                        origin.set(event_target_value(&ev));
                                        host_error.set(false);
                                    }
                                    on:keydown=on_host_keydown
                                />
                                <span
                                    class="set-remote-hint"
                                    class:visible=move || host_error.get()
                                >
                                    "Enter a valid hostname"
                                </span>
                            </div>
                        </div>

                        <div class="set-remote-field">
                            <label>"Bucket"</label>
                            <div class="set-remote-input-group">
                                <input
                                    id="set-remote-bucket"
                                    class="set-remote-input"
                                    class:error=move || bucket_error.get()
                                    type="text"
                                    placeholder="my-s3-bucket"
                                    prop:disabled=locked
                                    prop:value=move || bucket.get()
                                    on:input=move |ev| {
                                        bucket.set(event_target_value(&ev));
                                        bucket_error.set(false);
                                    }
                                    on:keydown=on_bucket_keydown
                                />
                                <span
                                    class="set-remote-hint"
                                    class:visible=move || bucket_error.get()
                                >
                                    "Enter an S3 bucket name"
                                </span>
                            </div>
                        </div>
                    </div>

                    <div class="set-remote-actions">
                        {(!locked).then(|| view! {
                            <buttons::FormPrimary on_click=on_submit_click disabled=submitting>
                                "Save"
                            </buttons::FormPrimary>
                        })}
                        <buttons::FormSecondary on_click=on_cancel>
                            {if locked { "Close" } else { "Cancel" }}
                        </buttons::FormSecondary>
                    </div>
                </div>
            </div>
        </div>
    }
}
