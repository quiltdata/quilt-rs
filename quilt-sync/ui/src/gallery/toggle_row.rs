//! `ToggleRow` stories. Each sits in a `Card`, which is its only real context.

use leptos::prelude::*;

use crate::Cell;
use crate::Story;
use crate::kit::Card;
use crate::kit::ToggleRow;

#[component]
pub fn ToggleRowStories() -> impl IntoView {
    let on = RwSignal::new(true);
    let off = RwSignal::new(false);
    let disabled_on = RwSignal::new(true);
    let disabled_off = RwSignal::new(false);
    let wrapping = RwSignal::new(true);

    view! {
        <Story
            title="ToggleRow"
            note="The label and its text toggle; the trailing slot does not. Click a countdown \
                  or 'nothing to publish' and nothing happens — those are information, not \
                  controls, and flipping autosync because you clicked a clock would be a bad \
                  surprise. Hover the trailing slot too: the checkbox does not react."
        >
            <Cell wide=true label="on">
                <Card title="State">
                    <ToggleRow
                        label="Get new revisions"
                        sublabel="Every 30s, when nothing is changed here"
                        checked=on
                        trailing=view! { "0:23" }.into_any()
                    />
                </Card>
            </Cell>
            <Cell wide=true label="off">
                <Card title="State">
                    <ToggleRow
                        label="Publish your changes"
                        sublabel="5 min after your last edit"
                        checked=off
                        trailing=view! { "nothing to publish" }.into_any()
                    />
                </Card>
            </Cell>
            <Cell wide=true label="disabled, on and off">
                <Card title="State">
                    <ToggleRow
                        label="Get new revisions"
                        sublabel="Unavailable while signed out"
                        checked=disabled_on
                        disabled=true
                        trailing=view! { "paused" }.into_any()
                    />
                    <ToggleRow
                        label="Publish your changes"
                        sublabel="Unavailable while signed out"
                        checked=disabled_off
                        disabled=true
                    />
                </Card>
            </Cell>
            <Cell wide=true label="sub-label wraps rather than truncating">
                <Card title="State">
                    <ToggleRow
                        label="Publish your changes"
                        sublabel="5 minutes after your last edit, and only while the working \
                                  tree has been quiet for that whole window"
                        checked=wrapping
                        trailing=view! { "4:12" }.into_any()
                    />
                </Card>
            </Cell>
            <Cell wide=true label="no trailing slot">
                <Card title="State">
                    <ToggleRow
                        label="Get new revisions"
                        sublabel="Every 30s, when nothing is changed here"
                        checked=on
                    />
                </Card>
            </Cell>
        </Story>
    }
}
