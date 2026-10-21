//! `Label` stories: its two variants, and which glyph it carries.

use leptos::prelude::*;

use crate::Cell;
use crate::Story;
use crate::kit::Label;
use crate::kit::LabelVariant;
use crate::kit::StateLabel;
use crate::kit::StateTone;
use crate::kit::icons;

#[component]
pub fn LabelStories() -> impl IntoView {
    view! {
        <VariantsStory />
        <GlyphChoiceStory />
    }
}

/// The two variants beside the state labels they sit with on a package row.
#[component]
fn VariantsStory() -> impl IntoView {
    let cells: [(&str, &str, LabelVariant, &str, StateTone); 4] = [
        (
            "accent · beside local changes",
            "3 to download",
            LabelVariant::Accent,
            "2 files changed",
            StateTone::Neutral,
        ),
        (
            "accent · beside a newer remote revision",
            "3 to download",
            LabelVariant::Accent,
            "Not the latest",
            StateTone::Attention,
        ),
        (
            "outline · beside Latest",
            "12 of 340 files",
            LabelVariant::Outline,
            "Latest",
            StateTone::Success,
        ),
        (
            "outline · beside a newer remote revision",
            "12 of 340 files",
            LabelVariant::Outline,
            "Not the latest",
            StateTone::Attention,
        ),
    ];

    view! {
        <Story
            title="Label — variants"
            note="A fact, not a state, so neither variant is a state tone. The outline is \
                  the default. The accent fill is for a fact the reader may act on, such \
                  as files left to download. Same box as StateLabel."
        >
            {cells
                .into_iter()
                .map(|(label, words, variant, state, tone)| {
                    view! {
                        <Cell wide=true label=label>
                            <div class="g-inline">
                                <Label icon=icons::download variant=variant>
                                    {words}
                                </Label>
                                <StateLabel tone=tone>{state}</StateLabel>
                            </div>
                        </Cell>
                    }
                })
                .collect_view()}
        </Story>
    }
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
            note="A badge with no state tone, for a fact rather than a state. Its \
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
                                    <Label icon=icon variant=LabelVariant::Accent>
                                        "3 to download"
                                    </Label>
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
