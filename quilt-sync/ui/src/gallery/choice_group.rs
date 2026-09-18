//! `ChoiceGroup` stories.

use leptos::prelude::*;

use crate::Cell;
use crate::Story;
use crate::kit::Choice;
use crate::kit::ChoiceGroup;

fn scopes() -> Vec<Choice> {
    vec![
        Choice::new("pick", "Files I pick"),
        Choice::new("all", "The whole package"),
    ]
}

#[component]
pub fn ChoiceGroupStories() -> impl IntoView {
    let keeping = RwSignal::new("pick".to_string());
    let second = RwSignal::new("all".to_string());
    let frozen = RwSignal::new("pick".to_string());
    let theme = RwSignal::new("system".to_string());

    // The consequence, in the present tense, changing with the choice — the whole
    // reason the caption belongs to the group rather than sitting beside it.
    let consequence = Signal::derive(move || {
        if keeping.get() == "all" {
            "54 of 56 downloaded — files added later are downloaded too.".to_string()
        } else {
            "54 of 56 downloaded.".to_string()
        }
    });

    view! {
        <Story
            title="ChoiceGroup"
            note="A named set of radios with the consequence of the current choice under it. \
                  Pick in the first group and watch its caption change, then pick in the \
                  second and check the first did not clear: the `name` is generated, not \
                  passed, after five SegmentedControls sharing a literal once became one \
                  radiogroup. Not a ToggleRow, which is one setting with two states. The \
                  last cell is the same component with none of this page's words in it."
        >
            <Cell wide=true label="the Keeping block — caption tracks the choice">
                <ChoiceGroup
                    label="Keeping"
                    caption=consequence
                    options=scopes()
                    selected=keeping
                />
            </Cell>
            <Cell wide=true label="a second instance — proves the generated name">
                <ChoiceGroup label="Keeping" options=scopes() selected=second />
            </Cell>
            <Cell wide=true label="a third option, and none of this page's words">
                <ChoiceGroup
                    label="Appearance"
                    caption="Follows your system setting until you pick one."
                    options=vec![
                        Choice::new("system", "Match system"),
                        Choice::new("light", "Light"),
                        Choice::new("dark", "Dark"),
                    ]
                    selected=theme
                />
            </Cell>
            <Cell wide=true label="disabled">
                <ChoiceGroup
                    label="Keeping"
                    caption="Signed out — sign in to change this."
                    options=scopes()
                    selected=frozen
                    disabled=true
                />
            </Cell>
        </Story>
    }
}
