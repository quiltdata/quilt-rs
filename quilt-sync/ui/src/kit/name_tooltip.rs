//! The whole of a name a list had to cut short, on hover or keyboard focus.
//!
//! # One for the list, not one per row
//!
//! A row's name ends in an ellipsis when it does not fit, and the reader who
//! wants the rest rests the pointer on it. A [`Tooltip`](super::Tooltip) per
//! row would do that, and would keep a surface, a signal, a timer, an effect
//! and four listeners mounted under every row while hidden — thousands of each
//! on a long list, for the one name read at a time. So the list wraps its rows
//! once: it listens for the pointer and for focus where they bubble to, and
//! moves one surface to whichever name they reached.
//!
//! A row only marks its name with `data-full-name`, holding the whole of it —
//! the whole path, where the row shows only the part under a group heading.
//!
//! # Only when it is cut
//!
//! A name that fits says everything already, and a sentence repeating it is
//! noise. Whether it fits is measured when the pointer or focus arrives, not
//! when the row is drawn: the list's width changes with the window, and a
//! measurement per row on every change is the per-row cost this exists to
//! avoid.
//!
//! # It behaves as `Tooltip` does
//!
//! Same delay, same warm hand-over from one name to the next, same Escape,
//! pointer down, scroll and resize closing it, nothing on touch, one open at a
//! time across the app: it is the same [`TooltipHandle`] and the same surface.
//!
//! Keyboard focus opens it at once on a focusable name, the openable row's
//! button, and on a control whose `<label>` holds a name, the selectable row's
//! checkbox. The row's `[⋯]` and a group's disclosure are neither, so they open
//! nothing. A row that differs opens its `Differs` sentence on that same focus,
//! and that sentence wins: what is wrong with a file outranks its name.
//!
//! # No `aria-describedby`
//!
//! The ellipsis is drawn by CSS and cuts only what is painted: the name's text
//! is whole in the document, and is already what a reader hears for the row.
//! The surface is `aria-hidden`, as every tooltip's is, and nothing names it —
//! a description would read the name a second time.

use leptos::prelude::*;
use wasm_bindgen::JsCast;

use super::Align;
use super::TooltipHandle;
use super::tooltip::{style, surface};

/// What a name carries for this to find it.
const NAMED: &str = "[data-full-name]";

#[component]
pub fn NameTooltip(
    /// The list. Its names carry `data-full-name`.
    children: Children,
) -> impl IntoView {
    let handle = TooltipHandle::new();
    let text = RwSignal::new(String::new());
    let anchor = RwSignal::new_local(None::<web_sys::Element>);
    let id = handle.id();

    // The name an event's element belongs to. The surface counts as the name
    // it shows, so the pointer may cross onto it and stay.
    let name_at = move |target: Option<web_sys::EventTarget>| {
        let element = target?.dyn_into::<web_sys::Element>().ok()?;
        if element.closest(&format!("#{id}")).ok().flatten().is_some() {
            return anchor.get_untracked();
        }
        element.closest(NAMED).ok().flatten()
    };
    let name_at = StoredValue::new_local(name_at);

    // The name focus on `target` stands for: itself, or the one in its label.
    let focused_name = |target: Option<web_sys::EventTarget>| {
        let element = target?.dyn_into::<web_sys::Element>().ok()?;
        if element.matches(NAMED).unwrap_or(false) {
            return Some(element);
        }
        element
            .closest("label")
            .ok()
            .flatten()?
            .query_selector(NAMED)
            .ok()
            .flatten()
    };

    // Points the surface at `name` and opens it, if the name is cut; lets go
    // of what shows if not. Measured here, when it would open, and nowhere
    // else: a pointer crossing a column of names reads no width, and a width
    // read here is never stale.
    let open_on = move |name: web_sys::Element| {
        if name.scroll_width() <= name.client_width() {
            handle.leave();
            return;
        }
        text.set(name.get_attribute("data-full-name").unwrap_or_default());
        anchor.set(Some(name));
        handle.show();
    };

    let pointer_over = move |event: web_sys::PointerEvent| {
        if event.pointer_type() == "touch" {
            return;
        }
        let Some(name) = name_at.with_value(|at| at(event.target())) else {
            return;
        };
        // Back onto the name that shows, or onto its surface: it stays.
        if handle.is_open() && anchor.get_untracked().as_ref() == Some(&name) {
            handle.enter();
        } else {
            handle.enter_with(move || open_on(name));
        }
    };
    // Leaving a name for anything but itself or its surface: the name that
    // shows, or one still waiting out its delay.
    let pointer_out = move |event: web_sys::PointerEvent| {
        let (from, to) = name_at.with_value(|at| (at(event.target()), at(event.related_target())));
        let waiting_or_shown = !handle.is_open() || from == anchor.get_untracked();
        if from.is_some() && from != to && waiting_or_shown {
            handle.leave();
        }
    };
    let focus_in = move |event: web_sys::FocusEvent| {
        let Some(name) = focused_name(event.target()) else {
            return;
        };
        let keyboard = event
            .target()
            .and_then(|target| target.dyn_into::<web_sys::Element>().ok())
            .is_some_and(|element| element.matches(":focus-visible").unwrap_or(false));
        // The row's own handler ran first, as the event bubbled to the list:
        // a `Differs` it opened for this focus stays. One opened by a hover
        // elsewhere gives way, as any tooltip does to the next.
        if keyboard && !handle.another_opened_by(&event) {
            open_on(name);
        }
    };
    let focus_out = move |event: web_sys::FocusEvent| {
        if focused_name(event.target()).is_some_and(|name| anchor.get_untracked() == Some(name)) {
            handle.focus_out();
        }
    };
    view! {
        // No box of its own, so the list lays out as it would unwrapped: a
        // scroll box under it still fills its column. Inline, so that holds
        // wherever the list is drawn, stylesheet or not.
        <div
            style="display:contents"
            on:pointerover=pointer_over
            on:pointerout=pointer_out
            on:focusin=focus_in
            on:focusout=focus_out
        >
            {children()}
            {surface(
                handle,
                text.into(),
                Signal::derive_local(move || anchor.get()),
                Align::Start,
                format!("{} {}", style::surface, style::name),
            )}
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::{EntryAction, EntryRow, EntrySelection};
    use crate::test_support::{
        blur, element_saying, focus_report, keyboard_focus, load_module_style, mount, sleep_ms,
        unmount_earlier,
    };
    use wasm_bindgen_test::*;

    #[expect(dead_code, reason = "only the classes that cut a name are loaded")]
    mod rows {
        stylance::import_crate_style!(pub style, "src/kit/entry_row.module.scss");
    }
    use rows::style as row;

    mod tooltips {
        stylance::import_crate_style!(pub style, "src/kit/tooltip.module.scss");
    }
    use tooltips::style as tips;

    const LONG: &str = "raw/2026-10-15/plate-07/instrument-export/a-name-far-too-long-for-the-column-it-is-drawn-in.csv";

    /// The rows' own stylesheet, so a long name is cut as it is in the app.
    fn row_style() {
        load_module_style(
            include_str!("entry_row.module.scss"),
            &[
                ("root", row::root),
                ("main", row::main),
                ("name", row::name),
                ("open", row::open),
                ("state", row::state),
                ("size", row::size),
            ],
        );
    }

    /// Two rows in a column 480px wide: one whose name is cut, one whose
    /// name fits.
    fn two_rows(action: impl Fn() -> Option<EntryAction> + 'static) -> web_sys::Element {
        row_style();
        mount(move || {
            let row = |name: &'static str| match action() {
                Some(action) => {
                    view! { <EntryRow name=name size="1 kB" action=action plain=true /> }.into_any()
                }
                None => view! { <EntryRow name=name size="1 kB" plain=true /> }.into_any(),
            };
            let (long, short) = (row(LONG), row("a.csv"));
            view! {
                <div style="width:480px">
                    <NameTooltip>
                        <ul>
                            <li>{long}</li>
                            <li>{short}</li>
                        </ul>
                    </NameTooltip>
                </div>
            }
        })
    }

    fn fire(target: &web_sys::Element, kind: &str) {
        let init = web_sys::PointerEventInit::new();
        init.set_pointer_type("mouse");
        init.set_bubbles(true);
        let event = web_sys::PointerEvent::new_with_event_init_dict(kind, &init).unwrap();
        target.dispatch_event(&event).unwrap();
    }

    fn hover(name: &web_sys::Element) {
        fire(name, "pointerover");
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

    /// The list's surface: the last one, after any a row draws for itself.
    fn tooltip(el: &web_sys::Element) -> web_sys::Element {
        let all = el.query_selector_all("[role='tooltip']").unwrap();
        all.item(
            all.length()
                .checked_sub(1)
                .expect("the list has its surface"),
        )
        .unwrap()
        .unchecked_into()
    }

    fn showing(el: &web_sys::Element) -> Option<String> {
        let tip = tooltip(el);
        tip.matches(":popover-open")
            .unwrap()
            .then(|| tip.text_content().unwrap_or_default())
    }

    #[wasm_bindgen_test]
    async fn a_cut_name_shows_its_whole_path_after_the_delay() {
        unmount_earlier();
        let el = two_rows(|| None);
        let name = element_saying(&el, LONG);
        assert!(name.scroll_width() > name.client_width(), "the name is cut");

        hover(&name);
        sleep_ms(100).await;
        assert_eq!(showing(&el), None, "a pointer passing over opens nothing");
        sleep_ms(500).await;
        assert_eq!(showing(&el).as_deref(), Some(LONG));

        fire(&name, "pointerout");
        sleep_ms(250).await;
        assert_eq!(showing(&el), None, "still open after the pointer left");
        sleep_ms(200).await;
    }

    /// The width is read when it would open, not when the pointer arrived: a
    /// name that fits by then opens nothing, however it was when hovered.
    #[wasm_bindgen_test]
    async fn the_width_is_read_when_it_would_open() {
        unmount_earlier();
        let el = two_rows(|| None);
        let column: web_sys::HtmlElement = el.first_element_child().unwrap().unchecked_into();
        let name = element_saying(&el, LONG);

        hover(&name);
        sleep_ms(100).await;
        column.style().set_property("width", "4000px").unwrap();
        sleep_ms(500).await;
        assert_eq!(showing(&el), None, "a name that fits by then opened it");

        // Cut again, and the same name opens on its next hover.
        fire(&name, "pointerout");
        column.style().set_property("width", "480px").unwrap();
        sleep_ms(200).await;
        hover(&name);
        sleep_ms(600).await;
        assert_eq!(showing(&el).as_deref(), Some(LONG));
        escape();
    }

    /// A differing row opens its `Differs` sentence on keyboard focus, and the
    /// list leaves it there rather than covering it with the name.
    #[wasm_bindgen_test]
    async fn keyboard_focus_on_a_differing_row_keeps_its_differs_sentence() {
        use crate::kit::DIFFERS_TITLE;
        let actions: [fn() -> EntryAction; 2] = [
            || EntryAction::Select(EntrySelection::new(false, Callback::new(|_| {}))),
            || EntryAction::Open(Callback::new(|()| {})),
        ];
        for action in actions {
            unmount_earlier();
            row_style();
            let el = mount(move || {
                view! {
                    <div style="width:480px">
                        <NameTooltip>
                            <EntryRow name=LONG size="1 kB" action=action() differs=true plain=true />
                        </NameTooltip>
                    </div>
                }
            });
            let control = el
                .query_selector("input[type=checkbox], button[aria-describedby]")
                .unwrap()
                .expect("the row's control");
            keyboard_focus(&control);
            leptos::task::tick().await;
            let open = el
                .query_selector_all("[role='tooltip']:popover-open")
                .unwrap();
            assert_eq!(open.length(), 1, "{}", focus_report(&control));
            assert_eq!(
                open.item(0).unwrap().text_content().as_deref(),
                Some(DIFFERS_TITLE),
                "the name covered the Differs sentence"
            );
            blur(&control);
            leptos::task::tick().await;
        }
    }

    /// A `Differs` sentence opened by hovering one row does not hold back
    /// keyboard focus on another row's cut name.
    #[wasm_bindgen_test]
    async fn a_hovered_differs_elsewhere_gives_way_to_a_focused_name() {
        unmount_earlier();
        row_style();
        let el = mount(|| {
            view! {
                <div style="width:480px">
                    <NameTooltip>
                        <EntryRow name="a.csv" size="1 kB" differs=true plain=true />
                        <EntryRow
                            name=LONG
                            size="1 kB"
                            action=EntryAction::Open(Callback::new(|()| {}))
                            plain=true
                        />
                    </NameTooltip>
                </div>
            }
        });
        // The row's surface comes first; the list's is last.
        let differs = el.query_selector("[role='tooltip']").unwrap().unwrap();
        let init = web_sys::PointerEventInit::new();
        init.set_pointer_type("mouse");
        let enter = web_sys::PointerEvent::new_with_event_init_dict("pointerenter", &init).unwrap();
        differs
            .parent_element()
            .unwrap()
            .dispatch_event(&enter)
            .unwrap();
        sleep_ms(600).await;
        assert!(differs.matches(":popover-open").unwrap(), "Differs opened");

        let name = element_saying(&el, LONG);
        keyboard_focus(&name);
        leptos::task::tick().await;
        assert_eq!(
            showing(&el).as_deref(),
            Some(LONG),
            "{}",
            focus_report(&name)
        );
        assert!(
            !differs.matches(":popover-open").unwrap(),
            "two tooltips open"
        );
        escape();
        blur(&name);
    }

    /// A path has no spaces to wrap at. The surface wraps it anywhere rather
    /// than scrolling it sideways.
    #[wasm_bindgen_test]
    async fn a_long_path_wraps_in_its_tooltip() {
        unmount_earlier();
        load_module_style(
            include_str!("tooltip.module.scss"),
            &[
                ("root", tips::root),
                ("surface", tips::surface),
                ("name", tips::name),
            ],
        );
        // No hyphen or space: nowhere a browser would wrap on its own.
        let path = "raw/2026_10_15/plate_07/instrument_export/\
                    a_name_far_too_long_for_the_column_it_is_drawn_in_and_for_its_tooltip.csv";
        row_style();
        let el = mount(move || {
            view! {
                <div style="width:480px">
                    <NameTooltip>
                        <EntryRow name=path size="1 kB" plain=true />
                    </NameTooltip>
                </div>
            }
        });
        hover(&element_saying(&el, path));
        sleep_ms(600).await;
        assert_eq!(showing(&el).as_deref(), Some(path));
        let tip = tooltip(&el);
        assert!(
            tip.scroll_width() <= tip.client_width(),
            "the path scrolls sideways: {} wide in {}",
            tip.scroll_width(),
            tip.client_width()
        );
        escape();
    }

    #[wasm_bindgen_test]
    async fn a_name_that_fits_shows_nothing() {
        unmount_earlier();
        let el = two_rows(|| None);
        hover(&element_saying(&el, "a.csv"));
        sleep_ms(600).await;
        assert_eq!(showing(&el), None);
    }

    /// The grouped list shows only the part under the heading, and the
    /// tooltip says the whole path.
    #[wasm_bindgen_test]
    async fn a_cut_leaf_shows_the_whole_path() {
        unmount_earlier();
        row_style();
        let leaf =
            "instrument-export-of-plate-07-a-name-far-too-long-for-the-column-of-this-list.csv";
        let path = format!("raw/2026/{leaf}");
        let el = mount(move || {
            view! {
                <div style="width:480px">
                    <NameTooltip>
                        <EntryRow name=leaf path=path.clone() size="1 kB" plain=true />
                    </NameTooltip>
                </div>
            }
        });
        hover(&element_saying(&el, leaf));
        sleep_ms(600).await;
        assert_eq!(
            showing(&el).as_deref(),
            Some(format!("raw/2026/{leaf}").as_str())
        );
        escape();
    }

    #[wasm_bindgen_test]
    async fn keyboard_focus_on_an_openable_name_opens_it_at_once_and_escape_closes_it() {
        unmount_earlier();
        let el = two_rows(|| Some(EntryAction::Open(Callback::new(|()| {}))));
        let name = element_saying(&el, LONG);
        keyboard_focus(&name);
        leptos::task::tick().await;
        assert_eq!(
            showing(&el).as_deref(),
            Some(LONG),
            "{}",
            focus_report(&name)
        );

        // Escape is heard from the frame after it opens.
        sleep_ms(50).await;
        escape();
        leptos::task::tick().await;
        assert_eq!(showing(&el), None);
        blur(&name);
    }

    /// A selectable row's focus lands on its box, and the name is in the
    /// box's label.
    #[wasm_bindgen_test]
    async fn keyboard_focus_on_a_row_s_box_opens_its_name() {
        unmount_earlier();
        let el = two_rows(|| {
            Some(EntryAction::Select(EntrySelection::new(
                false,
                Callback::new(|_| {}),
            )))
        });
        let boxes = el.query_selector_all("input[type=checkbox]").unwrap();
        let first: web_sys::Element = boxes.item(0).unwrap().unchecked_into();
        keyboard_focus(&first);
        leptos::task::tick().await;
        assert_eq!(
            showing(&el).as_deref(),
            Some(LONG),
            "{}",
            focus_report(&first)
        );
        blur(&first);
        leptos::task::tick().await;
        assert_eq!(showing(&el), None, "blur left it open");

        let second: web_sys::Element = boxes.item(1).unwrap().unchecked_into();
        keyboard_focus(&second);
        leptos::task::tick().await;
        assert_eq!(showing(&el), None, "a name that fits opened it");
        blur(&second);
    }

    #[wasm_bindgen_test]
    fn a_thousand_rows_mount_one_surface() {
        unmount_earlier();
        let el = mount(|| {
            view! {
                <NameTooltip>
                    <ul>
                        {(0..1000)
                            .map(|i| {
                                view! {
                                    <li>
                                        <EntryRow name=format!("data/{i}.csv") size="1 kB" />
                                    </li>
                                }
                            })
                            .collect_view()}
                    </ul>
                </NameTooltip>
            }
        });
        assert_eq!(el.query_selector_all("li").unwrap().length(), 1000);
        assert_eq!(
            el.query_selector_all("[role='tooltip']").unwrap().length(),
            1
        );
        assert_eq!(el.query_selector_all("[popover]").unwrap().length(), 1);
    }
}
