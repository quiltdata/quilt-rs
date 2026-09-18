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
            note="A message, a time, and whether anyone else can see it. The message \
                  ellipsises rather than wrapping — a list of these is a column, and one \
                  four-line message would push the rest out of the surface. \
                  \
                  The first three cells say nothing about the platform and draw no glyph. \
                  A published revision with no catalog host keeps the cloud and stays \
                  text; an unsent one is never a link."
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
            <Cell wide=true label="published — a cloud, and the message links out">
                <RevisionRow
                    message="Re-run plate 7 with the corrected layout"
                    at=ago(3.0 * DAY)
                    published=true
                    href=Some(
                        "https://quilt-lab.example/b/quilt-lab-plates/packages/user/plate-07/tree/c41d8f/"
                            .to_string(),
                    )
                />
            </Cell>
            <Cell wide=true label="published, no catalog host — the cloud without a link">
                <RevisionRow
                    message="Add Caihong folder-upload note"
                    at=ago(2.0 * HOUR)
                    published=true
                />
            </Cell>
            <Cell wide=true label="this copy only — the slashed cloud, never a link">
                <RevisionRow message="Fix the Ernest thread link" at=ago(0.4 * HOUR) published=false />
            </Cell>
        </Story>
    }
}
