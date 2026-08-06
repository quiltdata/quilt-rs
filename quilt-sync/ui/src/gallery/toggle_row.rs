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
            note="ARMED counts down. IDLE says why it is not counting, because a blank \
                  leaves the user guessing between broken, working, and nothing to do. \
                  PAUSED is the third state the earlier design had no representation for, \
                  and it is the whole of the 2026-07-11 report: autosync stopped on a \
                  transient error, never re-armed, and said nothing — so the app looked \
                  stuck while its switch read on. \
                  \
                  Note what paused does NOT do. The checkbox stays ON, because the setting \
                  IS on — what stopped is the machinery, and a checkbox that lied about \
                  the setting would be worse. And it stays ENABLED, because flipping it \
                  off and on is one of only three ways to clear the pause today; \
                  disabling it would remove the user's only lever. \
                  \
                  It is a StateLabel rather than plain text, so the pause gets the tone's \
                  glyph and survives greyscale like every other attention state on the \
                  page. \
                  \
                  There is no Resume button, here or anywhere. All six PausedReason \
                  variants are things the USER must fix — RoleDenied says outright that \
                  retrying cannot help, and Other is documented as non-transient — so a \
                  resume control would offer to retry something that pauses again on the \
                  next tick. The fix always lives in the queue row that named the reason, \
                  and publishing or resolving clears the pause as a side effect. See the \
                  two paused scenes."
        >
            <Cell wide=true label="armed — a next tick exists">
                <Card title="Autosync">
                    <ToggleRow
                        label="Get new revisions"
                        sublabel="Every 30s, when nothing is changed here"
                        checked=pull
                        trailing=view! { "0:23" }.into_any()
                    />
                </Card>
            </Cell>
            <Cell wide=true label="idle — on, nothing to do">
                <Card title="Autosync">
                    <ToggleRow
                        label="Publish your changes"
                        sublabel="5 min after your last edit"
                        checked=publish
                        trailing=view! { "nothing to publish" }.into_any()
                    />
                </Card>
            </Cell>
            <Cell wide=true label="paused — on, and not operating">
                <Card title="Autosync">
                    <ToggleRow
                        label="Publish your changes"
                        sublabel="5 min after your last edit"
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
