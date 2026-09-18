//! `ToggleRow` stories. Each sits in a `Card`, which is its only real context.

use leptos::prelude::*;

use crate::Cell;
use crate::Story;
use crate::kit::Card;
use crate::kit::StateLabel;
use crate::kit::StateTone;
use crate::kit::ToggleRow;

#[component]
pub fn ToggleRowStories() -> impl IntoView {
    view! { <TrailingStates /> <Shapes /> }
}

/// The three states a toggle's trailing slot must distinguish, which is the whole
/// point of the slot existing. The third one was missing until now.
#[component]
fn TrailingStates() -> impl IntoView {
    let pull = RwSignal::new(true);
    let publish = RwSignal::new(true);
    let paused = RwSignal::new(true);

    view! {
        <Story
            title="ToggleRow — armed · idle · paused"
            note="Armed counts down. Idle says why it is not counting, because a blank \
                  leaves the reader guessing between broken, working, and nothing to do. \
                  Paused is the third state the earlier design had no representation for, \
                  and it is the whole of the 2026-07-11 report. Note what paused does not \
                  do: the checkbox stays on, because the setting is on and what stopped is \
                  the machinery, and it stays enabled, because flipping it is one of only \
                  three ways to clear the pause today. There is no Resume button anywhere — \
                  all six `PausedReason` variants are things the user has to fix."
        >
            <Cell wide=true label="armed — a next tick exists">
                <Card title="Autosync">
                    <ToggleRow
                        label="Get new revisions"
                        sublabel="Every 30s, keeping any local changes"
                        checked=pull
                        trailing=view! { "0:23" }.into_any()
                    />
                </Card>
            </Cell>
            <Cell wide=true label="idle — on, nothing to do">
                <Card title="Autosync">
                    <ToggleRow
                        label="Publish your changes"
                        sublabel="After 5 min of inactivity"
                        checked=publish
                        trailing=view! { "nothing to publish" }.into_any()
                    />
                </Card>
            </Cell>
            <Cell wide=true label="paused — on, and not operating">
                <Card title="Autosync">
                    <ToggleRow
                        label="Publish your changes"
                        sublabel="After 5 min of inactivity"
                        checked=paused
                        trailing=view! {
                            <StateLabel tone=StateTone::Attention>"Paused"</StateLabel>
                        }
                            .into_any()
                    />
                </Card>
            </Cell>
        </Story>
    }
}

#[component]
fn Shapes() -> impl IntoView {
    let on = RwSignal::new(true);
    let off = RwSignal::new(false);
    let disabled_on = RwSignal::new(true);
    let disabled_off = RwSignal::new(false);
    let wrapping = RwSignal::new(true);

    view! {
        <Story
            title="ToggleRow — shapes"
            note="Disabled is NOT how paused is drawn — see the story above. Here the \
                  setting genuinely cannot be used, because there is no session to sync \
                  with. \
                  \
                  The label and its text toggle; the trailing slot does not. Click a countdown \
                  or 'nothing to publish' and nothing happens — those are information, not \
                  controls, and flipping autosync because you clicked a clock would be a bad \
                  surprise. Hover the trailing slot too: the checkbox does not react."
        >
            <Cell wide=true label="on">
                <Card title="State">
                    <ToggleRow
                        label="Get new revisions"
                        sublabel="Every 30s, keeping any local changes"
                        checked=on
                        trailing=view! { "0:23" }.into_any()
                    />
                </Card>
            </Cell>
            <Cell wide=true label="off">
                <Card title="State">
                    <ToggleRow
                        label="Publish your changes"
                        sublabel="After 5 min of inactivity"
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
                        trailing=view! { "unavailable" }.into_any()
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
                        sublabel="After 5 minutes of inactivity, and only while nothing in \
                                  the package has changed for that whole window"
                        checked=wrapping
                        trailing=view! { "4:12" }.into_any()
                    />
                </Card>
            </Cell>
            <Cell wide=true label="no trailing slot">
                <Card title="State">
                    <ToggleRow
                        label="Get new revisions"
                        sublabel="Every 30s, keeping any local changes"
                        checked=on
                    />
                </Card>
            </Cell>
        </Story>
    }
}
