//! `EntryGroup` stories.

use leptos::prelude::*;

use crate::Cell;
use crate::Story;
use crate::kit::CheckState;
use crate::kit::EntryAction;
use crate::kit::EntryGroup;
use crate::kit::EntryRow;
use crate::kit::EntrySelection;
use crate::kit::GroupSelection;

/// Enough rows to scroll the heading against. Downloaded, so none of them
/// carries a box — the point of the cell is the heading, not the selection.
fn notes() -> AnyView {
    view! {
        <EntryRow name="ernest-thread.md" size="12 KB" />
        <EntryRow name="caihong-upload.md" size="8 KB" />
        <EntryRow name="plate-notes.md" size="4 KB" />
        <EntryRow name="handoff.md" size="31 KB" />
        <EntryRow name="requirements.md" size="33 KB" />
        <EntryRow name="design.md" size="30 KB" />
    }
    .into_any()
}

/// A group whose files are all present. Every row is downloaded, so none of
/// them carries a box — and neither does the heading, because a box that can
/// select nothing is the dead control `Select` already refuses to be.
fn settled_group(open: RwSignal<bool>) -> AnyView {
    view! {
        <EntryGroup name="notes/" count=Signal::derive(|| 3) open=open>
            <EntryRow name="ernest-thread.md" size="12 KB" />
            <EntryRow name="caihong-upload.md" size="8 KB" />
            <EntryRow name="plate-notes.md" size="4 KB" />
        </EntryGroup>
    }
    .into_any()
}

/// The degenerate list: no group anywhere, which `Group: None` reaches from the
/// toolbar at any moment and a flat package reaches by existing. Every row's
/// disclosure gutter would be an indent with nothing to align to, so the list
/// sets `--q-entry-gutter` to zero and the names start where the box does.
fn flat_list(pending: RwSignal<bool>) -> AnyView {
    view! {
        <div class="g-stack" style="--q-entry-gutter: 0">
            <EntryRow name="README.md" size="2 KB" />
            <EntryRow name="quilt_summarize.json" size="1 KB" />
            <EntryRow
                name="manifest.jsonl"
                state="Not downloaded"
                size="44 KB"
                action=EntryAction::Select(
                    EntrySelection::new(pending, Callback::new(move |next| pending.set(next))),
                )
            />
        </div>
    }
    .into_any()
}

/// Root-level files above a group, which is the arrangement verdict 14
/// describes: they render **first** and ungrouped, because `(root)` names a
/// directory that does not exist and reads as a real folder.
///
/// Shown with a group under them rather than alone, because alone is
/// misleading — the rows carry an empty disclosure gutter and a checkbox
/// column, and with no heading anywhere in the cell those two columns look like
/// an indent nobody asked for. They are there so a root file's box lands on the
/// same x as a grouped file's, and that is only visible when both are present.
fn root_files(pending: RwSignal<bool>, grouped: RwSignal<bool>) -> AnyView {
    view! {
        // A column, because a cell's body is a flex ROW and three loose rows
        // would sit beside each other. An `EntryGroup` brings its own.
        <div class="g-stack">
            <EntryRow name="README.md" size="2 KB" />
            <EntryRow name="quilt_summarize.json" size="1 KB" />
            // `Not downloaded` and therefore selectable. A row in that state
            // without a box is the mock that teaches the wrong rule.
            <EntryRow
                name="manifest.jsonl"
                state="Not downloaded"
                size="44 KB"
                action=EntryAction::Select(
                    EntrySelection::new(pending, Callback::new(move |next| pending.set(next))),
                )
            />
            <EntryGroup name="raw/" count=Signal::derive(|| 2) open=grouped>
                <EntryRow name="plate-06.csv" size="4.0 MB" />
                <EntryRow name="plate-07.csv" size="4.1 MB" />
            </EntryGroup>
        </div>
    }
    .into_any()
}

const NOTE: &str = "A container, not a heading: it holds its rows, owns whether they are \
                  shown, and carries a box that ticks all of them. GroupHeading stays flat \
                  for the pages that emit one between runs of rows. \
                  \
                  A collapsed group renders nothing — `Show`, not `display: none`. The \
                  1000-entry cap is argued partly on grouping sparing the DOM, and a hidden \
                  subtree is still built, so hiding with CSS would have made that argument \
                  false while looking identical. Collapse one and inspect it. \
                  \
                  The heading cannot be a label, because it holds the disclosure button and \
                  a label may not contain another labelable element — so the box names \
                  itself, and the name says which group, since a list of them all reading \
                  `Select all` names nothing. \
                  \
                  Scroll the last cell: the heading sticks, and it costs 29px against a \
                  32px row, which at the height floor is most of a row per heading.";

/// The group whose heading box is derived from its three rows, so the heading
/// and the rows can never disagree about how many are ticked.
fn picks_group(
    open: RwSignal<bool>,
    picks: GroupSelection,
    selection: impl Fn(usize) -> EntrySelection + Send + Sync + 'static,
) -> AnyView {
    view! {
                <EntryGroup name="raw/" count=Signal::derive(|| 3) open=open selection=picks>
                    {[
                        ("plate-06.csv", "4.0 MB"),
                        ("plate-07.csv", "4.1 MB"),
                        ("plate-08.csv", "3.9 MB"),
                    ]
                        .into_iter()
                        .enumerate()
                        .map(|(i, (name, size))| {
                            view! {
                                <EntryRow
                                    name=name
                                    state="Not downloaded"
                                    size=size
                                    action=EntryAction::Select(selection(i))
                                />
                            }
                        })
                        .collect_view()}
                </EntryGroup>
    }
    .into_any()
}

#[component]
pub fn EntryGroupStories() -> impl IntoView {
    let open = RwSignal::new(true);
    let shut = RwSignal::new(false);
    let scroller = RwSignal::new(true);
    let settled = RwSignal::new(true);
    let pending = RwSignal::new(false);
    let collapsed_row = RwSignal::new(false);
    let grouped = RwSignal::new(true);
    let flat = RwSignal::new(false);
    // Three real ticks, so the group's box is derived from its rows rather than
    // asserted beside them — a heading that disagrees with what is under it is
    // the mock that teaches the wrong thing.
    let rows = [
        RwSignal::new(false),
        RwSignal::new(true),
        RwSignal::new(false),
    ];
    let picked = Signal::derive(move || rows.iter().filter(|r| r.get()).count());

    let state = Signal::derive(move || match picked.get() {
        0 => CheckState::Off,
        n if n >= rows.len() => CheckState::On,
        _ => CheckState::Mixed,
    });

    let picks = GroupSelection::new(
        state,
        Callback::new(move |next: bool| {
            for row in rows {
                row.set(next);
            }
        }),
    );

    let selection = move |index: usize| {
        let row = rows[index];
        EntrySelection::new(row, Callback::new(move |next| row.set(next)))
    };

    view! {
        <Story
            title="EntryGroup"
            note=NOTE
        >
            <Cell full=true label="expanded · tri-state box — tick a row, then the heading">
                {picks_group(open, picks, selection)}
            </Cell>
            <Cell full=true label="collapsed — the rows are not in the DOM">
                <EntryGroup
                    name="investigations/2026-09-15-installed-package-page/"
                    count=Signal::derive(|| 1)
                    open=shut
                    selection=GroupSelection::new(
                        Signal::derive(move || collapsed_row.get().into()),
                        Callback::new(move |next: bool| collapsed_row.set(next)),
                    )
                >
                    // Selectable, so the heading's box has something to act on.
                    // A `select all` over rows that cannot be selected is the
                    // same lie one level up.
                    <EntryRow
                        name="verdicts.md"
                        state="Not downloaded"
                        size="31 KB"
                        action=EntryAction::Select(
                            EntrySelection::new(
                                collapsed_row,
                                Callback::new(move |next| collapsed_row.set(next)),
                            ),
                        )
                    />
                </EntryGroup>
            </Cell>
            <Cell full=true label="every file already here — nothing to select, so no box">
                {settled_group(settled)}
            </Cell>
            <Cell full=true label="no group anywhere — Group: None, or a flat package. No gutter to keep">
                {flat_list(flat)}
            </Cell>
            <Cell full=true label="root files first, then a group — one column, and no (root) heading">
                {root_files(pending, grouped)}
            </Cell>
            <Cell full=true label="sticky, against a short scroll">
                <div style="width:100%;max-height:140px;overflow-y:auto">
                    <EntryGroup
                        name="notes/"
                        count=Signal::derive(|| 6)
                        open=scroller
                    >
                        {notes()}
                    </EntryGroup>
                </div>
            </Cell>
        </Story>
    }
}
