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
                  column and one four-line message would push the rest out of the surface. \
                  \
                  Whether anyone else can see the revision is a glyph, not a word: down a \
                  280px column most rows say the same thing, and a word would spend the \
                  width four times over for one bit. The pair is drawn — cloud, and the \
                  slashed cloud for what has not left this machine — because nothing tells \
                  an absent glyph from a component that was not told, which is what the \
                  first three cells are. The word is still there for anyone not reading \
                  pixels, and in the `title`. \
                  \
                  Two facts, two props. `published` earns the cloud; `href` makes the \
                  message a link. A published revision in a bucket with no catalog host \
                  has nowhere to point, so it keeps the cloud and stays text — the reverse \
                  never happens. The row itself is not the link: the time beside the \
                  message is when this copy obtained the revision, which the catalog does \
                  not know."
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
