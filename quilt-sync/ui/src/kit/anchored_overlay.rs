//! A surface that opens against the control that summoned it.
//!
//! # Why this is allowed to exist
//!
//! DESIGN.md's **Platform Owns The Keyboard Rule** says the system ships no
//! listbox, no combobox and no popover — because each of those *replaces* a
//! native element that already works. This replaces nothing: it is the platform's
//! own `popover`, driven the way [`Dialog`](super::Dialog) drives the platform's
//! own `<dialog>`. Light dismiss, Escape and the top layer are the browser's, so
//! none of them are hand-written here.
//!
//! What is hand-written is the position, and only because CSS anchor positioning
//! has not reached `WebKit`. When it does, the effect below is deleted.
//!
//! # Why not `<details>`
//!
//! It was the cheaper answer and it does not survive the first caller: a row's
//! overflow lives inside a scrolling list, and an inline `<details>` popup is
//! clipped by its own scroll container. The top layer is the whole point.
//!
//! # The body can be pending
//!
//! Its first caller fetches. The surface opens **immediately** into whatever the
//! caller passes — a few `SkeletonBox`es — and fills when the data lands. A
//! trigger that spins with nothing opening reads as broken, and
//! [`PageLayout`](super::PageLayout) already retired the whole-page version of
//! that idea: work is shown at the scope it belongs to, and this surface is its
//! own scope.

use leptos::prelude::*;
use wasm_bindgen::JsCast;

stylance::import_crate_style!(style, "src/kit/anchored_overlay.module.scss");

/// How far under the trigger the surface sits, in pixels.
const OFFSET: f64 = 4.0;

#[component]
pub fn AnchoredOverlay(
    /// The control that opens it. The caller builds it and writes `open`, so this
    /// works with a `Button` carrying a count and with an `IconButton` carrying a
    /// glyph, which are the two that exist.
    trigger: AnyView,
    /// Owned by the caller and kept in step with the element in both directions:
    /// the effect opens and closes the surface when this changes, and the
    /// `toggle` event — which fires for Escape and for a click outside as well as
    /// for our own call — writes back. Without the write-back, dismissing would
    /// leave the signal saying `true` and the next click would do nothing.
    open: RwSignal<bool>,
    /// Names the surface, because the trigger's name does not reach it once it is
    /// in the top layer and no longer a descendant in the visual tree.
    #[prop(into)]
    aria_label: String,
    /// The surface's contents. Rendered once and kept: the popover hides and
    /// shows the same subtree, so a caller whose body changes drives it with a
    /// signal rather than expecting a rebuild.
    children: Children,
) -> impl IntoView {
    let anchor: NodeRef<leptos::html::Div> = NodeRef::new();
    let surface: NodeRef<leptos::html::Div> = NodeRef::new();

    Effect::new(move |_| {
        let (Some(anchor), Some(surface)) = (anchor.get(), surface.get()) else {
            return;
        };
        let element: &web_sys::HtmlElement = surface.unchecked_ref();
        let is_open = element.matches(":popover-open").unwrap_or(false);

        if open.get() {
            // Positioned before it is shown, so it never paints at 0,0 first.
            let rect = anchor
                .unchecked_ref::<web_sys::Element>()
                .get_bounding_client_rect();
            let css = element.style();
            drop(css.set_property("left", &format!("{}px", rect.left())));
            drop(css.set_property("top", &format!("{}px", rect.bottom() + OFFSET)));
            if !is_open {
                drop(element.show_popover());
            }
        } else if is_open {
            drop(element.hide_popover());
        }
    });

    view! {
        <div class=style::root node_ref=anchor>
            {trigger}
            <div
                node_ref=surface
                class=style::surface
                // `auto`, not `manual`: `auto` is the one that brings light
                // dismiss and Escape with it, which is the entire reason for
                // using the platform's popover rather than a div.
                popover="auto"
                aria-label=aria_label
                // `ToggleEvent` is not in this web-sys, and it is not needed:
                // the element knows, and asking it cannot disagree with it.
                on:toggle=move |_| {
                    let Some(element) = surface.get_untracked() else { return };
                    let opened = element
                        .unchecked_ref::<web_sys::Element>()
                        .matches(":popover-open")
                        .unwrap_or(false);
                    if open.get_untracked() != opened {
                        open.set(opened);
                    }
                }
            >
                {children()}
            </div>
        </div>
    }
}
