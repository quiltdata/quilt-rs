//! `SplitButton` stories.

use leptos::prelude::*;

use crate::Cell;
use crate::Story;
use crate::kit::ButtonVariant;
use crate::kit::MenuAction;
use crate::kit::SplitButton;

fn alternatives() -> Vec<MenuAction> {
    vec![MenuAction::new(
        "Create new revision",
        Callback::new(|()| ()),
    )]
}

fn several() -> Vec<MenuAction> {
    vec![
        MenuAction::new("Create new revision", Callback::new(|()| ())),
        MenuAction::new("Publish without checks", Callback::new(|()| ())),
        MenuAction::new("Discard local changes", Callback::new(|()| ())).danger(),
    ]
}

#[component]
pub fn SplitButtonStories() -> impl IntoView {
    view! { <ShapeStory /><EdgesStory /> }
}

#[component]
fn ShapeStory() -> impl IntoView {
    view! {
        <Story
            title="SplitButton"
            note="One command on its face, its alternatives behind the caret. Open one: \
                  both halves are real buttons, so Tab stops at each in reading order and \
                  Escape or a click outside dismisses the surface — that is the \
                  platform's popover, not a hand-written keyboard model. \
                  \
                  The face's own command is never repeated inside the surface. A menu \
                  that lists what you just clicked reads as a mistake. \
                  \
                  The two halves share one border: the caret is pulled back a pixel so \
                  the seam is 1px like the outer edge, not 2px. Check the focus ring on \
                  the caret — it lifts above the face, or the shared edge would clip the \
                  half of it that falls on the seam. \
                  \
                  Which command is on the face is the caller's, not the component's — \
                  see the edges story below for what that buys a page."
        >
            <Cell wide=true label="primary — the common case">
                <SplitButton
                    label="Publish"
                    menu_label="Other ways to publish"
                    actions=alternatives()
                    variant=ButtonVariant::Primary
                    on_click=|_| ()
                />
            </Cell>
            <Cell wide=true label="default weight">
                <SplitButton
                    label="Publish"
                    menu_label="Other ways to publish"
                    actions=alternatives()
                    on_click=|_| ()
                />
            </Cell>
            <Cell wide=true label="several alternatives, one destructive">
                <SplitButton
                    label="Publish"
                    menu_label="Other ways to publish"
                    actions=several()
                    variant=ButtonVariant::Primary
                    on_click=|_| ()
                />
            </Cell>
        </Story>
    }
}

#[component]
fn EdgesStory() -> impl IntoView {
    let last = RwSignal::new("Publish".to_string());

    view! {
        <Story
            title="SplitButton — edges"
            note="Busy, unavailable, cramped, and the face swapped from outside. \
                  \
                  Loading and disabled both stop the caret as well as the face: work is \
                  in flight or the command is unavailable, so neither half should start \
                  more. A caret offering other ways to do something you cannot do is a \
                  dead end. \
                  \
                  The last cell is the one that matters for a page that wants to \
                  remember a choice. The face is a prop, so the page swaps it and the \
                  component never learns that preferences exist — the layering rule, and \
                  the reason this control can be reused."
        >
            <Cell wide=true label="working — the caret goes with the face">
                <SplitButton
                    label="Publishing…"
                    menu_label="Other ways to publish"
                    actions=alternatives()
                    variant=ButtonVariant::Primary
                    loading=true
                    on_click=|_| ()
                />
            </Cell>
            <Cell wide=true label="unavailable — both halves">
                <SplitButton
                    label="Publish"
                    menu_label="Other ways to publish"
                    actions=alternatives()
                    variant=ButtonVariant::Primary
                    disabled=true
                    on_click=|_| ()
                />
            </Cell>
            <Cell wide=true label="a long label shrinks rather than pushing the caret out">
                <div style="width:190px">
                    <SplitButton
                        label="Publish this revision to the bucket"
                        menu_label="Other ways to publish"
                        actions=alternatives()
                        variant=ButtonVariant::Primary
                        on_click=|_| ()
                    />
                </div>
            </Cell>
            <Cell
                wide=true
                label="the face is a prop — a page could persist this; the component cannot"
            >
                <SplitButton
                    label=Signal::derive(move || last.get())
                    menu_label="Other ways to publish"
                    actions=vec![
                        MenuAction::new(
                            "Create new revision",
                            Callback::new(move |()| last.set("Create new revision".to_string())),
                        ),
                        MenuAction::new(
                            "Publish",
                            Callback::new(move |()| last.set("Publish".to_string())),
                        ),
                    ]
                    variant=ButtonVariant::Primary
                    on_click=|_| ()
                />
            </Cell>
        </Story>
    }
}
