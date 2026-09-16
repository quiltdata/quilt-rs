//! `EntryRow` stories.

use leptos::prelude::*;

use crate::Cell;
use crate::Story;
use crate::kit::EntryRow;
use crate::kit::MenuAction;
use crate::kit::state_label::StateTone;

fn menu() -> Vec<MenuAction> {
    vec![
        MenuAction::new("Open file", Callback::new(|()| ())),
        MenuAction::new("Copy URI", Callback::new(|()| ())),
        MenuAction::new("Stop keeping", Callback::new(|()| ())).danger(),
    ]
}

#[component]
pub fn EntryRowStories() -> impl IntoView {
    let ticked = RwSignal::new(false);

    view! {
        <Story
            title="EntryRow"
            note="The resting state is silent. `Downloaded` is what most of a seven-hundred \
                  row list is, and printing it seven hundred times spends the reader's \
                  attention on the one thing that needs none — so absence says the file is \
                  here. An ignored file is silent too, for a different reason: the Ignored \
                  facet is the only view that shows them, so labelling each one repeats \
                  what the facet already said. \
                  \
                  Only a downloadable row carries a box. A tick on a file already here has \
                  nothing to act on, and it is what made `select all 56` disagree with \
                  `download 17`; the unselectable rows keep the column's width so the names \
                  still line up. Read the sizes down the column — the state slot is fixed \
                  so they stay in one column whether or not the row above carries a label. \
                  \
                  A row the two revisions disagree about carries no word at all: a rule, a \
                  tint and a title. It is information and never a control, because \
                  resolution happens at revision level and there is nothing to click here. \
                  It is also one channel where the design owes two — hover it for the half \
                  a mouse gets free. \
                  \
                  The label stops before the overflow: clicking the row ticks the box, but \
                  a button inside a label that is not its control is invalid markup. Click \
                  a row, then click its dots."
        >
            <Cell full=true label="downloaded — no label, and no box to tick">
                <EntryRow name="notes/ernest-thread.md" size="12 KB" actions=menu() />
            </Cell>
            <Cell full=true label="not downloaded — a fact, and the only kind that ticks">
                <EntryRow
                    name="raw/plate-07.csv"
                    state="Not downloaded"
                    size="4.1 MB"
                    selectable=true
                    selected=ticked
                    on_toggle=Callback::new(move |next| ticked.set(next))
                    actions=menu()
                />
            </Cell>
            <Cell full=true label="changed — attention, the colour v1 used">
                <EntryRow
                    name="notes/caihong-upload.md"
                    state="Changed"
                    tone=StateTone::Attention
                    size="8 KB"
                    actions=menu()
                />
            </Cell>
            <Cell full=true label="new">
                <EntryRow
                    name="raw/plate-08.csv"
                    state="New"
                    tone=StateTone::Attention
                    size="3.9 MB"
                    actions=menu()
                />
            </Cell>
            <Cell full=true label="deleted">
                <EntryRow
                    name="raw/plate-06.csv"
                    state="Deleted"
                    tone=StateTone::Danger
                    size="4.0 MB"
                    actions=menu()
                />
            </Cell>
            <Cell full=true label="ignored — silent, because the facet names it">
                <EntryRow name=".DS_Store" size="6 KB" actions=menu() />
            </Cell>
            <Cell full=true label="the two revisions disagree — no word, and hover it">
                <EntryRow
                    name="notes/ernest-thread.md"
                    size="12 KB"
                    differs=true
                    actions=menu()
                />
            </Cell>
            <Cell full=true label="long name — truncates, whole value in the title">
                <EntryRow
                    name="investigations/2026-09-15-installed-package-page/verdicts-and-handoff-with-a-very-long-leaf-name.md"
                    state="Not downloaded"
                    size="31 KB"
                    selectable=true
                    actions=menu()
                />
            </Cell>
            <Cell full=true label="no menu — an empty list draws none rather than an empty one">
                <EntryRow name="notes/ernest-thread.md" size="12 KB" />
            </Cell>
        </Story>
    }
}
