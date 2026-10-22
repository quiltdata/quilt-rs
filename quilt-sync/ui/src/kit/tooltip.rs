//! Words shown on hover or keyboard focus: a hint that explains a mark, or the
//! whole of a value the row had to cut short.
//!
//! # Why this is allowed to exist
//!
//! DESIGN.md bans the hand-built popover, and this is not one: it is the
//! platform's own `popover`, as [`AnchoredOverlay`](super::AnchoredOverlay) is,
//! with the position borrowed from the same arithmetic. It holds a title and a
//! few muted lines of plain text, nothing interactive, so it never grows into
//! a menu.
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
//! - **A whole row's tooltip is the exception**: the pointer passes through
//!   it. It hangs over the rows below, which are what a reader scans next, so
//!   moving down lands on the next row and the hand-over shows that row's
//!   words. A reader who needs them for longer finds the same facts on the
//!   package's own page.
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
use std::cell::RefCell;
use std::time::Duration;

use leptos::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;

use super::Align;
use super::placement::place;
use super::unique_id;

stylance::import_crate_style!(pub(super) style, "src/kit/tooltip.module.scss");

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
    /// The focus event that last opened one, so a list's names can tell a
    /// row's own sentence, opened by that same focus, from any other.
    static OPENED_BY_FOCUS: RefCell<Option<web_sys::Event>> = const { RefCell::new(None) };
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
            OPENED_BY_FOCUS.set(Some(event.clone()));
        }
    }

    /// Focus left it.
    pub fn focus_out(&self) {
        self.hide(false);
    }

    /// A finger fires `pointerenter` too, and holding it on a mark would open
    /// the sentence after the delay. Touch does nothing.
    pub(super) fn pointer_enter(self, event: &web_sys::PointerEvent) {
        if event.pointer_type() != "touch" {
            self.enter();
        }
    }

    pub(super) fn enter(self) {
        if self.open.get_untracked() {
            self.cancel();
            return;
        }
        self.enter_with(move || self.show());
    }

    /// A hover began, and `open` decides what it opens when the time comes: at
    /// once while warm, or after [`DELAY`]. While this one is open that is now,
    /// so a list's one surface can move to the next name or let go.
    pub(super) fn enter_with(self, open: impl FnOnce() + 'static) {
        self.cancel();
        let warm = self.open.get_untracked()
            || SHOWN.get().is_some()
            || now() - LET_GO_AT.get() < millis(GRACE);
        if warm {
            open();
        } else {
            self.later(DELAY, open);
        }
    }

    /// Whether this one shows.
    pub(super) fn is_open(self) -> bool {
        self.open.get_untracked()
    }

    /// Whether `event` opened another tooltip, as it bubbled through a row on
    /// its way to the list: that row's sentence about what it is outranks its
    /// name. A sentence opened by anything else does not.
    pub(super) fn another_opened_by(self, event: &web_sys::Event) -> bool {
        let by_this = OPENED_BY_FOCUS.with_borrow(|last| {
            last.as_ref()
                .is_some_and(|last| js_sys::Object::is(last, event))
        });
        by_this && SHOWN.get().is_some_and(|other| other != self.open)
    }

    pub(super) fn leave(self) {
        self.cancel();
        if self.open.get_untracked() {
            self.later(GRACE, move || self.hide(true));
        }
    }

    pub(super) fn show(self) {
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
    pub(super) fn hide(self, let_go: bool) {
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

    pub(super) fn cancel(self) {
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

/// The space keeps a description read from these words from running the
/// title into the first line; on screen it collapses.
fn line_views(lines: Vec<TooltipLine>) -> impl IntoView {
    lines
        .into_iter()
        .map(|line| {
            view! {
                " "
                <span class=style::line>
                    {line.icon.map(|draw| draw())}
                    <span>{line.text}</span>
                </span>
            }
        })
        .collect_view()
}

/// One muted line under a tooltip's title, optionally led by a glyph from
/// [`icons`](super::icons). The glyph is drawn in the line's grey and hidden
/// from assistive tech, as every kit glyph is.
#[derive(Clone)]
pub struct TooltipLine {
    pub text: String,
    pub icon: Option<fn() -> AnyView>,
}

impl TooltipLine {
    #[must_use]
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            icon: None,
        }
    }

    #[must_use]
    pub fn icon(mut self, icon: fn() -> AnyView) -> Self {
        self.icon = Some(icon);
        self
    }
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
    /// Lines under `text`, muted, when there is more than one sentence to say:
    /// `text` is then the title. Plain text too, for the same reason.
    #[prop(optional, into)]
    lines: Signal<Vec<TooltipLine>>,
    /// The wider surface a list's names get, for a title that is a name.
    #[prop(optional)]
    wide: bool,
    /// The trigger is a whole row: the wrapper fills its container's width, so
    /// the row keeps its own and the surface lines up with its edge. The
    /// pointer passes through the surface, onto the rows it covers.
    #[prop(optional)]
    fill: bool,
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
    let wrapper: NodeRef<leptos::html::Span> = NodeRef::new();
    let anchor = Signal::derive_local(move || wrapper.get().map(JsCast::unchecked_into));
    let root = if fill {
        format!("{} {}", style::root, style::fill)
    } else {
        style::root.to_string()
    };
    let mut class = style::surface.to_string();
    for (on, extra) in [(wide, style::name), (fill, style::through)] {
        if on {
            class.push(' ');
            class.push_str(extra);
        }
    }

    view! {
        <span
            class=root
            node_ref=wrapper
            on:pointerenter=move |event| handle.pointer_enter(&event)
            on:pointerleave=move |_| handle.leave()
            on:focusin=move |event| handle.focus_in(&event)
            on:focusout=move |_| handle.focus_out()
        >
            {trigger(handle.id())}
            // Inside the wrapper in the document, though drawn in the top
            // layer: the pointer crossing onto it has not left the wrapper, so
            // the sentence stays while it is read.
            {surface(handle, text, lines, anchor, align, class)}
        </span>
    }
}

/// The floating sentence, placed against whatever `anchor` holds while open:
/// a [`Tooltip`]'s own wrapper, or the name a
/// [`NameTooltip`](super::NameTooltip) last saw the pointer or focus reach.
pub(super) fn surface(
    handle: TooltipHandle,
    text: Signal<String>,
    lines: Signal<Vec<TooltipLine>>,
    anchor: Signal<Option<web_sys::Element>, LocalStorage>,
    align: Align,
    class: String,
) -> impl IntoView {
    let open = handle.open;
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
            if let (Some(Some(anchor)), Some(surface)) =
                (anchor.try_get_untracked(), surface.get_untracked())
            {
                place(
                    surface.unchecked_ref(),
                    &anchor.get_bounding_client_rect(),
                    align,
                );
            }
            // Closed and opened again inside one frame, two of these callbacks
            // run while open; the first one's listeners are the ones kept, or
            // the second would replace them and leave them attached for good.
            if dismisser.with_value(Option::is_some) {
                return;
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

    // Runs again when the anchor moves while open, which is a list's one
    // surface following the pointer from one name to the next — and when the
    // words change while open, since a longer surface may no longer fit below.
    Effect::new(move |_| {
        text.track();
        lines.track();
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
            node_ref=surface
            id=handle.id()
            // A title with lines is roomier than one sentence.
            class=move || {
                if lines.with(Vec::is_empty) {
                    class.clone()
                } else {
                    format!("{class} {}", style::titled)
                }
            }
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
            {move || line_views(lines.get())}
        </span>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::AnchoredOverlay;
    use crate::test_support::{
        blur, button_saying, describing_tooltip, focus_report, keyboard_focus, load_module_style,
        mount, sleep_ms, unmount_earlier,
    };
    use wasm_bindgen_test::*;

    /// A mouse's pointer event: the tooltip ignores a finger's.
    fn fire(target: &web_sys::EventTarget, kind: &str) {
        let init = web_sys::PointerEventInit::new();
        init.set_pointer_type("mouse");
        let event = web_sys::PointerEvent::new_with_event_init_dict(kind, &init).unwrap();
        target.dispatch_event(&event).unwrap();
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

    /// The tooltip's own stylesheet, so the surface is placed and drawn as in
    /// the app.
    fn tooltip_style() {
        load_module_style(
            include_str!("tooltip.module.scss"),
            &[
                ("root", style::root),
                ("fill", style::fill),
                ("surface", style::surface),
                ("name", style::name),
                ("through", style::through),
                ("titled", style::titled),
                ("line", style::line),
            ],
        );
    }

    fn pointer_events(surface: &web_sys::Element) -> String {
        web_sys::window()
            .unwrap()
            .get_computed_style(surface)
            .unwrap()
            .unwrap()
            .get_property_value("pointer-events")
            .unwrap()
    }

    /// Rows in a column, each one whole-row tooltip.
    fn rows(count: usize) -> web_sys::Element {
        mount(move || {
            view! {
                <div style="width:400px">
                    {(0..count)
                        .map(|n| {
                            view! {
                                <Tooltip
                                    text=format!("row {n}")
                                    lines=vec![TooltipLine::new("A line"), TooltipLine::new("Another")]
                                    fill=true
                                    trigger=move |id| {
                                        view! {
                                            <a
                                                href="#"
                                                aria-describedby=id
                                                style="display:block; width:100%; height:24px"
                                            >
                                                {format!("Row {n}")}
                                            </a>
                                        }
                                            .into_any()
                                    }
                                />
                            }
                        })
                        .collect_view()}
                </div>
            }
        })
    }

    /// A whole row's surface lets the pointer through to the rows it covers; a
    /// mark's surface still takes it, so its words can be reached.
    #[wasm_bindgen_test]
    fn only_a_row_s_surface_lets_the_pointer_through() {
        unmount_earlier();
        tooltip_style();
        let row = rows(1);
        let mark = one_mark();
        assert_eq!(pointer_events(&surface(&row)), "none");
        assert_eq!(pointer_events(&surface(&mark)), "auto");
    }

    /// Open over the rows below it, a row's tooltip is not what the pointer
    /// lands on: the row under it is, and that row's words show at once.
    #[wasm_bindgen_test]
    async fn the_row_under_an_open_row_tooltip_takes_over() {
        unmount_earlier();
        tooltip_style();
        let el = rows(6);
        let wrappers = el.query_selector_all("[aria-describedby]").unwrap();
        let wrapper_at = |i| {
            wrappers
                .item(i)
                .unwrap()
                .unchecked_into::<web_sys::Element>()
                .parent_element()
                .unwrap()
        };
        let surfaces = el.query_selector_all("[role='tooltip']").unwrap();
        let surface_at = |i| {
            surfaces
                .item(i)
                .unwrap()
                .unchecked_into::<web_sys::Element>()
        };

        fire(&wrapper_at(0), "pointerenter");
        sleep_ms(600).await;
        let first = surface_at(0);
        assert!(first.matches(":popover-open").unwrap());

        let rect = first.get_bounding_client_rect();
        #[allow(clippy::cast_possible_truncation, reason = "a point on screen")]
        let under = web_sys::window()
            .unwrap()
            .document()
            .unwrap()
            .element_from_point(
                (rect.left() + rect.width() / 2.0) as f32,
                (rect.top() + rect.height() / 2.0) as f32,
            )
            .expect("something is under the tooltip");
        assert!(
            !first.contains(Some(&under)),
            "the pointer lands on the tooltip, not the row under it"
        );
        let row = under
            .closest("a")
            .unwrap()
            .expect("a row is under the tooltip");
        let next = (1..wrappers.length())
            .find(|&i| wrapper_at(i).contains(Some(&row)))
            .expect("the row under it is another row");

        fire(&wrapper_at(0), "pointerleave");
        fire(&wrapper_at(next), "pointerenter");
        leptos::task::tick().await;
        assert!(
            surface_at(next).matches(":popover-open").unwrap(),
            "no hand-over"
        );
        assert!(!first.matches(":popover-open").unwrap());

        fire(&wrapper_at(next), "pointerleave");
        sleep_ms(400).await;
    }

    /// A finger resting on a mark opens nothing: on touch the fact is already
    /// on the page, and a long press is the system's.
    #[wasm_bindgen_test]
    async fn a_touch_opens_nothing() {
        unmount_earlier();
        let el = one_mark();
        let init = web_sys::PointerEventInit::new();
        init.set_pointer_type("touch");
        let touch = web_sys::PointerEvent::new_with_event_init_dict("pointerenter", &init).unwrap();
        wrapper(&el).dispatch_event(&touch).unwrap();
        sleep_ms(600).await;
        assert!(!is_open(&el), "a touch opened the tooltip");
    }
}
