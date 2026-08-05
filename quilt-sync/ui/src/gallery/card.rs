//! Card stories.

use leptos::prelude::*;

use crate::Cell;
use crate::Story;
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
            note="A titled container. The card draws the hairline between any two children, \
                  so a card holding a mix of row types stays evenly divided — which a rule \
                  inside one row's module could not do. Look at the two-child case for that."
        >
            <Cell wide=true label="two rows — hairline is the card's, not the row's">
                <Card title="Autosync">
                    <ToggleRow
                        label="Get new revisions"
                        sublabel="Every 30s, when nothing is changed here"
                        checked=a
                        trailing=view! { "0:23" }.into_any()
                    />
                    <ToggleRow
                        label="Publish your changes"
                        sublabel="5 min after your last edit"
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
        </Story>
    }
}
