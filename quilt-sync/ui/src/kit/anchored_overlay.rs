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
//! has not reached every `WebKit` this app runs on. When it does, the arithmetic
//! in `placement.rs` is deleted — for [`Tooltip`](super::Tooltip) too, which
//! borrows it rather than keeping a second copy.
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
//! closes it — except a scroll of the surface's own overflow, which moves no
//! anchor. Reopening is one click; acting on the wrong file is not undoable.
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

use super::placement::place;
use super::unique_id;

stylance::import_crate_style!(style, "src/kit/anchored_overlay.module.scss");

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

/// The scroll and resize listener held while a surface is open.
type Dismisser = Closure<dyn FnMut(web_sys::Event)>;

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
    /// The contents scroll themselves: the surface stops scrolling its
    /// overflow and lays them out as a column it bounds, so a list can scroll
    /// under a footer that stays in view. One scrollbar, never two.
    #[prop(optional)]
    contained: bool,
    /// The surface's contents. Rendered once and kept: the popover hides and
    /// shows the same subtree, so a caller whose body changes drives it with a
    /// signal rather than expecting a rebuild.
    children: Children,
) -> impl IntoView {
    let mut surface_class = String::from(style::surface);
    for (on, class) in [(tight, style::tight), (contained, style::contained)] {
        if on {
            surface_class.push(' ');
            surface_class.push_str(class);
        }
    }
    let anchor: NodeRef<leptos::html::Div> = NodeRef::new();
    let surface: NodeRef<leptos::html::Div> = NodeRef::new();
    let surface_id = unique_id("overlay");

    // Held only while open, so a list of a thousand rows has at most one
    // listener rather than a thousand — a popover of `auto` type closes any
    // other, so at most one of these surfaces is open at a time.
    // `new_local`: a DOM closure is neither `Send` nor `Sync`, and this is a
    // single-threaded wasm document.
    let dismisser: StoredValue<Option<Dismisser>, LocalStorage> = StoredValue::new_local(None);

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
            // It may have been dismissed inside that frame, or disposed with
            // its owner. A disposed one must not attach: `on_cleanup` has
            // already run, so nothing would ever remove the listener.
            if !open.try_get_untracked().unwrap_or(false) {
                return;
            }
            let closure =
                Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
                    // The surface scrolling its own overflow has moved nothing it is anchored to.
                    let inside = event
                        .target()
                        .and_then(|target| target.dyn_into::<web_sys::Node>().ok())
                        .zip(surface.get_untracked())
                        .is_some_and(|(target, surface)| {
                            surface
                                .unchecked_ref::<web_sys::Node>()
                                .contains(Some(&target))
                        });
                    if !inside {
                        open.set(false);
                    }
                });
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{mount, sleep_ms, unmount_earlier};
    use wasm_bindgen_test::*;

    fn scroll(target: &web_sys::EventTarget) {
        let event = web_sys::Event::new("scroll").unwrap();
        target.dispatch_event(&event).unwrap();
    }

    /// The surface scrolls its own overflow (`max-height: 60vh`), and a scroll
    /// there has not moved the anchor, so it must not dismiss the surface the
    /// reader is scrolling. A scroll anywhere else still does.
    #[wasm_bindgen_test]
    async fn scrolling_inside_the_surface_keeps_it_open() {
        // Alone on the page: what earlier tests left mounted can scroll the
        // window once this surface opens, and that scroll closes it for a
        // reason this test is not about. Which tests run first shifts as tests
        // are added, so without this it passes or fails by link order.
        unmount_earlier();
        let open = RwSignal::new(false);
        let el = mount(move || {
            view! {
                <AnchoredOverlay
                    trigger=|_| view! { <button>"Open"</button> }.into_any()
                    open=open
                    aria_label="Scrolled"
                >
                    <p>"Long"</p>
                </AnchoredOverlay>
            }
        });
        open.set(true);
        leptos::task::tick().await;
        // The dismisser is armed a frame late; wait it out.
        sleep_ms(50).await;
        let surface = el
            .query_selector("[aria-label='Scrolled']")
            .unwrap()
            .unwrap();

        scroll(&surface);
        leptos::task::tick().await;
        assert!(
            open.get_untracked(),
            "a scroll inside the surface closed it"
        );
        assert!(surface.matches(":popover-open").unwrap());

        scroll(&web_sys::window().unwrap().document().unwrap());
        leptos::task::tick().await;
        assert!(
            !open.get_untracked(),
            "a scroll outside the surface must close it"
        );
    }
}
