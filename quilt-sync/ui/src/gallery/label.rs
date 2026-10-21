//! `Label` stories: the outlined badge, and which glyph it carries.

use leptos::prelude::*;

use crate::Cell;
use crate::Story;
use crate::kit::Label;
use crate::kit::StateLabel;
use crate::kit::StateTone;
use crate::kit::icons;

#[component]
pub fn LabelStories() -> impl IntoView {
    view! { <GlyphChoiceStory /> }
}

/// Both download markers beside the state labels they sit with on a row, once per
/// glyph, so the two can be compared.
type Glyph = fn() -> AnyView;

#[component]
fn GlyphChoiceStory() -> impl IntoView {
    let choices: [(&str, &str, Glyph); 2] = [
        (
            "down-arrow · whole package",
            "down-arrow · picked files",
            icons::download,
        ),
        ("file · whole package", "file · picked files", icons::file),
    ];

    view! {
        <Story
            title="Label — glyph choice"
            note="An outline with no fill and no tone, for a fact rather than a state. Its \
                  glyph says what the fact is about. The tone glyphs stay with StateLabel. \
                  Two candidates for the download markers, each beside the state label it \
                  sits with on a row; `PackageRow` draws the down-arrow."
        >
            {choices
                .into_iter()
                .flat_map(|(whole, picked, icon)| {
                    [
                        view! {
                            <Cell wide=true label=whole>
                                <div class="g-inline">
                                    <Label icon=icon>"3 to download"</Label>
                                    <StateLabel tone=StateTone::Neutral>
                                        "2 files changed"
                                    </StateLabel>
                                </div>
                            </Cell>
                        },
                        view! {
                            <Cell wide=true label=picked>
                                <div class="g-inline">
                                    <Label icon=icon>"12 of 340 files"</Label>
                                    <StateLabel tone=StateTone::Success>"Latest"</StateLabel>
                                </div>
                            </Cell>
                        },
                    ]
                })
                .collect_view()}
        </Story>
    }
}
