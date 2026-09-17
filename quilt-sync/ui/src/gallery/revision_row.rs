//! `RevisionRow` stories.

use leptos::prelude::*;

use crate::Cell;
use crate::Story;
use crate::kit::RevisionRow;

const HOUR: f64 = 3_600_000.0;
const DAY: f64 = 24.0 * HOUR;

fn ago(ms: f64) -> f64 {
    js_sys::Date::now() - ms
}

#[component]
pub fn RevisionRowStories() -> impl IntoView {
    view! {
        <Story
            title="RevisionRow"
            note="A message and a time, and deliberately nothing else. No `Yours` label — \
                  that belongs to the PaneSection around it, which is what lets the same \
                  row serve the current revision, both sides of a resolve, and every row of \
                  the revisions overlay where there are no labels at all. No bucket either: \
                  that is a fact about the package, so in a list it would repeat itself \
                  identically all the way down. And no hash — a revision is named by what \
                  it says and when. \
                  \
                  The message ellipsises rather than wrapping, because a list of these is a \
                  column and one four-line message would push the rest out of the surface."
        >
            <Cell wide=true label="the ordinary case">
                <RevisionRow message="Add Ernest thread" at=ago(2.0 * HOUR) />
            </Cell>
            <Cell wide=true label="long — truncates, whole value in the title">
                <RevisionRow
                    message="Reconcile the folder-upload notes with the feedback thread and \
                             drop the duplicated paragraph about ignored files"
                    at=ago(3.0 * DAY)
                />
            </Cell>
            <Cell wide=true label="empty message — reachable, and not a pair of bare quotes">
                <RevisionRow message="" at=ago(9.0 * DAY) />
            </Cell>
        </Story>
    }
}
