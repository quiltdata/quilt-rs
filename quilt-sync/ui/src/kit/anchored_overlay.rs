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
//! has not reached `WebKit`. When it does, the arithmetic below is deleted.
//!
//! # Why not `<details>`
//!
//! It was the cheaper answer and it does not survive the first caller: a row's
//! overflow lives inside a scrolling list, and an inline `<details>` popup is
//! clipped by its own scroll container. The top layer is the whole point.
//!
//! # A surface that has lost its anchor closes
//!
//! Being in the top layer means the surface does not move when the page under it
//! does. Left alone it would sit where the trigger *used* to be, and in a list of
//! rows that is worse than a bug: the commands would appear to belong to whatever
//! row had scrolled into that spot. So any scroll anywhere, and any resize,
//! closes it. Reopening is one click; acting on the wrong file is not undoable.
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
use wasm_bindgen::closure::Closure;

use super::unique_id;

stylance::import_crate_style!(style, "src/kit/anchored_overlay.module.scss");

/// How far under the trigger the surface sits, in pixels.
const OFFSET: f64 = 4.0;
/// How close to the viewport's edge it is allowed to come.
const MARGIN: f64 = 8.0;

/// Which of the surface's edges lines up with the trigger's.
///
/// Not cosmetic. A trigger near the right of its container has no room to grow
/// rightwards, and a surface that tries lands against the viewport clamp — which
/// pins it to the *window* rather than to the trigger, so the gap between the two
/// changes as the window resizes. Trailing controls therefore hang leftwards from
/// their own right edge, which is also where the eye already is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Align {
    /// Left edges together. For a trigger that leads its row.
    #[default]
    Start,
    /// Right edges together, growing leftwards. For a trailing trigger — an
    /// overflow `[⋯]`, or the caret of a [`SplitButton`](super::SplitButton).
    End,
}

#[component]
pub fn AnchoredOverlay(
    /// Draws the control that opens it, handed the surface's id so it can point
    /// at what it controls. Taking a closure rather than a finished view is what
    /// makes that wiring possible at all — the same reason
    /// [`FormControl`](super::FormControl) hands its control the ids it
    /// allocated.
    ///
    /// **The caller owes the trigger `aria-expanded`**, driven by the same
    /// `open` signal it writes, and `aria-controls` set to this id. Without them
    /// the button announces a name and nothing else, and a reader has no way to
    /// know it opens anything.
    trigger: impl FnOnce(String) -> AnyView,
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
    /// Which of the surface's edges lines up with the trigger's. Defaults to
    /// [`Align::Start`]; a trailing trigger wants [`Align::End`]. See the enum for
    /// why this is not left to the viewport clamp.
    #[prop(optional)]
    align: Align,
    /// Drop the surface's own padding, for contents that pad themselves and
    /// need to reach its edge — a menu's items, whose hover highlight would
    /// otherwise stop short of the surface it is drawn inside.
    #[prop(optional)]
    tight: bool,
    /// The surface's contents. Rendered once and kept: the popover hides and
    /// shows the same subtree, so a caller whose body changes drives it with a
    /// signal rather than expecting a rebuild.
    children: Children,
) -> impl IntoView {
    let surface_class = if tight {
        format!("{} {}", style::surface, style::tight)
    } else {
        String::from(style::surface)
    };
    let anchor: NodeRef<leptos::html::Div> = NodeRef::new();
    let surface: NodeRef<leptos::html::Div> = NodeRef::new();
    let surface_id = unique_id("overlay");

    // Held only while open, so a list of a thousand rows has at most one
    // listener rather than a thousand — a popover of `auto` type closes any
    // other, so at most one of these surfaces is open at a time.
    // `new_local`: a DOM closure is neither `Send` nor `Sync`, and this is a
    // single-threaded wasm document.
    let dismisser: StoredValue<Option<Closure<dyn FnMut()>>, LocalStorage> =
        StoredValue::new_local(None);

    let detach = move || {
        dismisser.update_value(|held| {
            if let (Some(closure), Some(window)) = (held.take(), web_sys::window()) {
                let f = closure.as_ref().unchecked_ref();
                drop(window.remove_event_listener_with_callback_and_bool("scroll", f, true));
                drop(window.remove_event_listener_with_callback("resize", f));
            }
        });
    };

    let attach = move || {
        // A frame late, on purpose. Showing a popover can itself scroll the page
        // — moving focus into the top layer is enough — and a listener armed in
        // the same tick would catch that scroll and close what just opened.
        request_animation_frame(move || {
            let Some(window) = web_sys::window() else {
                return;
            };
            // It may have been dismissed inside that frame.
            if !open.get_untracked() {
                return;
            }
            let closure = Closure::<dyn FnMut()>::new(move || open.set(false));
            let f = closure.as_ref().unchecked_ref();
            // Capture phase: a scroll inside the file list never reaches `window`
            // by bubbling, and that list is exactly where the rows are.
            drop(window.add_event_listener_with_callback_and_bool("scroll", f, true));
            drop(window.add_event_listener_with_callback("resize", f));
            dismisser.set_value(Some(closure));
        });
    };

    Effect::new(move |_| {
        let (Some(anchor), Some(surface)) = (anchor.get(), surface.get()) else {
            return;
        };
        let element: &web_sys::HtmlElement = surface.unchecked_ref();
        let was_open = element.matches(":popover-open").unwrap_or(false);

        if open.get() {
            // Shown before it is measured — a popover has no box until it is in
            // the top layer — and positioned in the same synchronous block, so
            // nothing is painted at the default position first.
            if !was_open {
                drop(element.show_popover());
                attach();
            }
            place(element, &anchor.get_bounding_client_rect(), align);
        } else if was_open {
            drop(element.hide_popover());
            detach();
        }
    });

    on_cleanup(detach);

    view! {
        <div class=style::root node_ref=anchor>
            {trigger(surface_id.clone())}
            <div
                node_ref=surface
                id=surface_id
                class=surface_class
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

/// Put the surface under its trigger, and inside the viewport.
///
/// Below by preference and above when below does not fit, because a menu that
/// opens upward is merely unusual while one that opens off the bottom of the
/// window is unreachable.
fn place(element: &web_sys::HtmlElement, anchor: &web_sys::DomRect, align: Align) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let width = f64::from(element.offset_width());
    let height = f64::from(element.offset_height());
    let view_width = window
        .inner_width()
        .ok()
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let view_height = window
        .inner_height()
        .ok()
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);

    let left = horizontal(align, anchor.left(), anchor.right(), width, view_width);

    let below = anchor.bottom() + OFFSET;
    let above = anchor.top() - OFFSET - height;
    let top = if below + height <= view_height - MARGIN || above < MARGIN {
        below.min((view_height - MARGIN - height).max(MARGIN))
    } else {
        above
    };

    let css = element.style();
    drop(css.set_property("left", &format!("{left}px")));
    drop(css.set_property("top", &format!("{top}px")));
}

/// Where the surface's left edge goes: lined up with one of the trigger's edges,
/// then pulled back inside the viewport.
///
/// Split out from [`place`] because it is the whole decision and the rest is DOM
/// plumbing — it can be checked without a browser, a popover or a layout pass.
///
/// The clamp is a last resort and not an alignment. It ties the surface to the
/// *window* instead of to its trigger, so the gap between the two moves when the
/// window resizes; choosing the edge that has room is what keeps it from firing.
/// `max` comes last, so a surface wider than the viewport still starts on screen
/// rather than off the left of it.
fn horizontal(
    align: Align,
    anchor_left: f64,
    anchor_right: f64,
    width: f64,
    view_width: f64,
) -> f64 {
    let aligned = match align {
        Align::Start => anchor_left,
        Align::End => anchor_right - width,
    };
    aligned.min(view_width - MARGIN - width).max(MARGIN)
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "every input here is integral and the arithmetic is min/max over               them, so the results are exact — a tolerance would only hide a               placement that had genuinely moved"
)]
mod tests {
    use super::*;
    use wasm_bindgen_test::*;

    /// A trailing trigger hangs its surface leftwards from its own right edge.
    /// This is the case the clamp used to swallow: left-aligning a `[⋯]` near the
    /// window's edge pinned the surface to the window, which is why the menu
    /// overhung its own button by 45px at a 1280 viewport.
    #[wasm_bindgen_test]
    fn end_alignment_puts_the_right_edges_together() {
        // A `[⋯]` at 1203..1227 in a 1280 viewport, 200px of menu.
        let left = horizontal(Align::End, 1203.0, 1227.0, 200.0, 1280.0);
        assert_eq!(left + 200.0, 1227.0, "right edges must coincide");
        assert!(left > MARGIN, "and it must not have hit the clamp");
    }

    #[wasm_bindgen_test]
    fn start_alignment_puts_the_left_edges_together() {
        let left = horizontal(Align::Start, 40.0, 72.0, 200.0, 1280.0);
        assert_eq!(left, 40.0);
    }

    /// The clamp still exists for the case it was written for: a surface that
    /// genuinely cannot fit beside its trigger.
    #[wasm_bindgen_test]
    fn a_surface_that_cannot_fit_is_pulled_inside_the_viewport() {
        // Start-aligned against a trigger hard against the right edge.
        let left = horizontal(Align::Start, 1200.0, 1232.0, 200.0, 1280.0);
        assert_eq!(left, 1280.0 - MARGIN - 200.0);

        // End-aligned against a trigger hard against the left edge: the surface
        // would start off-screen, so it is pushed back to the margin.
        let left = horizontal(Align::End, 0.0, 24.0, 200.0, 1280.0);
        assert_eq!(left, MARGIN);
    }

    /// Wider than the viewport: it starts on screen rather than off the left of
    /// it, which is what putting `max` last buys.
    #[wasm_bindgen_test]
    fn a_surface_wider_than_the_viewport_still_starts_on_screen() {
        let left = horizontal(Align::End, 300.0, 340.0, 900.0, 400.0);
        assert_eq!(left, MARGIN);
    }
}
