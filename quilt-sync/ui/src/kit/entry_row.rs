//! One file in an installed package.
//!
//! # Not a `FileRow`
//!
//! That one is *a recently-changed file* for the cross-package feed: it carries
//! the package it belongs to, which a page about one package does not need, and
//! its row opens the file. This one chooses instead — **the checkbox is the
//! control and the name is not a button** — because this page's verb is picking
//! files in bulk, not acting on one.
//!
//! # The resting state is silent
//!
//! `Downloaded` is what most of a seven-hundred-row list is, and printing it
//! seven hundred times spends the reader's attention on the one thing that needs
//! none. Absence says the file is here. Pass no `state` for it — and none for an
//! ignored file either, for a different reason: the `Ignored` facet is the only
//! view that shows those, so every row in it is ignored and the label would
//! repeat what the facet already said.
//!
//! # A click does the thing this row can do
//!
//! Where the file is decides both the box and the gesture, so they are **one
//! value** — [`EntryAction`] — and not a flag beside two optional halves:
//!
//! - **not downloaded** → [`EntryAction::Select`]: it carries a box, and a click
//!   anywhere on the row ticks it. There is no local file to open, and choosing
//!   is what the page is for.
//! - **downloaded** → [`EntryAction::Open`]: no box, and a click opens the local
//!   file — the recent-files list's own gesture, so the two file lists in this
//!   app mean the same thing by a click.
//! - **neither** → no action at all: a file that is here but that the caller will
//!   not open, such as a deleted or ignored one. No box, no pointer, nothing to
//!   click. A row that advertises a click it cannot honour is worse than an
//!   inert one.
//!
//! A tick on a file that is already here has nothing to act on, and it is what
//! made *select all 56* disagree with *download 17*. A row without a box keeps
//! the column's width so the names still line up.
//!
//! A caller whose row's file is on disk passes `have_mark`, and the hole holds
//! a muted check: *have it*. The rule is per row, in every scope and state: a
//! file that is here carries the check, and one that is not carries a box when
//! it can be picked, otherwise nothing. Were the check drawn only while some
//! row has a box, the column would flip from ticks to blank the moment the
//! last file landed, and the finished list would look like another design
//! rather than the completed one; per row, the column simply fills up. A
//! uniform column carries no information, so a thousand muted ticks stay
//! quiet — it is unevenness that draws the eye.
//!
//! The check is a statement and never a control: no hit target of its own,
//! and ignored on a row that has a box, since the box already says everything
//! a mark could. The glyph is the kit's choice, made once here: an empty,
//! disabled box would say *not picked*, which in this list means *not
//! downloaded*, and a download arrow would invite the click it is reporting.
//!
//! One value rather than two optional props is the same discipline
//! [`CheckState`](super::CheckState) follows: a box that accepts clicks and
//! discards them cannot be built, and neither can a row that both selects and
//! opens.
//!
//! # The destination never depends on state
//!
//! `Open` is always the **local** file. Opening in the catalog leaves the
//! application, and that is a named command in the `[⋯]`, never something a
//! click infers — v1 splits the same way and is legible because it splits by
//! which control is present, not by what a gesture guesses.
//!
//! # The gutter is the list's, and it can be nothing
//!
//! The empty column before the checkbox is the width of a group's disclosure
//! button, so a file's box lands under its group's box rather than a triangle's
//! width to the left of it. It comes from `--q-entry-gutter`, which the list
//! sets once for its rows, its headings and its select-all.
//!
//! **A list with no headings at all sets it to `0`** — a package whose files are
//! all at the root, or any package under `Group: None`, which is a control the
//! reader can reach at any time. There is no triangle anywhere in such a list,
//! so the column has nothing to align to and is simply an indent nobody asked
//! for. The value exists to keep three components honest; when there is nothing
//! to be honest about, it is zero.
//!
//! # A row that differs says so twice, and explains itself on request
//!
//! A row can carry two states, on two independent axes. One is where the file
//! is — downloaded, `Changed`, `Not downloaded`, `Deleted`. The other is
//! whether the two revisions agree about it, which only resolve mode asks, and
//! whose one answer worth drawing is `Differs`. So `Differs` is a
//! [`StateLabel`] of its own, in the Danger tone, beside the place's label and
//! never instead of it: a file that is deleted and differs shows both.
//!
//! It is said in two channels. The rule and the tint are colour; the label's
//! word and its × are not, and survive greyscale. The resolve pane's sentence
//! counts the set, and a [`Tooltip`](super::Tooltip) says what differs.
//!
//! The label shares the state slot with the place's, before it, and the two
//! stack right-aligned like any pair of labels. The slot is right-aligned and
//! at least one label wide, so sizes stay one column whether or not a row
//! carries two — no row reserves anything, in or out of resolve mode, and only
//! a marked row's name gives up the width.
//!
//! Rest the pointer on `Differs`, or reach the row's control from the
//! keyboard, and the tooltip's sentence appears. The control names it in
//! `aria-describedby`, beside the pane's sentence. The label's word is real
//! text, so a reader moving through the list hears `Differs` where a sighted
//! reader sees it, and both have the pane's sentence — no hidden copy of the
//! row's sentence is needed. An inert
//! row has no control and never gets one — a tab stop that does nothing is
//! worse than none — so its sentence is hover-only, which is why the word has
//! to carry the meaning without it.
//!
//! The row itself has no `title`. It had one, and the name's own `title` (the
//! whole path) covered most of the row and won, so the hint showed only over
//! the gutter and the size. An element with a tooltip carries no `title` of its
//! own, or the browser's would sit on top of ours.
//!
//! # The `<label>` stops before the overflow, and only a selectable row has one
//!
//! Clicking a selectable row toggles its box, which wants a `<label>` around the
//! row — but `<button>` is a labelable element, and one inside a `<label>` that
//! is not its control is invalid. So the label covers box, name, state and size,
//! and the menu is its sibling.
//!
//! An openable row is not a label at all. Its **name is a real `<button>`**, as
//! [`FileRow`](super::FileRow)'s is, so the platform turns Enter and Space into a
//! click. The row carries the click too, as a pointer convenience — but only up
//! to the name, state and size: the handler is on the row's inner area and the
//! `[⋯]` is that area's sibling, so reaching for the menu cannot open the file.

use leptos::prelude::*;

use super::ActionMenu;
use super::Checkbox;
use super::MenuAction;
use super::StateLabel;
use super::Tooltip;
use super::TooltipHandle;
use super::icons;
use super::state_label::StateTone;

stylance::import_crate_style!(style, "src/kit/entry_row.module.scss");

/// A row's tick: where it stands, and what to do when it moves.
///
/// A row is selectable exactly when it has one of these. There is no way to say
/// "selectable" without saying what that means.
#[derive(Clone, Copy)]
pub struct EntrySelection {
    pub selected: Signal<bool>,
    pub on_toggle: Callback<bool>,
    /// The box is shown but cannot move, as while a download runs.
    pub disabled: Signal<bool>,
}

impl EntrySelection {
    #[must_use]
    pub fn new(selected: impl Into<Signal<bool>>, on_toggle: Callback<bool>) -> Self {
        Self {
            selected: selected.into(),
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

/// What this row's click does — which is a consequence of where the file is,
/// so the box and the gesture are decided together and cannot disagree.
#[derive(Clone, Copy)]
pub enum EntryAction {
    /// Not downloaded: draw a box, and let a click anywhere on the row tick it.
    Select(EntrySelection),
    /// Downloaded: draw no box, and open the **local** file on a click. Never
    /// the catalog — leaving the application is a named command, not an
    /// inference.
    Open(Callback<()>),
}

/// What a marked row's tooltip says, explaining its `Differs`. One sentence,
/// in the page's own words — no `remote`, no `diverged`, and no platform named as the
/// other place.
pub const DIFFERS_TITLE: &str =
    "Your version of this file and the published version have different contents.";

/// The id of the resolve pane's sentence counting the marked rows, which each
/// marked row names as its description.
pub const DIFFERS_ID: &str = "resolve-differing";

/// Another id for that sentence, for everything drawn inside the context that
/// provides it.
///
/// The app draws one resolve sentence per page, so it provides nothing and
/// keeps `DIFFERS_ID`. The gallery draws several on one page, one per cell that
/// marks rows, and a fixed id there is several elements with one id: each
/// cell's rows would be described by whichever sentence the browser found
/// first. Each such cell provides its own, so its rows and its sentence agree.
#[derive(Clone, Copy, Debug)]
pub struct DiffersId(pub &'static str);

/// The resolve sentence's id here: the provided `DiffersId`, or `DIFFERS_ID`.
#[must_use]
pub fn differs_id() -> &'static str {
    use_context::<DiffersId>().map_or(DIFFERS_ID, |id| id.0)
}

#[component]
pub fn EntryRow(
    /// What to show. The caller decides whether that is the whole path or the
    /// leaf under a group heading; this row only truncates it, at the end.
    #[prop(into)]
    name: String,
    /// The whole path, for the `title`, when `name` shows only part of it.
    /// Absent means `name` already is the whole path.
    #[prop(optional, into)]
    path: Option<String>,
    /// The state's words, or nothing at all for a resting state.
    ///
    /// A `MaybeProp` and not an `Option`: a caller enumerating the states — the
    /// page's own list does — computes this rather than writing it, and an
    /// `optional` prop cannot be handed a `None` it worked out. It would have to
    /// branch on presence and repeat the whole call.
    #[prop(optional, into)]
    state: MaybeProp<String>,
    /// How loudly. Ignored when `state` is absent.
    #[prop(optional)]
    tone: StateTone,
    /// Already formatted — the kit has no opinion about units.
    #[prop(into)]
    size: String,
    /// What a click does, and therefore whether a box is drawn. Absent is a row
    /// that can do neither — a deleted or ignored file — which keeps the
    /// column's width, draws nothing in it and takes no pointer.
    #[prop(optional)]
    action: Option<EntryAction>,
    /// The two revisions disagree about this file: a second state, on the
    /// comparison's axis, drawn as its own `Differs` label beside the place's.
    /// **Never a control** — resolution happens at revision level, so there is
    /// nothing to click here.
    #[prop(optional)]
    differs: bool,
    /// The row's `[⋯]`. Empty means no menu at all rather than an empty one.
    /// A signal, so an item can be disabled and released under an open menu;
    /// whether there is a menu is decided once, from the items it starts with.
    #[prop(optional, into)]
    actions: Signal<Vec<MenuAction>>,
    /// The file is here, so the box-shaped hole holds a muted check rather
    /// than a blank — whatever the rest of the list is. Ignored on a selectable
    /// row: the box is its only occupant.
    #[prop(optional)]
    have_mark: bool,
) -> impl IntoView {
    let full_name = path.unwrap_or_else(|| name.clone());

    let class = if differs {
        format!("{} {}", style::root, style::differs)
    } else {
        String::from(style::root)
    };

    // Made here rather than by the tooltip, because the label it hangs from and
    // the control that focus lands on are two elements: the control names the
    // surface and forwards its focus, the label is what a pointer rests on.
    let tip = differs.then(TooltipHandle::new);
    let differs_id = differs_id();
    // The control's description: the row's own sentence first, then the
    // pane's count of the set. A list, so neither replaces the other.
    let described = tip.map(|tip| format!("{} {differs_id}", tip.id()));
    let focus_in = move |event: web_sys::FocusEvent| {
        if let Some(tip) = tip {
            tip.focus_in(&event);
        }
    };
    let focus_out = move |_: web_sys::FocusEvent| {
        if let Some(tip) = tip {
            tip.focus_out();
        }
    };

    // The state and the size are the same in all three shapes, and building them
    // once keeps the arms about the one thing that actually differs.
    let trailing = move || {
        view! {
            // A fixed slot, so a size lands in the same column whether or not
            // the row above carries a label. Sizes exist to be compared, and
            // ragged ones cannot be. A row that differs puts `Differs` in the
            // same slot, before the place's label: the two stack right-aligned
            // like any labels, and the slot grows for them.
            <span class=style::state>
                {tip.map(differs_label)}
                {move || {
                    state.get().map(|words| view! { <StateLabel tone=tone>{words}</StateLabel> })
                }}
            </span>
            <span class=style::size>{size}</span>
        }
    };

    // Empty, and exactly a disclosure button wide, so a file's box sits under
    // its group's box rather than a triangle's width to the left of it.
    let gutter = move || view! { <span class=style::gutter /> };

    let main = match action {
        // Selectable: the whole row is the box's label, which is what makes a
        // click anywhere on it a tick.
        Some(EntryAction::Select(EntrySelection {
            selected,
            on_toggle,
            disabled,
        })) => view! {
            <label class=style::main on:focusin=focus_in on:focusout=focus_out>
                {gutter()}
                <Checkbox
                    state=Signal::derive(move || selected.get().into())
                    on_toggle=move |next| on_toggle.run(next)
                    disabled=disabled
                    aria_describedby=described.clone()
                />
                <span class=style::name title=full_name>{name}</span>
                {trailing()}
            </label>
        }
        .into_any(),
        // Openable: not a label — there is no control to label. The name is the
        // button, so Enter and Space are the platform's, and its click bubbles to
        // the row, which is the one handler.
        Some(EntryAction::Open(on_open)) => view! {
            <div
                class=style::main
                on:click=move |_| on_open.run(())
                on:focusin=focus_in
                on:focusout=focus_out
            >
                {gutter()}
                {nobox(have_mark)}
                <button
                    type="button"
                    class=style::open
                    title=full_name
                    aria-describedby=described.clone()
                >
                    {name}
                </button>
                {trailing()}
            </div>
        }
        .into_any(),
        // Neither. Inert, and says so: no pointer, no hover target.
        None => view! {
            <div class=format!("{} {}", style::main, style::inert)>
                {gutter()}
                {nobox(have_mark)}
                <span class=style::name title=full_name>{name}</span>
                {trailing()}
            </div>
        }
        .into_any(),
    };

    view! {
        <div
            class=class
            // Kept on the row as well as its control: an inert row has no
            // control, and this is the one element it has to name the pane's
            // sentence from.
            aria-describedby=differs.then_some(differs_id)
        >
            {main}
            {if actions.with_untracked(Vec::is_empty) {
                // A menu-shaped hole, for the same reason a boxless row keeps a
                // box-shaped one: without it the sizes in a list where one row has
                // no actions stop being a column.
                view! { <span class=style::nomenu /> }.into_any()
            } else {
                view! { <ActionMenu aria_label="More actions for this file" actions=actions /> }
                    .into_any()
            }}
        </div>
    }
}

/// The `Differs` label, with its tooltip.
///
/// Inside `main` like the place's label beside it, so a click on it does what
/// a click on that one does — ticks or opens — and it is never a control of its
/// own. The tooltip's surface is inside too, which is safe only because the
/// surface keeps itself out of the accessibility tree: on a selectable row
/// `main` is the `<label>`, and its words would otherwise join the checkbox's
/// name.
fn differs_label(tip: TooltipHandle) -> AnyView {
    view! {
        <span class=style::compared>
            <Tooltip
                handle=tip
                text=DIFFERS_TITLE.to_string()
                trigger=|_| {
                    view! { <StateLabel tone=StateTone::Danger>"Differs"</StateLabel> }.into_any()
                }
            />
        </span>
    }
    .into_any()
}

/// The hole a row without a box keeps, empty or holding the check.
fn nobox(have_mark: bool) -> AnyView {
    if have_mark {
        view! {
            <span class=format!("{} {}", style::nobox, style::mark)>{icons::check()}</span>
        }
        .into_any()
    } else {
        view! { <span class=style::nobox /> }.into_any()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::mount;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::*;

    fn click(el: &web_sys::Element) {
        let el: web_sys::HtmlElement = el.clone().dyn_into().unwrap();
        el.click();
    }

    #[wasm_bindgen_test]
    fn a_selectable_row_is_a_label_so_the_whole_row_ticks() {
        let ticked = RwSignal::new(false);
        let el = mount(move || {
            view! {
                <EntryRow
                    name="raw/plate-07.csv"
                    state="Not downloaded"
                    size="4.1 MB"
                    action=EntryAction::Select(
                        EntrySelection::new(ticked, Callback::new(move |n| ticked.set(n))),
                    )
                />
            }
        });
        let main = el.query_selector("label").unwrap();
        assert!(
            main.is_some(),
            "a selectable row's main area is a `<label>`"
        );
        assert!(
            el.query_selector("button").unwrap().is_none(),
            "the name is not a button when the row selects",
        );
        click(&main.unwrap());
        assert!(ticked.get_untracked(), "clicking the row ticked its box");
    }

    #[wasm_bindgen_test]
    fn an_openable_row_opens_from_the_row_and_from_its_name_button() {
        // A signal, not an `Rc<Cell>`: `Callback` is `Send + Sync`.
        let opened = RwSignal::new(0_u32);
        let el = mount(move || {
            view! {
                <EntryRow
                    name="notes/kickoff-thread.md"
                    size="12 KB"
                    action=EntryAction::Open(Callback::new(move |()| {
                        opened.update(|n| *n += 1);
                    }))
                />
            }
        });

        // The name is a real button, so the platform owns Enter and Space.
        let name = el
            .query_selector("button")
            .unwrap()
            .expect("name is a button");
        assert_eq!(name.text_content().unwrap(), "notes/kickoff-thread.md");
        click(&name);
        assert_eq!(opened.get_untracked(), 1, "the name button opened the file");

        // And the row itself is a pointer target, which is the one handler the
        // button's click bubbles into.
        assert!(
            el.query_selector("label").unwrap().is_none(),
            "an openable row is not a label — there is no control to label",
        );
        assert!(
            el.query_selector("input[type=checkbox]").unwrap().is_none(),
            "a file that is here has nothing to tick",
        );
    }

    /// The name may be only the part under a heading; the `title` is always
    /// the whole path, in every shape, since the name can be truncated.
    #[wasm_bindgen_test]
    fn the_title_is_the_whole_path() {
        let el = mount(|| {
            view! {
                <EntryRow name="plate-07.csv" path="raw/plate-07.csv" size="4.1 MB" />
                <EntryRow
                    name="design-01.md"
                    path="notes/design-01.md"
                    size="33 KB"
                    action=EntryAction::Open(Callback::new(|()| ()))
                />
                <EntryRow name="README.md" size="2 KB" />
            }
        });
        for (path, name) in [
            ("raw/plate-07.csv", "plate-07.csv"),
            ("notes/design-01.md", "design-01.md"),
            ("README.md", "README.md"),
        ] {
            let titled = el
                .query_selector(&format!("[title='{path}']"))
                .unwrap()
                .unwrap_or_else(|| panic!("titled {path}; markup was {}", el.inner_html()));
            assert_eq!(titled.text_content().unwrap(), name);
        }
    }

    #[wasm_bindgen_test]
    fn a_row_with_no_action_is_inert_and_says_so() {
        // The defect this rule removes: a row that draws a pointer and does
        // nothing when clicked. Deleted and ignored files reach this shape.
        let el = mount(|| view! { <EntryRow name=".DS_Store" size="6 KB" /> });
        let main = el
            .query_selector(&format!(".{}", style::main))
            .unwrap()
            .expect("the row has a main area");
        let class = main.get_attribute("class").unwrap();
        assert!(
            class.contains(style::inert),
            "an actionless row carries the inert class, which drops the pointer",
        );
        assert!(el.query_selector("label").unwrap().is_none());
        assert!(el.query_selector("button").unwrap().is_none());
    }

    #[wasm_bindgen_test]
    fn the_overflow_menu_does_not_open_the_file() {
        // The open handler is on the row's inner area and the `[...]` is that
        // area's sibling, so the menu's click never passes through it. Widening
        // the handler to the whole row is what this pins — it would make
        // reaching for the menu open the file as well.
        // A signal, not an `Rc<Cell>`: `Callback` is `Send + Sync`.
        let opened = RwSignal::new(0_u32);
        let el = mount(move || {
            view! {
                <EntryRow
                    name="notes/kickoff-thread.md"
                    size="12 KB"
                    action=EntryAction::Open(Callback::new(move |()| {
                        opened.update(|n| *n += 1);
                    }))
                    actions=vec![MenuAction::new("Copy URI", Callback::new(|()| ()))]
                />
            }
        });
        let trigger = el
            .query_selector(&format!(".{} ~ * button", style::main))
            .unwrap()
            .expect("the menu trigger sits outside the row's clickable area");
        click(&trigger);
        assert_eq!(
            opened.get_untracked(),
            0,
            "opening the menu did not open the file"
        );
    }

    /// The title is not keyboard-reachable, so a marked row also points at the
    /// resolve pane's sentence.
    #[wasm_bindgen_test]
    fn a_marked_row_is_described_by_the_pane_s_sentence() {
        let marked = mount(|| view! { <EntryRow name="plate/a.csv" size="1 KB" differs=true /> });
        let row = marked
            .query_selector("[aria-describedby]")
            .unwrap()
            .expect("a described row");
        assert_eq!(
            row.get_attribute("aria-describedby").as_deref(),
            Some(DIFFERS_ID)
        );

        let plain = mount(|| view! { <EntryRow name="plate/b.csv" size="1 KB" /> });
        assert!(
            plain
                .query_selector("[aria-describedby]")
                .unwrap()
                .is_none(),
            "markup was {}",
            plain.inner_html()
        );
    }

    /// Where several sentences share a page, a provided id wins, so a row can
    /// name the sentence beside it rather than the page's first.
    #[wasm_bindgen_test]
    fn a_provided_id_is_the_one_a_marked_row_names() {
        use leptos::context::Provider;

        let el = mount(|| {
            view! {
                <Provider value=DiffersId("resolve-differing-cell")>
                    <EntryRow name="plate/a.csv" size="1 KB" differs=true />
                </Provider>
            }
        });
        let row = el
            .query_selector("[aria-describedby]")
            .unwrap()
            .expect("a described row");
        assert_eq!(
            row.get_attribute("aria-describedby").as_deref(),
            Some("resolve-differing-cell")
        );
    }

    /// The mark lives in the hole, so it can only appear where there is one: a
    /// row with a box has none, and a mark asked of it is dropped. Without the
    /// prop the hole stays blank.
    #[wasm_bindgen_test]
    fn a_mark_fills_the_hole_and_only_the_hole() {
        let slot = format!(".{} svg", style::nobox);

        let here = mount(|| {
            view! {
                <EntryRow
                    name="notes/kickoff-thread.md"
                    size="12 KB"
                    action=EntryAction::Open(Callback::new(|()| ()))
                    have_mark=true
                />
            }
        });
        assert!(
            here.query_selector(&slot).unwrap().is_some(),
            "an openable row draws its mark inside the box-shaped hole; markup was {}",
            here.inner_html()
        );

        let missing = mount(|| {
            view! {
                <EntryRow
                    name="raw/plate-07.csv"
                    state="Not downloaded"
                    size="4.1 MB"
                    action=EntryAction::Select(
                        EntrySelection::new(RwSignal::new(false), Callback::new(|_| ())),
                    )
                    have_mark=true
                />
            }
        });
        assert!(
            missing.query_selector(&slot).unwrap().is_none(),
            "a selectable row has a box, not a hole, and draws no mark; markup was {}",
            missing.inner_html()
        );

        let blank = mount(|| {
            view! {
                <EntryRow
                    name="notes/kickoff-thread.md"
                    size="12 KB"
                    action=EntryAction::Open(Callback::new(|()| ()))
                />
            }
        });
        assert!(
            blank.query_selector(&slot).unwrap().is_none(),
            "a row not asked for the mark keeps its hole blank; markup was {}",
            blank.inner_html()
        );
    }
}
