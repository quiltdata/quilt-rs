//! The toast stack: server-posted notifications, newest first, each dismissable.
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

/// Replace the visible set with the backend's, which is authoritative.
///
/// **Replace, not merge.** The backend evicts its oldest past a capacity bound
/// and emits only the toast that arrived, so a client that merely inserted
/// would keep showing toasts the backend has dropped and grow past the same
/// bound. Replacing mirrors evictions for free and needs no second copy of the
/// capacity constant to drift.
fn apply_snapshot(toasts: RwSignal<Toasts>, snapshot: Vec<Toast>) {
    let next: Toasts = snapshot.into_iter().map(|t| (t.id, t)).collect();
    toasts.set(next);
}

/// Read the backend's list and adopt it.
async fn reconcile(toasts: RwSignal<Toasts>) {
    if let Ok(snapshot) = commands::get_toasts().await {
        apply_snapshot(toasts, snapshot);
    }
}

#[component]
pub fn ToastStack() -> impl IntoView {
    let toasts: RwSignal<Toasts> = RwSignal::new(BTreeMap::new());

    // Whatever was posted while nothing was listening.
    spawn_local(reconcile(toasts));

    let listener = tauri_bridge::listen::<Toast>(commands::TOAST_EVENT, move |toast| {
        // Insert first so the toast appears without waiting for a round trip,
        // then reconcile, because the payload alone cannot say what the
        // backend evicted to make room for it.
        toasts.update(|map| {
            map.insert(toast.id, toast);
        });
        spawn_local(reconcile(toasts));
    });

    // A second read, sequenced after the listener exists. This **narrows** the
    // startup race rather than closing it: registration completes on a promise
    // the bridge does not expose, so a toast posted between the first read and
    // the listener going live is still deliverable only by the reconcile above,
    // which the next toast triggers. Closing it properly needs the bridge to
    // hand back a registration future.
    spawn_local(reconcile(toasts));
    on_cleanup(move || drop(listener));

    view! { <ToastLayer toasts=toasts /> }
}

/// The markup, split from the bridge wiring above so it can be mounted in a
/// test — `ToastStack` cannot, since it reaches for Tauri on mount.
#[component]
fn ToastLayer(toasts: RwSignal<Toasts>) -> impl IntoView {
    // Newest first: the stack hangs from the top of the window, so the newest
    // belongs nearest the eye rather than pushed furthest from it.
    //
    // Hoisted out of `view!`: the macro's tag tokenizer reads a turbofish
    // (`::<Vec<_>>`) as markup, so the collect stays outside it.
    let ordered = move || -> Vec<Toast> { toasts.get().into_values().rev().collect() };
    let many = move || toasts.with(|map| map.len() > 1);
    // Nothing at all when empty. The layer captures pointer events, so an
    // always-rendered container would leave an invisible band across the page
    // blocking the rows beneath it.
    let any = move || !toasts.with(BTreeMap::is_empty);

    let dismiss_all = move |_| {
        let ids: Vec<u64> = toasts.with_untracked(|map| map.keys().copied().collect());
        toasts.update(BTreeMap::clear);
        spawn_local(async move {
            for id in ids {
                let _ = commands::dismiss_toast(id).await;
            }
        });
    };

    view! {
        // Two blocks: the scrolling list, which grows, and the dismiss-all
        // control pinned under it so it does not move as toasts arrive.
        <Show when=any>
            <div class="qui-toasts">
                <div class="list">
                    <For each=ordered key=|toast| toast.id let:toast>
                        <ToastCard toast=toast toasts=toasts />
                    </For>
                </div>
                <Show when=many>
                    <button class="dismiss-all" type="button" on:click=dismiss_all>
                        "Dismiss all"
                    </button>
                </Show>
            </div>
        </Show>
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
                "\u{2715}"
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

    /// The backend evicts past its capacity bound and emits only the arrival,
    /// so a merging client would keep what the backend dropped and outgrow the
    /// same bound. Replacing is what keeps the two in step.
    #[wasm_bindgen_test]
    fn a_snapshot_replaces_rather_than_merges() {
        let signal: RwSignal<Toasts> = RwSignal::new(BTreeMap::from([
            (1, toast(ToastKind::Info, None, "evicted")),
            (2, toast(ToastKind::Info, None, "kept")),
        ]));
        apply_snapshot(
            signal,
            vec![
                Toast {
                    id: 2,
                    ..toast(ToastKind::Info, None, "kept")
                },
                Toast {
                    id: 3,
                    ..toast(ToastKind::Info, None, "new")
                },
            ],
        );
        let ids: Vec<u64> = signal.with_untracked(|map| map.keys().copied().collect());
        assert_eq!(
            ids,
            vec![2, 3],
            "id 1 was evicted by the backend and must go"
        );
    }

    /// The other half of the wire contract, pinned from this side. The backend
    /// names the same event, and a rename on either side breaks live delivery
    /// silently — hydration keeps working, which is what hides it.
    #[wasm_bindgen_test]
    fn the_event_name_is_the_one_the_backend_emits() {
        assert_eq!(commands::TOAST_EVENT, "toast");
    }

    #[wasm_bindgen_test]
    fn an_empty_stack_renders_nothing_at_all() {
        // Not cosmetic: the list captures pointer events so its gaps and its
        // scrolling work, so a container rendered with no toasts would leave an
        // invisible band blocking the rows beneath it.
        let signal: RwSignal<Toasts> = RwSignal::new(BTreeMap::new());
        let el = mount(move || view! { <ToastLayer toasts=signal /> });
        assert!(el.query_selector(".qui-toasts").unwrap().is_none());
    }

    #[wasm_bindgen_test]
    fn dismiss_all_appears_only_from_the_second_toast() {
        let one = BTreeMap::from([(1, toast(ToastKind::Info, None, "one"))]);
        let signal = RwSignal::new(one);
        let el = mount(move || view! { <ToastLayer toasts=signal /> });
        assert!(el.query_selector(".qui-toasts").unwrap().is_some());
        assert!(el.query_selector("button.dismiss-all").unwrap().is_none());

        let two = BTreeMap::from([
            (1, toast(ToastKind::Info, None, "one")),
            (2, toast(ToastKind::Info, None, "two")),
        ]);
        let signal = RwSignal::new(two);
        let el = mount(move || view! { <ToastLayer toasts=signal /> });
        assert!(el.query_selector("button.dismiss-all").unwrap().is_some());
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
