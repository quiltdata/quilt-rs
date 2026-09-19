//! Card stories.

use leptos::prelude::*;

use crate::Cell;
use crate::Story;
use crate::kit::Button;
use crate::kit::Card;
use crate::kit::ToggleRow;

#[component]
pub fn CardStories() -> impl IntoView {
    let a = RwSignal::new(true);
    let b = RwSignal::new(true);
    let solo = RwSignal::new(false);

    view! {
            <Story
                title="Card"
                note="The page's four regions are all one of these. The card draws a hairline \
                      between any two children, so a card holding a mix of row types stays \
                      evenly divided — pass one wrapper child to opt out, as the queue does. The \
                      rows were measured against this surface: their hairlines and hover tint \
                      assume `--q-bgColor-default` under them. Title and count are both \
                      optional, and the count must be derived from the rows."
            >
                <Cell wide=true label="two rows — hairline is the card's, not the row's">
                    <Card title="Autosync">
                        <ToggleRow
                            label="Get new revisions"
                            sublabel="Every 30s, keeping any local changes"
                            checked=a
                            trailing=view! { "0:23" }.into_any()
                        />
                        <ToggleRow
                            label="Publish your changes"
                            sublabel="After 5 min of inactivity"
                            checked=b
                            trailing=view! { "nothing to publish" }.into_any()
                        />
                    </Card>
                </Cell>
                <Cell wide=true label="one row — no divider to draw">
                    <Card title="Accounts">
                        <ToggleRow
                            label="Sign me in automatically"
                            sublabel="Uses the browser session"
                            checked=solo
                        />
                    </Card>
                </Cell>
                <Cell wide=true label="long title">
                    <Card title="Autosync and background publishing">
                        <ToggleRow label="Get new revisions" sublabel="Every 30s" checked=a />
                    </Card>
                </Cell>
                <Cell wide=true label="with a count — the queue's header">
                    <Card title="Needs your attention" count=19>
                        <ToggleRow label="Get new revisions" sublabel="Every 30s" checked=a />
                    </Card>
                </Cell>
                <Cell wide=true label="count of one">
                    <Card title="Needs your attention" count=1>
                        <ToggleRow label="Get new revisions" sublabel="Every 30s" checked=a />
                    </Card>
                </Cell>
                <Cell wide=true label="no title — the list card, named by its own SegmentedControl">
                    <Card>
                        <ToggleRow label="Get new revisions" sublabel="Every 30s" checked=a />
                    </Card>
                </Cell>
                        <Cell wide=true label="flush — rows that carry their own padding reach the border">
                    <Card flush=true label="Files">
                        <div style="padding:var(--q-space-2) var(--q-space-3)">"README.md"</div>
                        <div style="padding:var(--q-space-2) var(--q-space-3)">"raw/plate-07.csv"</div>
                        <div style="padding:var(--q-space-2) var(--q-space-3); \
                                    display:flex; justify-content:flex-end">
                            <Button on_click=|_| ()>"Download 2"</Button>
                        </div>
                    </Card>
                </Cell>
    </Story>
        }
}
