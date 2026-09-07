//! The toast stack: server-posted notifications, newest last, each dismissable.
//!
//! Mounted once in `App`, **outside** the router, so it survives navigation —
//! a report about a background pull must not vanish because the user changed
//! page. It is deliberately separate from [`Layout`](super::layout::Layout)'s
//! single notification slot, which stays exactly as it was: that slot belongs
//! to the screen that raised it, this stack to the backend.
//!
//! The backend is the source of truth. Two reads keep it so: [`get_toasts`]
//! on mount, which catches everything posted while no window was open, and the
//! [`TOAST_EVENT`] listener for everything after. Dismissing tells the backend
//! first-class — it holds the list, so a toast closed here must not come back
//! on the next mount.

use std::collections::BTreeMap;
use std::time::Duration;

use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::commands;
use crate::commands::Toast;
use crate::commands::ToastKind;
use crate::tauri as tauri_bridge;

/// The visible toasts, keyed by id so a hydration read and a live event cannot
/// double-render the same toast — the backend retains before it emits, so that
/// overlap is expected rather than exceptional.
type Toasts = BTreeMap<u64, Toast>;

#[component]
pub fn ToastStack() -> impl IntoView {
    let toasts: RwSignal<Toasts> = RwSignal::new(BTreeMap::new());

    // Hydrate: whatever was posted while nothing was listening.
    spawn_local(async move {
        if let Ok(existing) = commands::get_toasts().await {
            toasts.update(|map| {
                for toast in existing {
                    map.insert(toast.id, toast);
                }
            });
        }
    });

    let listener = tauri_bridge::listen::<Toast>(commands::TOAST_EVENT, move |toast| {
        toasts.update(|map| {
            map.insert(toast.id, toast);
        });
    });
    on_cleanup(move || drop(listener));

    // Hoisted out of `view!`: the macro's tag tokenizer reads a turbofish
    // (`::<Vec<_>>`) as markup, so the collect stays outside it.
    let ordered = move || -> Vec<Toast> { toasts.get().into_values().collect() };

    view! {
        <div class="qui-toasts">
            <For each=ordered key=|toast| toast.id let:toast>
                <ToastCard toast=toast toasts=toasts />
            </For>
        </div>
    }
}

#[component]
fn ToastCard(toast: Toast, toasts: RwSignal<Toasts>) -> impl IntoView {
    let id = toast.id;

    // One dismissal path for the button and the timer alike, so an
    // auto-dismissed toast is as gone from the backend as a clicked one — the
    // backend holds the list, and a toast that expired on screen but not there
    // would return on the next mount.
    let dismiss = move || {
        toasts.update(|map| {
            map.remove(&id);
        });
        spawn_local(async move {
            let _ = commands::dismiss_toast(id).await;
        });
    };

    // The optional timer. `None` stands until the user acts, which is the
    // right default for a report about something that happened while they were
    // away: a countdown would be racing their absence.
    let timer = StoredValue::new(None::<TimeoutHandle>);
    if let Some(ms) = toast.timeout_ms
        && let Ok(handle) = set_timeout_with_handle(dismiss, Duration::from_millis(ms.into()))
    {
        timer.set_value(Some(handle));
    }
    on_cleanup(move || {
        if let Some(Some(handle)) = timer.try_get_value() {
            handle.clear();
        }
    });

    let kind_class = match toast.kind {
        ToastKind::Info => "qui-toast info",
        ToastKind::Success => "qui-toast success",
        ToastKind::Warning => "qui-toast warning",
        ToastKind::Error => "qui-toast error",
    };
    let title = toast.title.clone();
    let body = toast.body.clone();

    view! {
        <div class=kind_class role="status">
            <div class="content">
                {title.map(|t| view! { <span class="title">{t}</span> })}
                <span class="body">{body}</span>
            </div>
            <button
                class="close"
                type="button"
                aria-label="Dismiss"
                on:click=move |_| {
                    if let Some(Some(handle)) = timer.try_get_value() {
                        handle.clear();
                    }
                    dismiss();
                }
            >
                "×"
            </button>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::*;

    fn mount<N: IntoView + 'static>(f: impl FnOnce() -> N + 'static) -> web_sys::Element {
        let doc = web_sys::window().unwrap().document().unwrap();
        let container: web_sys::HtmlElement =
            doc.create_element("div").unwrap().dyn_into().unwrap();
        doc.body().unwrap().append_child(&container).unwrap();
        leptos::mount::mount_to(container.clone(), f).forget();
        container.into()
    }

    fn toast(kind: ToastKind, title: Option<&str>, body: &str) -> Toast {
        Toast {
            id: 7,
            kind,
            title: title.map(str::to_owned),
            body: body.to_owned(),
            timeout_ms: None,
        }
    }

    #[wasm_bindgen_test]
    fn kind_reaches_the_class_so_the_border_can_differ() {
        for (kind, expected) in [
            (ToastKind::Info, "qui-toast info"),
            (ToastKind::Success, "qui-toast success"),
            (ToastKind::Warning, "qui-toast warning"),
            (ToastKind::Error, "qui-toast error"),
        ] {
            let t = toast(kind, None, "body");
            let signal = RwSignal::new(BTreeMap::new());
            let el = mount(move || view! { <ToastCard toast=t toasts=signal /> });
            let card = el.query_selector(".qui-toast").unwrap().unwrap();
            assert_eq!(card.get_attribute("class").unwrap(), expected);
        }
    }

    #[wasm_bindgen_test]
    fn a_title_renders_beside_the_body_and_is_optional() {
        let signal = RwSignal::new(BTreeMap::new());
        let with = toast(ToastKind::Info, Some("acme/rna-seq"), "2 new files");
        let el = mount(move || view! { <ToastCard toast=with toasts=signal /> });
        assert!(el.query_selector(".title").unwrap().is_some());
        let text = el.text_content().unwrap();
        assert!(text.contains("acme/rna-seq"), "title missing: {text}");
        assert!(text.contains("2 new files"), "body missing: {text}");

        let signal = RwSignal::new(BTreeMap::new());
        let without = toast(ToastKind::Info, None, "body only");
        let el = mount(move || view! { <ToastCard toast=without toasts=signal /> });
        assert!(el.query_selector(".title").unwrap().is_none());
        assert!(el.text_content().unwrap().contains("body only"));
    }

    #[wasm_bindgen_test]
    fn every_toast_carries_a_labelled_close_control() {
        let signal = RwSignal::new(BTreeMap::new());
        let t = toast(ToastKind::Error, None, "went wrong");
        let el = mount(move || view! { <ToastCard toast=t toasts=signal /> });
        let close = el.query_selector("button.close").unwrap().unwrap();
        assert_eq!(close.get_attribute("aria-label").unwrap(), "Dismiss");
    }
}
