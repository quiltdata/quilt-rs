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
    // former at the latter would need an id, which is the same threading `Field` avoids.
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
