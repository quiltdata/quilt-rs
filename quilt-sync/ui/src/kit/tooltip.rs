//! Words shown on hover or keyboard focus: a hint that explains a mark, or the
//! whole of a value the row had to cut short.
//!
//! # Why this is allowed to exist
//!
//! DESIGN.md bans the hand-built popover, and this is not one: it is the
//! platform's own `popover`, as [`AnchoredOverlay`](super::AnchoredOverlay) is,
//! with the position borrowed from the same arithmetic. It holds plain text
//! only, so it never grows into a menu.
//!
//! # Instead of `title`
//!
//! DESIGN.md's Tooltip Rule: what shows on hover or focus is this, not a
//! `title`. A `title` waits about a second, never appears on keyboard focus or
//! touch, cannot be styled, and an inner one silently wins over an outer one,
//! which is how the row that differs came to show its path where its
//! explanation was meant to be. The `title`s still in the kit predate this
//! component and move to it as their code is next worked on. An element with a
//! tooltip carries no `title`, or the browser's box covers this one.
//!
//! A touch screen never shows a tooltip, so it explains a mark and is never the
//! mark: what a reader needs at a glance is drawn on the page.
//!
//! # `manual`, not `auto`
//!
//! An `auto` popover closes every other `auto` popover when it opens. A row's
//! `[⋯]` menu is one, so hovering a row's mark on the way to a menu item would
//! shut the menu under the pointer. `manual` closes nothing, and the price is
//! that Escape is ours to write: one `keydown` listener, held only while open.
//!
//! # How it behaves
//!
//! - **Hover** opens it after [`DELAY`] and closes it [`GRACE`] after the
//!   pointer leaves. The pointer may cross onto the surface in that window and
//!   it stays: what is shown on hover has to be hoverable, or a reader who
//!   reaches for the words to read them loses them. Moving from one trigger to
//!   the next inside the window opens the next at once — the reader has already
//!   said they want these.
//! - **Keyboard focus** opens it at once, and only `:focus-visible` focus: a
//!   click focuses a button too, and a sentence that pops up under every click
//!   is noise. Blur closes it.
//! - **Escape** closes it and nothing else. It cancels the key, so the dialog or
//!   menu underneath stays open; the listener exists only while a tooltip is
//!   showing, so with none showing Escape is theirs untouched. Focus does not
//!   move.
//! - **Pointer down, any scroll, any resize** close it, for the reason
//!   [`AnchoredOverlay`](super::AnchoredOverlay) closes: in the top layer it
//!   does not move with the page, and a sentence beside the wrong row is worse
//!   than none.
//! - **Touch** does nothing. A long press in a webview is the system's context
//!   menu, and the fact is already elsewhere on the page.
//!
//! # The surface is always in the document
//!
//! Closed, it is hidden and not removed. A description is read from the element
//! `aria-describedby` names, and a hidden one still serves — so focus that lands
//! before the surface opens, or a reader that never opens it, still gets the
//! words. One surface per trigger costs a node, and a list marks only its rows
//! that differ.
//!
//! It is also `aria-hidden`, open or closed. Its words reach a reader through
//! the description and only through it — so a reading cursor never meets them
//! a second time, and a surface drawn inside a `<label>` never becomes part of
//! the name of the control that label belongs to.
//!
//! # Where the hover lands and where the focus lands can differ
//!
//! The usual caller has one element that is both: an icon button. The trigger
//! closure draws it, is handed the surface's id, and puts `aria-describedby` on
//! it; focus bubbling out of it is heard by the wrapper. That is the whole API.
//!
//! A row is not that shape. The thing to point at is a mark — not focusable,
//! and making it focusable would put a tab stop that does nothing on every
//! marked row. The thing focus lands on is the row's own control, drawn
//! elsewhere and not inside the mark. So a caller may make the
//! [`TooltipHandle`] itself, before either is drawn: the mark goes in `trigger`
//! and is the hover target and the anchor, and the control names
//! [`TooltipHandle::id`] and forwards its focus to the same handle. Same
//! surface, same timers, one sentence — and a row with no control is simply
//! hover-only, which is why the mark beside it has to carry the meaning
//! without the sentence.

use std::cell::Cell;
use std::time::Duration;

use leptos::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;

use super::Align;
use super::placement::place;
use super::unique_id;

stylance::import_crate_style!(style, "src/kit/tooltip.module.scss");

/// How long the pointer rests before it opens. The common default; shorter
/// fires on every row a pointer merely crosses in a dense list.
const DELAY: Duration = Duration::from_millis(500);

/// How long it survives the pointer leaving, which is the time the pointer has
/// to reach the surface — or the next trigger, which then opens at once.
const GRACE: Duration = Duration::from_millis(150);

thread_local! {
    /// The one tooltip showing, if any. Showing another closes it first, so a
    /// pointer moving down a column of marks never leaves two sentences up.
    static SHOWN: Cell<Option<RwSignal<bool>>> = const { Cell::new(None) };
    /// When a hover last let one go, in `Date` milliseconds. A pointer
    /// entering a trigger within [`GRACE`] of it is still reading.
    static LET_GO_AT: Cell<f64> = const { Cell::new(f64::NEG_INFINITY) };
}

/// One tooltip's state: its id, whether it shows, and its pending timer.
///
/// Made by [`Tooltip`] when the caller passes none, which is the usual case.
/// Made by the caller when the element focus lands on is not inside `trigger`,
/// so that element can name the surface and forward its focus — see the module
/// docs. `Copy`, so it can be handed to both places.
#[derive(Clone, Copy)]
pub struct TooltipHandle {
    id: StoredValue<String>,
    open: RwSignal<bool>,
    timer: StoredValue<Option<TimeoutHandle>>,
}

impl Default for TooltipHandle {
    fn default() -> Self {
        Self::new()
    }
}

impl TooltipHandle {
    #[must_use]
    pub fn new() -> Self {
        Self {
            id: StoredValue::new(unique_id("tooltip")),
            open: RwSignal::new(false),
            timer: StoredValue::new(None),
        }
    }

    /// The surface's id, for the focusable element's `aria-describedby`. Join it
    /// to any id that element already names, with a space: a description is a
    /// list, and replacing one drops what it said.
    #[must_use]
    pub fn id(&self) -> String {
        self.id.get_value()
    }

    /// Focus reached the element this describes. Opens at once if it came from
    /// the keyboard, and does nothing for a click's focus.
    pub fn focus_in(&self, event: &web_sys::Event) {
        let keyboard = event
            .target()
            .and_then(|target| target.dyn_into::<web_sys::Element>().ok())
            .is_some_and(|element| element.matches(":focus-visible").unwrap_or(false));
        if keyboard {
            self.show();
        }
    }

    /// Focus left it.
    pub fn focus_out(&self) {
        self.hide(false);
    }

    fn enter(self) {
        self.cancel();
        if self.open.get_untracked() {
            return;
        }
        let warm = SHOWN.get().is_some() || now() - LET_GO_AT.get() < millis(GRACE);
        if warm {
            self.show();
        } else {
            self.later(DELAY, move || self.show());
        }
    }

    fn leave(self) {
        self.cancel();
        if self.open.get_untracked() {
            self.later(GRACE, move || self.hide(true));
        }
    }

    fn show(self) {
        self.cancel();
        if let Some(other) = SHOWN.get()
            && other != self.open
        {
            other.try_set(false);
        }
        SHOWN.set(Some(self.open));
        self.open.try_set(true);
    }

    /// `let_go` is a hover ending, which warms the next trigger; Escape, a
    /// click or a scroll is the reader dismissing it, which does not.
    fn hide(self, let_go: bool) {
        self.cancel();
        if SHOWN.get() == Some(self.open) {
            SHOWN.set(None);
        }
        if self.open.try_get_untracked().unwrap_or(false) {
            if let_go {
                LET_GO_AT.set(now());
            }
            self.open.try_set(false);
        }
    }

    fn later(self, after: Duration, then: impl FnOnce() + 'static) {
        let pending = set_timeout_with_handle(then, after).ok();
        self.timer.try_set_value(pending);
    }

    fn cancel(self) {
        if let Some(Some(pending)) = self.timer.try_update_value(Option::take) {
            pending.clear();
        }
    }
}

fn now() -> f64 {
    js_sys::Date::now()
}

fn millis(duration: Duration) -> f64 {
    // Both constants are a few hundred milliseconds; nothing is lost.
    f64::from(u32::try_from(duration.as_millis()).unwrap_or(u32::MAX))
}

/// The listener held while a tooltip shows: Escape, pointer down, scroll and
/// resize, all of which close it.
type Dismisser = Closure<dyn FnMut(web_sys::Event)>;

const DISMISSING: [&str; 4] = ["keydown", "pointerdown", "scroll", "resize"];

#[component]
pub fn Tooltip(
    /// What it says. Plain text and nothing else, enforced here rather than
    /// asked for: a link or a button inside would be a control only a hovering
    /// pointer can reach. Anything richer is
    /// [`AnchoredOverlay`](super::AnchoredOverlay)'s job.
    #[prop(into)]
    text: Signal<String>,
    /// Which of the surface's edges lines up with the trigger's — the overlay's
    /// own [`Align`], for the same reason: a trailing trigger hangs its sentence
    /// leftwards rather than against the window.
    #[prop(optional)]
    align: Align,
    /// The state, when the caller made it because the element focus lands on is
    /// not inside `trigger`. Absent, the tooltip makes its own.
    #[prop(optional)]
    handle: Option<TooltipHandle>,
    /// Draws what the pointer rests on, handed the surface's id so a focusable
    /// trigger can name it in `aria-describedby`. **That wiring is the
    /// caller's**, as `aria-controls` is for the overlay: the tooltip cannot
    /// reach into the element the closure draws, and a surface no control
    /// names is a sentence a screen reader never hears.
    trigger: impl FnOnce(String) -> AnyView,
) -> impl IntoView {
    let handle = handle.unwrap_or_default();
    let open = handle.open;
    let anchor: NodeRef<leptos::html::Span> = NodeRef::new();
    let surface: NodeRef<leptos::html::Span> = NodeRef::new();

    // Held only while it shows, and only one shows at a time, so a list of
    // marked rows holds at most one set of window listeners.
    let dismisser: StoredValue<Option<Dismisser>, LocalStorage> = StoredValue::new_local(None);

    let detach = move || {
        dismisser.update_value(|held| {
            if let (Some(closure), Some(window)) = (held.take(), web_sys::window()) {
                let f = closure.as_ref().unchecked_ref();
                for kind in DISMISSING {
                    drop(window.remove_event_listener_with_callback_and_bool(kind, f, true));
                }
            }
        });
    };

    let attach = move || {
        // A frame late, as the overlay does and for a sharper reason: focus
        // from Tab scrolls its element into view *after* `focusin`, so a
        // listener armed at once would catch that scroll and close what the
        // focus just opened. Placed again here, because that scroll moved the
        // anchor under the position written a moment ago.
        request_animation_frame(move || {
            let Some(window) = web_sys::window() else {
                return;
            };
            if !open.try_get_untracked().unwrap_or(false) {
                return;
            }
            if let (Some(anchor), Some(surface)) = (anchor.get_untracked(), surface.get_untracked())
            {
                place(
                    surface.unchecked_ref(),
                    &anchor.get_bounding_client_rect(),
                    align,
                );
            }
            let closure =
                Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
                    if event.type_() == "keydown" {
                        let escape = event
                            .dyn_ref::<web_sys::KeyboardEvent>()
                            .is_some_and(|key| key.key() == "Escape");
                        if !escape {
                            return;
                        }
                        // Cancelled, so the platform does not also treat it as
                        // a close request for the dialog or menu underneath.
                        // The first Escape takes the sentence away; the next is
                        // theirs.
                        event.prevent_default();
                    }
                    handle.hide(false);
                });
            let f = closure.as_ref().unchecked_ref();
            // Capture phase throughout: a scroll inside the file list never
            // reaches `window` by bubbling, and a `keydown` handled and stopped
            // on its way down must still close the sentence first.
            for kind in DISMISSING {
                drop(window.add_event_listener_with_callback_and_bool(kind, f, true));
            }
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
            // Shown before it is measured, and placed in the same synchronous
            // block, so nothing is painted at the default position first.
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

    on_cleanup(move || {
        detach();
        handle.hide(false);
    });

    view! {
        <span
            class=style::root
            node_ref=anchor
            on:pointerenter=move |_| handle.enter()
            on:pointerleave=move |_| handle.leave()
            on:focusin=move |event| handle.focus_in(&event)
            on:focusout=move |_| handle.focus_out()
        >
            {trigger(handle.id())}
            // Inside the wrapper in the document, though drawn in the top
            // layer: the pointer crossing onto it has not left the wrapper, so
            // the sentence stays while it is read.
            <span
                node_ref=surface
                id=handle.id()
                class=style::surface
                popover="manual"
                role="tooltip"
                // Out of the tree, and still a description: a node named
                // directly by `aria-describedby` is read even when hidden. Left
                // in, an open surface drawn inside a `<label>` — a selectable
                // row's `Differs` label is — would join the label's control's name, and a
                // reader would hear the sentence twice.
                aria-hidden="true"
            >
                {move || text.get()}
            </span>
        </span>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::AnchoredOverlay;
    use crate::test_support::{
        blur, button_saying, describing_tooltip, focus_report, keyboard_focus, mount, sleep_ms,
        unmount_earlier,
    };
    use wasm_bindgen_test::*;

    fn fire(target: &web_sys::EventTarget, kind: &str) {
        target
            .dispatch_event(&web_sys::Event::new(kind).unwrap())
            .unwrap();
    }

    fn escape() {
        let init = web_sys::KeyboardEventInit::new();
        init.set_key("Escape");
        init.set_cancelable(true);
        let event =
            web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
        web_sys::window()
            .unwrap()
            .document()
            .unwrap()
            .dispatch_event(&event)
            .unwrap();
    }

    /// The trigger's wrapper, which is what hears the pointer.
    fn wrapper(el: &web_sys::Element) -> web_sys::Element {
        el.query_selector("[aria-describedby]")
            .unwrap()
            .expect("the trigger names its tooltip")
            .parent_element()
            .unwrap()
    }

    fn surface(el: &web_sys::Element) -> web_sys::Element {
        el.query_selector("[role='tooltip']")
            .unwrap()
            .expect("the surface is in the document while closed")
    }

    /// The trigger names the surface, the surface holds the words, and it is
    /// there before it is shown — so a description is never missing.
    #[wasm_bindgen_test]
    fn the_trigger_names_a_surface_that_is_always_there() {
        let el = mount(|| {
            view! {
                <Tooltip
                    text="Explains the mark.".to_string()
                    trigger=|id| view! { <button aria-describedby=id>"Mark"</button> }.into_any()
                />
            }
        });
        let surface = surface(&el);
        assert_eq!(surface.text_content().unwrap(), "Explains the mark.");
        assert_eq!(
            wrapper(&el)
                .query_selector("button")
                .unwrap()
                .unwrap()
                .get_attribute("aria-describedby"),
            surface.get_attribute("id"),
        );
        assert!(!surface.matches(":popover-open").unwrap());
    }

    /// Hover waits, then opens; Escape closes it. An `auto` popover open
    /// underneath, which is what a row's `[⋯]` is, survives the tooltip
    /// opening — the whole reason for `manual` — and our Escape handler leaves
    /// it alone. (A synthetic key is not a close request to the platform, so
    /// this cannot show the cancelled key sparing the menu; that wants a real
    /// key, in the gallery.)
    #[wasm_bindgen_test]
    async fn hover_opens_late_and_escape_closes_only_the_tooltip() {
        unmount_earlier();
        let menu = RwSignal::new(false);
        let el = mount(move || {
            view! {
                <AnchoredOverlay
                    trigger=|_| view! { <button>"Menu"</button> }.into_any()
                    open=menu
                    aria_label="Menu"
                >
                    <p>"Items"</p>
                </AnchoredOverlay>
                <Tooltip
                    text="Explains the mark.".to_string()
                    trigger=|id| view! { <button aria-describedby=id>"Mark"</button> }.into_any()
                />
            }
        });
        menu.set(true);
        leptos::task::tick().await;
        sleep_ms(50).await;

        fire(&wrapper(&el), "pointerenter");
        sleep_ms(100).await;
        assert!(
            !surface(&el).matches(":popover-open").unwrap(),
            "a pointer passing over opens nothing"
        );
        sleep_ms(500).await;
        assert!(surface(&el).matches(":popover-open").unwrap());
        assert!(menu.get_untracked(), "showing the tooltip closed the menu");

        escape();
        leptos::task::tick().await;
        assert!(!surface(&el).matches(":popover-open").unwrap());
        assert!(
            menu.get_untracked(),
            "the Escape that closed the tooltip also closed the menu"
        );
    }

    fn is_open(el: &web_sys::Element) -> bool {
        surface(el).matches(":popover-open").unwrap()
    }

    fn focused() -> Option<web_sys::Element> {
        web_sys::window()
            .unwrap()
            .document()
            .unwrap()
            .active_element()
    }

    fn one_mark() -> web_sys::Element {
        mount(|| {
            view! {
                <Tooltip
                    text="Explains the mark.".to_string()
                    trigger=|id| view! { <button aria-describedby=id>"Mark"</button> }.into_any()
                />
            }
        })
    }

    /// Keyboard focus opens it at once, with no hover delay; Escape closes it
    /// and leaves focus where it was.
    #[wasm_bindgen_test]
    async fn keyboard_focus_opens_it_and_escape_leaves_focus_on_the_trigger() {
        unmount_earlier();
        let el = one_mark();
        let mark = button_saying(&el, "Mark");
        let tip = describing_tooltip(&mark).expect("the trigger names its tooltip");
        assert_eq!(tip.text_content().unwrap(), "Explains the mark.");

        keyboard_focus(&mark);
        leptos::task::tick().await;
        assert!(
            tip.matches(":popover-open").unwrap(),
            "keyboard focus opened nothing; {}",
            focus_report(&mark)
        );

        // Escape is heard from the frame after it opens.
        sleep_ms(50).await;
        escape();
        leptos::task::tick().await;
        assert!(!tip.matches(":popover-open").unwrap());
        assert_eq!(
            focused().as_ref(),
            Some(mark.unchecked_ref::<web_sys::Element>()),
            "Escape moved focus off the trigger"
        );
        blur(&mark);
    }

    #[wasm_bindgen_test]
    async fn blur_closes_it() {
        unmount_earlier();
        let el = one_mark();
        let mark = button_saying(&el, "Mark");
        keyboard_focus(&mark);
        leptos::task::tick().await;
        assert!(is_open(&el), "{}", focus_report(&mark));

        blur(&mark);
        leptos::task::tick().await;
        assert!(!is_open(&el), "blur left the tooltip open");
    }

    /// Leaving does not close it at once — the pointer may be on its way to the
    /// surface — but it does close once the grace is over.
    #[wasm_bindgen_test]
    async fn a_pointer_leaving_closes_it_after_the_grace() {
        unmount_earlier();
        let el = one_mark();
        fire(&wrapper(&el), "pointerenter");
        sleep_ms(600).await;
        assert!(is_open(&el));

        fire(&wrapper(&el), "pointerleave");
        sleep_ms(50).await;
        assert!(is_open(&el), "closed before the grace was over");
        sleep_ms(200).await;
        assert!(!is_open(&el), "still open after the grace");
        // Out of the warm window, so the next hover waits its delay again.
        sleep_ms(200).await;
    }
}
