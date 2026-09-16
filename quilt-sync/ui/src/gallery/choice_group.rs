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
            note="A named set of radios with the consequence of the current choice under \
                  it. Two things belong to the set and not to any one option: the shared \
                  name that makes the radios exclusive, and the caption — a scope and its \
                  effect have to read as one statement. \
                  \
                  Pick in the first group and watch its caption change; then pick in the \
                  second and check the first did not clear. The name is generated, not \
                  passed: a string that has to be unique across the document is a \
                  precondition no call site can check, and five SegmentedControls sharing a \
                  literal once became one radiogroup. No kit component asks a caller for a \
                  globally unique string. \
                  \
                  Not a ToggleRow: that is an independent setting with two states, this is \
                  a choice between alternatives where exactly one holds."
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
