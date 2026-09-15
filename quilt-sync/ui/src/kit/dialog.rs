//! A modal, on the native `<dialog>` element.
//!
//! # It replaces four hand-rolled overlays
//!
//! v1 has the same modal written four times — `set_remote_popup`, `ignore_popup`,
//! `workflow_select`, and the create-package form inside `installed_packages_list`. Each
//! is a `div.popup-overlay` with a click-outside handler, and **none of them traps focus,
//! handles Escape, or escapes its parent's `overflow`**. Tab out of one and you are
//! tabbing through the page behind it.
//!
//! `<dialog>` + `showModal()` gives all three from the platform: the top layer (so no
//! `z-index` and no clipping), a focus trap, Escape, and `::backdrop` to style. `WebKitGTK`
//! has had it since 2.36; this machine runs 2.52.
//!
//! # Not a violation of the anchored-positioning ban
//!
//! That ban is about positioning relative to an *element* — tooltips, popovers, dropdowns,
//! the flip-and-shift machinery that comes with them. A centred modal is positioned
//! relative to the viewport, needs none of it, and the design record already names "a
//! centred native `<dialog>`" as one of the two honest options for anything that will not
//! fit inline.
//!
//! # The backdrop does not close it
//!
//! Deliberately unlike v1, whose overlay closed on any outside click. Every one of these
//! dialogs contains a form, and a stray click discarding typed input is a bad trade for
//! saving a movement to Cancel. Escape still closes — that is native and expected — and
//! there is always an explicit Cancel.

use leptos::prelude::*;

stylance::import_crate_style!(style, "src/kit/dialog.module.scss");

#[component]
pub fn Dialog(
    /// Owned by the caller, and kept in step with the element in both directions: the
    /// effect below opens and closes the dialog when this changes, and the `close` event —
    /// which fires for Escape as well as for `close()` — writes back. Without the write-back
    /// an Escape would leave the signal saying `true` and the next open would do nothing.
    open: RwSignal<bool>,
    #[prop(into)] title: String,
    /// The buttons, right-aligned in the footer. Primary last, as everywhere else on this
    /// platform.
    footer: AnyView,
    children: Children,
) -> impl IntoView {
    let element: NodeRef<leptos::html::Dialog> = NodeRef::new();
    // Named twice on purpose: `aria-label` is what a screen reader announces when the
    // modal opens, and the heading is what a sighted reader sees first. Pointing the
    // former at the latter would need an id, which is the same threading `FormControl` avoids.
    let heading = title.clone();

    Effect::new(move |_| {
        let Some(dialog) = element.get() else { return };
        if open.get() {
            if !dialog.open() {
                // `show_modal`, never `show`: the non-modal form gets no focus trap, no
                // Escape and no backdrop, which is the whole reason for being here.
                drop(dialog.show_modal());
            }
        } else if dialog.open() {
            dialog.close();
        }
    });

    view! {
        <dialog
            node_ref=element
            class=style::root
            aria-label=title
            on:close=move |_| open.set(false)
        >
            <h2 class=style::title>{heading}</h2>
            <div class=style::body>{children()}</div>
            <div class=style::footer>{footer}</div>
        </dialog>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::*;

    /// Yields to the event loop, not just to the reactive system. `close` is queued as
    /// a task rather than fired synchronously, so a `tick` alone never sees it.
    async fn next_task() {
        let promise = js_sys::Promise::new(&mut |resolve, _| {
            drop(
                web_sys::window()
                    .unwrap()
                    .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 0),
            );
        });
        drop(wasm_bindgen_futures::JsFuture::from(promise).await);
    }

    fn mount<N: IntoView + 'static>(f: impl FnOnce() -> N + 'static) -> web_sys::Element {
        let doc = web_sys::window().unwrap().document().unwrap();
        let container: web_sys::HtmlElement =
            doc.create_element("div").unwrap().dyn_into().unwrap();
        doc.body().unwrap().append_child(&container).unwrap();
        leptos::mount::mount_to(container.clone(), f).forget();
        container.into()
    }

    /// `show_modal`, never `show`: the non-modal form gets no focus trap, no Escape and
    /// no backdrop, which is the whole reason for being here.
    #[wasm_bindgen_test]
    async fn the_signal_opens_and_closes_the_dialog() {
        let open = RwSignal::new(false);
        let el = mount(move || {
            view! {
                <Dialog open=open title="Create package" footer=view! { "footer" }.into_any()>
                    "body"
                </Dialog>
            }
        });
        let dialog: web_sys::HtmlDialogElement = el
            .query_selector("dialog")
            .unwrap()
            .expect("the dialog")
            .dyn_into()
            .unwrap();
        assert!(!dialog.open(), "closed until the caller says otherwise");

        open.set(true);
        leptos::task::tick().await;
        assert!(dialog.open());

        open.set(false);
        leptos::task::tick().await;
        assert!(!dialog.open());
    }

    /// The write-back. Escape closes the element without touching the signal, so
    /// without this the next open would do nothing.
    #[wasm_bindgen_test]
    async fn closing_the_element_writes_back_to_the_signal() {
        let open = RwSignal::new(false);
        let el = mount(move || {
            view! {
                <Dialog open=open title="Create package" footer=view! { "footer" }.into_any()>
                    "body"
                </Dialog>
            }
        });
        let dialog: web_sys::HtmlDialogElement = el
            .query_selector("dialog")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap();
        // Opened through the signal, as a caller does: `close()` on a dialog that was
        // never open fires nothing, and the write-back is what this is about.
        open.set(true);
        leptos::task::tick().await;
        assert!(dialog.open(), "open before closing it");

        dialog.close();
        next_task().await;
        leptos::task::tick().await;
        assert!(
            !open.get_untracked(),
            "an Escape that left the signal true would jam the next open"
        );
    }

    /// Named twice: `aria-label` is what a screen reader announces when the modal
    /// opens, and the heading is what a sighted reader sees first.
    #[wasm_bindgen_test]
    fn the_dialog_is_named_for_both_readers() {
        let open = RwSignal::new(false);
        let el = mount(move || {
            view! {
                <Dialog open=open title="Create package" footer=view! { "footer" }.into_any()>
                    "body"
                </Dialog>
            }
        });
        let dialog = el.query_selector("dialog").unwrap().unwrap();
        assert_eq!(
            dialog.get_attribute("aria-label").as_deref(),
            Some("Create package")
        );
        let heading = el.query_selector("h2").unwrap().expect("a visible title");
        assert_eq!(heading.text_content().as_deref(), Some("Create package"));
    }
}
