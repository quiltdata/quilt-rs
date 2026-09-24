//! `EntryRow` stories.

use leptos::prelude::*;

use crate::Cell;
use crate::Story;
use crate::differs_caption;
use crate::kit::EntryAction;
use crate::kit::EntryRow;
use crate::kit::EntrySelection;
use crate::kit::MenuAction;
use crate::kit::state_label::StateTone;

fn menu() -> Vec<MenuAction> {
    vec![
        MenuAction::new("Open file", Callback::new(|()| ())),
        MenuAction::new("Copy URI", Callback::new(|()| ())),
        MenuAction::new("Stop keeping", Callback::new(|()| ())).danger(),
    ]
}

/// A tick wired to a real signal. A row whose box discards the click is the mock
/// that teaches the wrong rule, which is why `EntrySelection` carries both.
fn pick(at: RwSignal<bool>) -> EntryAction {
    EntryAction::Select(EntrySelection::new(at, Callback::new(move |n| at.set(n))))
}

/// An open wired to a readout. A no-op callback would let a cell claim a
/// behaviour and then not show it — the exact defect this rule exists to fix.
fn open(name: &'static str, into: RwSignal<String>) -> EntryAction {
    EntryAction::Open(Callback::new(move |()| into.set(name.to_string())))
}

const STATES: &str = "The resting state is silent: `Downloaded` is what most of a seven- \
                      hundred row list is, and printing it seven hundred times spends \
                      attention on the one thing that needs none. An ignored file is silent \
                      too, because the Ignored facet is the only view that shows them. Only \
                      a downloadable row carries a box — a tick on a file already here has \
                      nothing to act on, and it is what made `select all 56` disagree with \
                      `download 17`. The boxless rows keep the column's width, so the names \
                      still line up.";

fn states(ticked: RwSignal<bool>, fresh: RwSignal<bool>, opened: RwSignal<String>) -> AnyView {
    view! {
        <Story title="EntryRow" note=STATES>
            <Cell full=true label="downloaded — no box; a click opens the local file">
                <EntryRow
                    name="notes/ernest-thread.md"
                    size="12 KB"
                    action=open("notes/ernest-thread.md", opened)
                    actions=menu()
                />
            </Cell>
            <Cell full=true label="not downloaded — a fact, and the only kind that ticks">
                <EntryRow
                    name="raw/plate-07.csv"
                    state="Not downloaded"
                    size="4.1 MB"
                    action=pick(ticked)
                    actions=menu()
                />
            </Cell>
            <Cell full=true label="changed — here, so a click opens it">
                <EntryRow
                    name="notes/caihong-upload.md"
                    state="Changed"
                    tone=StateTone::Attention
                    size="8 KB"
                    action=open("notes/caihong-upload.md", opened)
                    actions=menu()
                />
            </Cell>
            <Cell full=true label="new — added by the revision and not here yet, so it ticks">
                <EntryRow
                    name="raw/plate-08.csv"
                    state="New"
                    tone=StateTone::Attention
                    size="3.9 MB"
                    action=pick(fresh)
                    actions=menu()
                />
            </Cell>
        </Story>
    }
    .into_any()
}

const CLICK: &str = "A click does the thing the row can do, and where the file is decides \
                     which, so the box and the gesture cannot disagree. A file that is here \
                     has no box and opens locally — the recent-files list's own gesture, so \
                     a click means the same in both of this app's file lists. One that is \
                     not here ticks instead. One that can do neither draws no pointer at \
                     all, because a row advertising a click it cannot honour is worse than \
                     an inert one. Click the first two rows, then read the last cell.";

fn click_rule(opened: RwSignal<String>) -> AnyView {
    view! {
        <Story title="EntryRow · what a click does" note=CLICK>
            <Cell full=true label="here — opens">
                <EntryRow
                    name="notes/plate-notes.md"
                    size="4 KB"
                    action=open("notes/plate-notes.md", opened)
                    actions=menu()
                />
            </Cell>
            <Cell full=true label="not here — ticks">
                <EntryRow
                    name="raw/plate-09.csv"
                    state="Not downloaded"
                    size="4.4 MB"
                    action=pick(RwSignal::new(false))
                    actions=menu()
                />
            </Cell>
            <Cell full=true label="deleted — neither; no pointer">
                <EntryRow
                    name="raw/plate-06.csv"
                    state="Deleted"
                    tone=StateTone::Danger
                    size="4.0 MB"
                    actions=menu()
                />
            </Cell>
            <Cell full=true label="ignored — silent, and inert for the same reason">
                <EntryRow name=".DS_Store" size="6 KB" actions=menu() />
            </Cell>
            <Cell full=true label="what the last click opened">
                <span class="g-note">{move || opened.get()}</span>
            </Cell>
        </Story>
    }
    .into_any()
}

const EDGES: &str = "A row the two revisions disagree about carries no word: a rule, a tint \
                     and a title. It is information and never a control, since resolution \
                     happens at revision level. It is also one channel where the design owes \
                     two — hover it for the half a mouse gets free. The label stops before \
                     the overflow, because a button inside a label that is not its control \
                     is invalid markup. Click a row, then click its dots.";

fn edges(long: RwSignal<bool>, opened: RwSignal<String>) -> AnyView {
    view! {
        <Story title="EntryRow · edges" note=EDGES>
            <Cell full=true label="the two revisions disagree — no word, and hover it">
                <EntryRow
                    name="notes/ernest-thread.md"
                    size="12 KB"
                    differs=true
                    action=open("notes/ernest-thread.md", opened)
                    actions=menu()
                />
                {differs_caption(1)}
            </Cell>
            <Cell full=true label="long name — truncates, whole value in the title">
                <EntryRow
                    name="investigations/2026-09-15-installed-package-page/verdicts-and-handoff-with-a-very-long-leaf-name.md"
                    state="Not downloaded"
                    size="31 KB"
                    action=pick(long)
                    actions=menu()
                />
            </Cell>
            <Cell full=true label="no menu — an empty list draws none rather than an empty one">
                <EntryRow
                    name="notes/ernest-thread.md"
                    size="12 KB"
                    action=open("notes/ernest-thread.md", opened)
                />
            </Cell>
        </Story>
    }
    .into_any()
}

#[component]
pub fn EntryRowStories() -> impl IntoView {
    let ticked = RwSignal::new(false);
    let fresh = RwSignal::new(false);
    let long = RwSignal::new(false);
    // Shared by all three stories, so a click anywhere in the section reports in
    // one place.
    let opened = RwSignal::new(String::from("nothing yet"));

    view! {
        {states(ticked, fresh, opened)}
        {click_rule(opened)}
        {edges(long, opened)}
    }
}
