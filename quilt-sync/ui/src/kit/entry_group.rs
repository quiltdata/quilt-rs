//! A heading and the rows under it, collapsible.
//!
//! # Not `GroupHeading`
//!
//! That one is a flat heading emitted *between* runs of rows, and the pages using
//! it want it to stay that way. This is a container: it holds its rows, owns
//! whether they are shown, and carries a box that ticks all of them. Different
//! contract, different component.
//!
//! # A collapsed group renders nothing
//!
//! `<Show>` and not `display: none`. The 1000-entry cap is argued partly on
//! grouping sparing the DOM, and a hidden subtree is still built — so hiding with
//! CSS would have made that argument false while looking identical.
//!
//! # A group with nothing to select carries no box
//!
//! Only a downloadable row has a checkbox, so a group whose files are all
//! present has none to tick. Its heading box would be a control that cannot act
//! — the dead control [`Select`](super::Select) already refuses to be. Pass no
//! [`GroupSelection`] and the column stays open so the names still line up, but
//! nothing is drawn in it.
//!
//! # The gutter is the list's
//!
//! The disclosure sits in `--q-entry-gutter`, the same column the rows leave
//! empty. A list that renders no group at all sets that value to `0` — see
//! [`EntryRow`](super::EntryRow) — which is why the width lives on the list
//! rather than in here.
//!
//! # The heading cannot be a `<label>`
//!
//! It holds the disclosure `<button>`, and a `<label>` may not contain another
//! labelable element. So the box names itself — this is the call site
//! [`Checkbox`](super::Checkbox)'s `aria_label` exists for, and the name has to
//! say which group, because a list of them all reading `Select all` names
//! nothing.
//!
//! # A closed group still says a row differs
//!
//! In resolve mode a row that differs is marked, and a collapsed group draws no
//! rows, so `differs` puts a small mark on the heading instead: collapsing never
//! hides a difference. Open, the rows carry it and the heading does not. It is
//! said as the rows' mark is: a hidden sentence for reading, and, for the
//! pointer and the keyboard, a [`Tooltip`](super::Tooltip) — hung from the
//! dot, and opened as well by the disclosure, which is the heading's one
//! control that is always there. The disclosure names both the tooltip and
//! [`DIFFERS_ID`], the pane's count.
//!
//! # A heading summarises its rows
//!
//! The way the select-all slot summarises the list: a tri-state box while any
//! row under it has a box, and, when every row under it is here, `have_mark`
//! puts the rows' muted check in the hole. It pays most closed — a collapsed
//! heading with a check says *all of this is here* without being opened, and
//! one with a box says *something in here is still to fetch*. Muted, not the
//! Success tone: a summary of a subset is bookkeeping, and the green tick stays
//! the one package-level statement on the screen. Like the rows' check it is a
//! statement and never a control, and it is ignored on a heading that has a
//! box.

use leptos::prelude::*;

use super::CheckState;
use super::Checkbox;
use super::Tooltip;
use super::TooltipHandle;
use super::differs_id;
use super::icons;

stylance::import_crate_style!(style, "src/kit/entry_group.module.scss");

/// What a closed heading holding a row that differs says, in words and in its
/// mark's tooltip.
const DIFFERS_GROUP_TEXT: &str = "Contains files that differ";

/// A group's tick: where its selectable rows stand, and what to do when the
/// heading box moves.
///
/// A group has one exactly when at least one row under it can be downloaded.
/// Same shape as [`EntrySelection`](super::EntrySelection), and for the same
/// reason: there is no way to say "selectable" without saying what it means.
#[derive(Clone, Copy)]
pub struct GroupSelection {
    pub state: Signal<CheckState>,
    pub on_toggle: Callback<bool>,
    /// The box is shown but cannot move, as while a download runs.
    pub disabled: Signal<bool>,
}

impl GroupSelection {
    #[must_use]
    pub fn new(state: impl Into<Signal<CheckState>>, on_toggle: Callback<bool>) -> Self {
        Self {
            state: state.into(),
            on_toggle,
            disabled: Signal::stored(false),
        }
    }

    /// Hold the box still while `disabled` is true.
    #[must_use]
    pub fn disabled(self, disabled: impl Into<Signal<bool>>) -> Self {
        Self {
            disabled: disabled.into(),
            ..self
        }
    }
}

#[component]
pub fn EntryGroup(
    /// The folder this run of rows shares.
    #[prop(into)]
    name: String,
    /// How many rows follow. Always shown, including one — "1" is information,
    /// and hiding it would make a single-row group look like a header with a bug.
    #[prop(into)]
    count: Signal<usize>,
    open: RwSignal<bool>,
    /// Present when at least one row under this heading can be downloaded.
    /// Absent draws no box at all.
    #[prop(optional)]
    selection: Option<GroupSelection>,
    /// A row under this heading differs between the revisions. Marked on the
    /// heading only while it is closed; open, the rows carry the mark.
    #[prop(optional)]
    differs: bool,
    /// Every row under this heading is here, so the box-shaped hole holds the
    /// rows' muted check rather than a blank. Ignored when there is a
    /// `selection`: the box is the hole's only occupant.
    #[prop(optional)]
    have_mark: bool,
    children: ChildrenFn,
) -> impl IntoView {
    let full_name = name.clone();
    let box_label = format!("Select all in {name}");
    let hides_difference = move || differs && !open.get();
    let differs_id = differs_id();
    // The dot is drawn only while closed, and the disclosure always: so the
    // state is made out here and outlives the dot's tooltip, which the `Show`
    // rebuilds each time the group closes.
    let tip = TooltipHandle::new();

    view! {
        <div class=style::root>
            <div class=style::head>
                <button
                    type="button"
                    class=style::disclose
                    aria-expanded=move || open.get().to_string()
                    // Names what it does, not what it looks like. The glyph is
                    // `aria-hidden`, so without this the button announces nothing.
                    aria-label=move || {
                        if open.get() { "Collapse group" } else { "Expand group" }
                    }
                    aria-describedby=move || {
                        hides_difference().then(|| format!("{} {differs_id}", tip.id()))
                    }
                    // Only while the dot is there to explain; open, the rows
                    // carry their own marks and their own sentences.
                    on:focusin=move |event| {
                        if hides_difference() {
                            tip.focus_in(&event);
                        }
                    }
                    on:focusout=move |_| tip.focus_out()
                    on:click=move |_| open.update(|o| *o = !*o)
                >
                    {move || if open.get() { icons::chevron_down() } else { icons::chevron_right() }}
                </button>
                {match selection {
                    Some(GroupSelection { state, on_toggle, disabled }) => {
                        view! {
                            <Checkbox
                                state=state
                                on_toggle=move |next| on_toggle.run(next)
                                aria_label=box_label
                                disabled=disabled
                            />
                        }
                            .into_any()
                    }
                    // The rows' check, summarised: a statement, never a control.
                    None if have_mark => {
                        view! {
                            <span class=format!(
                                "{} {}",
                                style::nobox,
                                style::mark,
                            )>{icons::check()}</span>
                        }
                            .into_any()
                    }
                    None => view! { <span class=style::nobox /> }.into_any(),
                }}
                <span class=style::name title=full_name>{name}</span>
                <Show when=hides_difference>
                    <Tooltip
                        handle=tip
                        text=DIFFERS_GROUP_TEXT.to_string()
                        trigger=|_| {
                            view! {
                                <span class=style::differs>
                                    <span data-sr-only>{DIFFERS_GROUP_TEXT}</span>
                                </span>
                            }
                                .into_any()
                        }
                    />
                </Show>
                <span class=style::count>{move || count.get()}</span>
            </div>
            <Show when=move || open.get()>{children()}</Show>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{
        blur, describing_tooltip, focus_report, keyboard_focus, mount, unmount_earlier,
    };
    use wasm_bindgen_test::*;

    /// The check lives in the hole, so it appears only where there is one: a
    /// heading with a box has none, and a mark asked of it is dropped. Without
    /// the prop the hole stays blank.
    #[wasm_bindgen_test]
    fn a_mark_fills_the_hole_and_only_the_hole() {
        let slot = format!(".{} svg", style::nobox);

        let here = mount(|| {
            view! {
                <EntryGroup name="notes/" count=6 open=RwSignal::new(false) have_mark=true>
                    <span />
                </EntryGroup>
            }
        });
        assert!(
            here.query_selector(&slot).unwrap().is_some(),
            "a heading whose rows are all here draws the check in its hole; markup was {}",
            here.inner_html()
        );

        let picking = mount(|| {
            view! {
                <EntryGroup
                    name="raw/"
                    count=2
                    open=RwSignal::new(false)
                    selection=GroupSelection::new(CheckState::Off, Callback::new(|_| ()))
                    have_mark=true
                >
                    <span />
                </EntryGroup>
            }
        });
        assert!(
            picking
                .query_selector("input[type=checkbox]")
                .unwrap()
                .is_some(),
            "a heading with a selection draws its box; markup was {}",
            picking.inner_html()
        );
        assert!(
            picking.query_selector(&slot).unwrap().is_none(),
            "a heading with a box has no hole and draws no check; markup was {}",
            picking.inner_html()
        );

        let blank = mount(|| {
            view! {
                <EntryGroup name="raw/" count=2 open=RwSignal::new(false)>
                    <span />
                </EntryGroup>
            }
        });
        let hole = blank
            .query_selector(&format!(".{}", style::nobox))
            .unwrap()
            .expect("a heading with neither keeps its hole");
        assert!(
            hole.child_element_count() == 0,
            "a heading not asked for the mark keeps its hole blank; markup was {}",
            blank.inner_html()
        );
    }

    /// Closed, a heading hides its rows' marks, so its disclosure button is
    /// described by the dot's tooltip and opens it from the keyboard. Open, the
    /// rows speak for themselves and the button names nothing.
    #[wasm_bindgen_test]
    async fn a_closed_differing_heading_s_button_is_described_by_the_tooltip() {
        unmount_earlier();
        let open = RwSignal::new(false);
        let el = mount(move || {
            view! {
                <EntryGroup name="raw/" count=2 open=open differs=true>
                    <span />
                </EntryGroup>
            }
        });
        let disclose = el
            .query_selector("button[aria-expanded]")
            .unwrap()
            .expect("the disclosure button");
        let tip = describing_tooltip(&disclose).expect("the closed heading names its tooltip");
        assert_eq!(tip.text_content().unwrap(), DIFFERS_GROUP_TEXT);

        keyboard_focus(&disclose);
        leptos::task::tick().await;
        assert!(
            tip.matches(":popover-open").unwrap(),
            "keyboard focus opened nothing; {}",
            focus_report(&disclose)
        );
        blur(&disclose);
        leptos::task::tick().await;

        open.set(true);
        leptos::task::tick().await;
        assert!(
            describing_tooltip(&disclose).is_none(),
            "an open heading still names the dot's tooltip"
        );
    }
}
