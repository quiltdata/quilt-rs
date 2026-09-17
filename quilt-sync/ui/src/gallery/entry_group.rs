//! `EntryGroup` stories.

use leptos::prelude::*;

use crate::Cell;
use crate::Story;
use crate::kit::CheckState;
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

/// Root-level files, which are deliberately **not** a group: `(root)` names a
/// directory that does not exist and reads as a real folder, so they render
/// first and ungrouped. The absence of a heading is the design, not a gap.
fn root_files() -> AnyView {
    view! {
        // A column, because a cell's body is a flex ROW and three loose rows
        // would sit beside each other. An `EntryGroup` brings its own.
        <div class="g-stack">
            <EntryRow name="README.md" size="2 KB" />
            <EntryRow name="quilt_summarize.json" size="1 KB" />
            <EntryRow name="manifest.jsonl" state="Not downloaded" size="44 KB" />
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

#[component]
pub fn EntryGroupStories() -> impl IntoView {
    let open = RwSignal::new(true);
    let shut = RwSignal::new(false);
    let scroller = RwSignal::new(true);
    let settled = RwSignal::new(true);
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
                <EntryGroup
                    name="raw/"
                    count=Signal::derive(|| 3)
                    open=open
                    selection=GroupSelection::new(
                        state,
                        Callback::new(move |next: bool| {
                            for row in rows {
                                row.set(next);
                            }
                        }),
                    )
                >
                    <EntryRow
                        name="plate-06.csv"
                        state="Not downloaded"
                        size="4.0 MB"
                        selection=selection(0)
                    />
                    <EntryRow
                        name="plate-07.csv"
                        state="Not downloaded"
                        size="4.1 MB"
                        selection=selection(1)
                    />
                    <EntryRow
                        name="plate-08.csv"
                        state="Not downloaded"
                        size="3.9 MB"
                        selection=selection(2)
                    />
                </EntryGroup>
            </Cell>
            <Cell full=true label="collapsed — the rows are not in the DOM">
                <EntryGroup
                    name="investigations/2026-09-15-installed-package-page/"
                    count=Signal::derive(|| 1)
                    open=shut
                    selection=GroupSelection::new(
                        Signal::derive(|| CheckState::Off),
                        Callback::new(|_: bool| ()),
                    )
                >
                    <EntryRow name="verdicts.md" size="31 KB" />
                </EntryGroup>
            </Cell>
            <Cell full=true label="every file already here — nothing to select, so no box">
                {settled_group(settled)}
            </Cell>
            <Cell full=true label="everything at the root — verdict 14 draws no heading at all">
                {root_files()}
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
