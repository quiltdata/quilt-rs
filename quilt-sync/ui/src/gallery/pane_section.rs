//! `PaneSection` stories.

use leptos::prelude::*;

use crate::Cell;
use crate::Story;
use crate::kit::Card;
use crate::kit::PaneSection;
use crate::kit::RevisionRow;

const HOUR: f64 = 3_600_000.0;

fn ago(ms: f64) -> f64 {
    js_sys::Date::now() - ms
}

#[component]
pub fn PaneSectionStories() -> impl IntoView {
    view! {
        <Story
            title="PaneSection"
            note="A Card's label with none of a Card's surface, for blocks inside a pane \
                  that is already the box — a bordered block inside a bordered pane is the \
                  box-in-box the design rules out. \
                  \
                  Resolve mode stacks one inside another, and two identical labels at two \
                  levels is a hierarchy nobody can see, so the nested one drops a heading \
                  level and takes the weight down with it. The last cell is the shape that \
                  drove this: a titleless Card holding the sections."
        >
            <Cell wide=true label="labelled">
                <PaneSection label="Revision">
                    <RevisionRow message="Add Ernest thread" at=ago(2.0 * HOUR) />
                </PaneSection>
            </Cell>
            <Cell wide=true label="no label — the pane names it">
                <PaneSection>
                    <p style="margin:0">
                        "Quilt doesn't merge file contents. Pick which side's revision
                         becomes the shared one."
                    </p>
                </PaneSection>
            </Cell>
            <Cell full=true label="nested, inside a titleless Card — the context pane's shape">
                <Card>
                    <PaneSection>
                        <PaneSection nested=true label="Yours">
                            <RevisionRow message="Fix the Ernest thread link" at=ago(0.4 * HOUR) />
                        </PaneSection>
                        <PaneSection nested=true label="Published">
                            <RevisionRow
                                message="Add Caihong folder-upload note"
                                at=ago(2.0 * HOUR)
                            />
                        </PaneSection>
                    </PaneSection>
                </Card>
            </Cell>
        </Story>
    }
}
