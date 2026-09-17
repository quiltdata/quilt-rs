//! Combined · the file list.
//!
//! `EntryGroup`, `EntryRow` and `Checkbox` wired the way the file pane wires
//! them, over one path fixture rather than a hand-built cell per rule. Nothing
//! new: each component has its own Core section, and what only appears once they
//! are composed is what they agree about.
//!
//! # One gutter, three checkbox columns
//!
//! Select-all's box, a group's box and a file's box sit on one x, and none of
//! the three components chooses it. `--q-entry-gutter` is set once, here, by the
//! list; each component reads it with its own fallback so it is not broken when
//! looked at alone. The boxes lining up is the contract — there is nothing else
//! to check, and nothing else would catch a component that stopped reading the
//! value.
//!
//! # The zero-gutter rule is a view, not an edge case
//!
//! The list decides whether it rendered any heading and sets the value to `0`
//! when it did not, because a disclosure column with no disclosure anywhere in
//! it is an indent nobody asked for. `Group: None` reaches that state from the
//! toolbar at any moment, so it is a first-class view of the same package rather
//! than a degenerate one — which is why the second cell is the same fixture and
//! not a shorter one.
//!
//! # Root files first, and no `(root)` heading
//!
//! Verdict 14: files at the package root render above the groups and ungrouped,
//! because `(root)` names a directory that does not exist. They share the
//! column, which is only visible with a group beneath them.
//!
//! # Where select-all is, and is not
//!
//! The list carries one here because the gutter contract is three columns and
//! two of them are not worth measuring. It belongs to the toolbar on the page —
//! see the toolbar section, which draws it without an offset, having no rows to
//! align to.

use leptos::prelude::*;

use crate::Cell;
use crate::Story;
use crate::kit::CheckState;
use crate::kit::EntryGroup;
use crate::kit::EntryRow;
use crate::kit::EntrySelection;
use crate::kit::GroupSelection;
use crate::kit::MenuAction;
use crate::kit::SelectAll;
use crate::kit::state_label::StateTone;

/// One file in the fixture, as the row draws it.
///
/// Three variants and not a struct of options, because the row's props are not
/// independent: a resting file has no state *and* no box, and a changed file has
/// a state and no box. `Missing` carries its index in the list's selection
/// vector — the row's identity for the whole section, since a group's tri-state
/// is derived from the indices under it and so cannot disagree with its rows.
enum Entry {
    /// Here and unchanged. Silent — no label at all, which is what most of a
    /// seven-hundred-row list is.
    Here(&'static str, &'static str),
    /// Here, and different from the revision. A fact, and nothing to tick.
    Changed(&'static str, &'static str),
    /// Not here. The only kind that carries a box.
    Missing(&'static str, &'static str, usize),
}

impl Entry {
    /// Its index in the selection vector, when it has one. What decides whether
    /// the row draws a box, and what a group collects to derive its own.
    const fn pick(&self) -> Option<usize> {
        match *self {
            Self::Missing(_, _, i) => Some(i),
            _ => None,
        }
    }
}

/// The rows at the package root. Three files, one of them downloadable, so the
/// ungrouped run has a box in it and the column is real rather than reserved.
fn root() -> Vec<Entry> {
    vec![
        Entry::Here("README.md", "2 KB"),
        Entry::Here("quilt_summarize.json", "1 KB"),
        Entry::Missing("manifest.jsonl", "44 KB", 0),
    ]
}

/// Three groups, chosen for what they disagree about: one with nothing to
/// select, one with everything, and one long enough to truncate its own name.
fn groups() -> Vec<(&'static str, Vec<Entry>)> {
    vec![
        (
            "notes/",
            vec![
                Entry::Changed("ernest-thread.md", "12 KB"),
                Entry::Changed("caihong-upload.md", "8 KB"),
                Entry::Here("plate-notes.md", "4 KB"),
            ],
        ),
        (
            "raw/",
            vec![
                Entry::Here("plate-05.csv", "4.2 MB"),
                Entry::Missing("plate-06.csv", "4.0 MB", 1),
                Entry::Missing("plate-07.csv", "4.1 MB", 2),
                Entry::Missing("plate-08.csv", "3.9 MB", 3),
            ],
        ),
        (
            "investigations/2026-09-15-installed-package-page/",
            vec![
                Entry::Here("2026-09-15-requirements.md", "33 KB"),
                Entry::Missing("2026-09-16-verdicts-and-handoff.md", "31 KB", 4),
            ],
        ),
    ]
}

/// How many rows the fixture offers a tick. The list's own number, not a
/// constant beside it — `SelectAll`'s label and every group's tri-state read
/// the same vector.
const PICKS: usize = 5;

fn menu() -> Vec<MenuAction> {
    vec![
        MenuAction::new("Open file", Callback::new(|()| ())),
        MenuAction::new("Copy URI", Callback::new(|()| ())),
        MenuAction::new("Ignore", Callback::new(|()| ())),
        MenuAction::new("Stop keeping", Callback::new(|()| ())).danger(),
    ]
}

/// One row, in the shape its state puts it in.
///
/// Three calls rather than one call with three optional values: `state` and
/// `selection` are both absent-means-something, and `EntrySelection` is a value
/// carrying its own callback, so the unselectable case is the call that does not
/// pass it. Spelling the arms out is the component's contract showing through.
fn row(entry: &Entry, picks: RwSignal<Vec<bool>>) -> AnyView {
    match *entry {
        Entry::Here(name, size) => {
            view! { <EntryRow name=name size=size actions=menu() /> }.into_any()
        }
        Entry::Changed(name, size) => view! {
            <EntryRow
                name=name
                state="Changed"
                tone=StateTone::Attention
                size=size
                actions=menu()
            />
        }
        .into_any(),
        Entry::Missing(name, size, i) => view! {
            <EntryRow
                name=name
                state="Not downloaded"
                size=size
                selection=EntrySelection::new(
                    Signal::derive(move || picks.with(|p| p[i])),
                    Callback::new(move |next| picks.update(|p| p[i] = next)),
                )
                actions=menu()
            />
        }
        .into_any(),
    }
}

/// A heading and its rows. The heading's box is derived from the indices under
/// it and toggles exactly those, so `Mixed` is a fact about the rows rather
/// than an assertion beside them — and a group with no index carries no box,
/// because it would be a control with nothing to act on.
fn group(name: &'static str, entries: Vec<Entry>, picks: RwSignal<Vec<bool>>) -> AnyView {
    let open = RwSignal::new(true);
    let count = entries.len();
    let mine: Vec<usize> = entries.iter().filter_map(Entry::pick).collect();

    let selection = (!mine.is_empty()).then(|| {
        let derived = mine.clone();
        let toggled = mine.clone();
        GroupSelection::new(
            Signal::derive(move || {
                let ticked = picks.with(|p| derived.iter().filter(|&&i| p[i]).count());
                if ticked == 0 {
                    CheckState::Off
                } else if ticked == derived.len() {
                    CheckState::On
                } else {
                    CheckState::Mixed
                }
            }),
            Callback::new(move |next: bool| {
                picks.update(|p| {
                    for &i in &toggled {
                        p[i] = next;
                    }
                });
            }),
        )
    });

    let rows = StoredValue::new(entries);
    let children = move || rows.with_value(|es| es.iter().map(|e| row(e, picks)).collect_view());

    // Same two arms as a row, for the same reason one level up: a group with
    // nothing selectable under it does not take a `GroupSelection` at all.
    match selection {
        Some(picks_all) => view! {
            <EntryGroup
                name=name
                count=Signal::derive(move || count)
                open=open
                selection=picks_all
            >
                {children}
            </EntryGroup>
        }
        .into_any(),
        None => view! {
            <EntryGroup name=name count=Signal::derive(move || count) open=open>
                {children}
            </EntryGroup>
        }
        .into_any(),
    }
}

/// The list, and the select-all above it.
///
/// `grouped=false` is `Group: None`: every file flat, in one run, and therefore
/// no heading anywhere — which is what sets the gutter to zero. The same
/// fixture, the same rows, one control moved.
fn file_list(grouped: bool, picks: RwSignal<Vec<bool>>) -> AnyView {
    let ticked = Signal::derive(move || picks.with(|p| p.iter().filter(|t| **t).count()));

    // Zero when the list rendered no heading, which is the whole rule: the
    // column exists to put a file's box under its group's box, and with no group
    // there is nothing to be under.
    let style = if grouped {
        "--q-entry-gutter: 16px"
    } else {
        "--q-entry-gutter: 0"
    };

    let flat = StoredValue::new(
        root()
            .into_iter()
            .chain(groups().into_iter().flat_map(|(_, es)| es))
            .collect::<Vec<_>>(),
    );
    let roots = StoredValue::new(root());

    view! {
        <div class="g-stack" style=style>
            // Select-all sits in the rows' checkbox column, with the disclosure
            // gutter to its left — the same two values the rows and the headings
            // start from, so the third box lands on the one x. Under
            // `Group: None` the gutter is zero and it moves left with them.
            <div style="display:flex; padding:var(--q-space-1) var(--q-space-3); \
                        gap:var(--q-space-2)">
                <span style="flex:0 0 var(--q-entry-gutter, 16px)" />
                <SelectAll
                    selected=ticked
                    total=Signal::derive(|| PICKS)
                    on_toggle=move |next| picks.update(|p| p.fill(next))
                />
            </div>
            // Short enough that the headings have something to stick against.
            <div style="max-height:260px; overflow-y:auto; border-top:1px solid \
                        var(--q-borderColor-muted)">
                {if grouped {
                    view! {
                        {move || roots.with_value(|es| es.iter().map(|e| row(e, picks)).collect_view())}
                        {groups()
                            .into_iter()
                            .map(|(name, entries)| group(name, entries, picks))
                            .collect_view()}
                    }
                        .into_any()
                } else {
                    view! {
                        {move || flat.with_value(|es| es.iter().map(|e| row(e, picks)).collect_view())}
                    }
                        .into_any()
                }}
            </div>
        </div>
    }
    .into_any()
}

const NOTE: &str = "Three checkbox columns on one x — select-all's, a group's and a file's — \
                    and none of the three components chose it. The list sets \
                    `--q-entry-gutter` once and each component reads it with its own \
                    fallback. Sight down the boxes; that is the whole contract, and nothing \
                    smaller would catch a component that stopped reading the value. \
                    \
                    Root files render first and ungrouped, because `(root)` names a \
                    directory that does not exist. Alone they look indented for no reason, \
                    which is why they are only ever shown with a group beneath them. \
                    \
                    Second cell is the same fixture under `Group: None`. No heading renders, \
                    so the list sets the gutter to zero and every box moves left together — \
                    a view the toolbar reaches at any moment, not an edge case. \
                    \
                    Tick a row and watch its heading go indeterminate: the group's box is \
                    derived from the rows under it, so the two cannot disagree. `notes/` has \
                    no box at all — every file is already here, and a box that can select \
                    nothing is the dead control `Select` refuses to be. \
                    \
                    Collapse a group and inspect it: the rows leave the DOM. Scroll the \
                    list and the headings stick, at 29px against a 32px row.";

#[component]
pub fn FileListStories() -> impl IntoView {
    let grouped = RwSignal::new(vec![false; PICKS]);
    let flat = RwSignal::new(vec![false; PICKS]);

    view! {
        <Story title="The file list" note=NOTE>
            <Cell full=true label="grouped — root files, then three groups, one gutter">
                {file_list(true, grouped)}
            </Cell>
            <Cell full=true label="Group: None — no heading renders, so the gutter is 0">
                {file_list(false, flat)}
            </Cell>
        </Story>
    }
}
