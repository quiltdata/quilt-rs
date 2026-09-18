//! `SplitButton` stories.

use leptos::prelude::*;

use crate::Cell;
use crate::Story;
use crate::kit::ButtonVariant;
use crate::kit::SplitButton;
use crate::kit::SplitOption;

fn publish_or_revision() -> Vec<SplitOption> {
    vec![
        SplitOption::new("Publish", Callback::new(|()| ())),
        SplitOption::new("Create new revision", Callback::new(|()| ())),
    ]
}

fn three() -> Vec<SplitOption> {
    vec![
        SplitOption::new("Publish", Callback::new(|()| ())),
        SplitOption::new("Create new revision", Callback::new(|()| ())),
        SplitOption::new("Publish without checks", Callback::new(|()| ())),
    ]
}

#[component]
pub fn SplitButtonStories() -> impl IntoView {
    view! { <ShapeStory /><EdgesStory /> }
}

#[component]
fn ShapeStory() -> impl IntoView {
    let live = RwSignal::new(0_usize);
    let three_way = RwSignal::new(0_usize);

    view! {
        <Story
            title="SplitButton"
            note="The face is whichever option is selected; the caret opens the whole set \
                  with that one ticked. Open the live cell and pick the other option: the \
                  face changes and nothing runs. Picking sets the default; the face runs \
                  it. A menu you opened to look at your options must not publish. \
                  \
                  That is also why the menu lists the option already on the face. It is \
                  where you see which one is active, so leaving the active one out would \
                  leave out the point of opening it. \
                  \
                  Both halves and every option are real buttons: Tab stops at each in \
                  reading order, Escape and a click outside dismiss. No arrow keys, and \
                  no `role=menu` claiming there are — the active option carries \
                  `aria-current`, which says which one rather than how to move between \
                  them. \
                  \
                  The two halves share one border: the caret is pulled back a pixel so \
                  the seam is 1px like the outer edge, not 2px. The surface hangs from \
                  the caret's right edge, because a trailing control has no room to grow \
                  rightwards."
        >
            <Cell wide=true label="live — pick the other option, watch the face">
                <SplitButton
                    options=publish_or_revision()
                    selected=live
                    menu_label="Change what this button does"
                    variant=ButtonVariant::Primary
                />
            </Cell>
            <Cell wide=true label="default weight">
                <SplitButton
                    options=publish_or_revision()
                    selected=RwSignal::new(0)
                    menu_label="Change what this button does"
                />
            </Cell>
            <Cell wide=true label="three options">
                <SplitButton
                    options=three()
                    selected=three_way
                    menu_label="Change what this button does"
                    variant=ButtonVariant::Primary
                />
            </Cell>
        </Story>
    }
}

#[component]
fn EdgesStory() -> impl IntoView {
    view! {
        <Story
            title="SplitButton — edges"
            note="Busy, unavailable, cramped, and a stored selection that no longer \
                  exists. \
                  \
                  Loading and disabled both stop the caret as well as the face: work is \
                  in flight or the command is unavailable, so neither half should start \
                  more, and a caret offering other ways to do something you cannot do is \
                  a dead end. \
                  \
                  The last cell is a persisted index pointing past the end of the list — \
                  what a preference saved by an older build looks like once the options \
                  have changed. It falls back to the first option rather than taking the \
                  page down."
        >
            <Cell wide=true label="working — the caret goes with the face">
                <SplitButton
                    options=publish_or_revision()
                    selected=RwSignal::new(0)
                    menu_label="Change what this button does"
                    variant=ButtonVariant::Primary
                    loading=true
                />
            </Cell>
            <Cell wide=true label="unavailable — both halves">
                <SplitButton
                    options=publish_or_revision()
                    selected=RwSignal::new(0)
                    menu_label="Change what this button does"
                    variant=ButtonVariant::Primary
                    disabled=true
                />
            </Cell>
            <Cell wide=true label="a long label shrinks rather than pushing the caret out">
                <div style="width:190px">
                    <SplitButton
                        options=vec![
                            SplitOption::new(
                                "Publish this revision to the bucket",
                                Callback::new(|()| ()),
                            ),
                            SplitOption::new("Create new revision", Callback::new(|()| ())),
                        ]
                        selected=RwSignal::new(0)
                        menu_label="Change what this button does"
                        variant=ButtonVariant::Primary
                    />
                </div>
            </Cell>
            <Cell wide=true label="a stored selection that no longer exists — falls back">
                <SplitButton
                    options=publish_or_revision()
                    selected=RwSignal::new(7)
                    menu_label="Change what this button does"
                    variant=ButtonVariant::Primary
                />
            </Cell>
        </Story>
    }
}
