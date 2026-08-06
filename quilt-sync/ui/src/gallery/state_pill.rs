//! `StatePill` stories — the whole state vocabulary, which is the reason the
//! gallery exists at all.

use leptos::prelude::*;

use crate::Cell;
use crate::Story;
use crate::kit::StatePill;
use crate::kit::StateTone;

/// Every state the page can put on a row, in the order the vocabulary lists them.
/// Reviewing this list *is* reviewing the vocabulary: a label that reads badly here
/// reads badly on the page.
const STATES: &[(&str, StateTone)] = &[
    ("Latest", StateTone::Ok),
    ("Not the latest", StateTone::Attention),
    ("Newer revision available", StateTone::Attention),
    ("2 files changed", StateTone::Neutral),
    ("conflicts in 2 files", StateTone::Danger),
    ("Changed in both places", StateTone::Danger),
    ("No access", StateTone::Danger),
    ("No S3 bucket yet", StateTone::Attention),
    ("Not published yet", StateTone::Attention),
    ("Revision not published", StateTone::Attention),
];

fn tone_name(tone: StateTone) -> &'static str {
    match tone {
        StateTone::Ok => "Ok",
        StateTone::Neutral => "Neutral",
        StateTone::Attention => "Attention",
        StateTone::Danger => "Danger",
    }
}

#[component]
pub fn StatePillStories() -> impl IntoView {
    view! { <VocabularyStory /><GreyscaleStory /> }
}

#[component]
fn VocabularyStory() -> impl IntoView {
    view! {
        <Story
            title="StatePill — the ten states"
            note="Read the words, not the colours. Each label is what the page says out loud, \
                  and the tone only sets how loudly. Steps 3 / 7 / 12 of the tone's scale, in \
                  that order: fill, edge, text. Text is step 12 rather than the scale's own \
                  text step because 11 on 3 measures 4.21:1 for green and 4.25:1 for amber — \
                  under AA. Step 12 measures 10.5:1 or better in both themes."
        >
            {STATES
                .iter()
                .map(|&(label, tone)| {
                    view! {
                        <Cell label=tone_name(tone)>
                            <StatePill label=label tone=tone />
                        </Cell>
                    }
                })
                .collect_view()}
        </Story>
    }
}

#[component]
fn GreyscaleStory() -> impl IntoView {
    // A gallery-only filter, inline because nothing in the kit should own it. It is
    // the cheapest honest test of "does this survive without hue" — and it is
    // stricter than any colourblindness simulation, since it removes all three.
    let grey = "filter: grayscale(1)";
    let row = "display: flex; flex-wrap: wrap; gap: var(--q-space-2)";

    view! {
        <Story
            title="StatePill — tone without hue"
            note="Tier 1 lightness-matches step 11 across hues deliberately, so desaturating \
                  the four tones leaves four near-identical pills. That is why each tone \
                  carries a silhouette — tick, dot, triangle, cross. Compare the two cells: \
                  the greyscale one must still be readable at a glance. If it is not, the \
                  glyphs are wrong."
        >
            <Cell label="in colour" wide=true>
                <div style=row>
                    <StatePill label="Latest" tone=StateTone::Ok />
                    <StatePill label="2 files changed" tone=StateTone::Neutral />
                    <StatePill label="Not published yet" tone=StateTone::Attention />
                    <StatePill label="No access" tone=StateTone::Danger />
                </div>
            </Cell>
            <Cell label="hue removed — the glyph is all that is left" wide=true>
                <div style=format!("{row}; {grey}")>
                    <StatePill label="Latest" tone=StateTone::Ok />
                    <StatePill label="2 files changed" tone=StateTone::Neutral />
                    <StatePill label="Not published yet" tone=StateTone::Attention />
                    <StatePill label="No access" tone=StateTone::Danger />
                </div>
            </Cell>
            <Cell label="beside a Latest run — what 43 rows actually look like" wide=true>
                <div style=row>
                    {(0..7)
                        .map(|i| {
                            let (label, tone) = if i == 3 {
                                ("Changed in both places", StateTone::Danger)
                            } else {
                                ("Latest", StateTone::Ok)
                            };
                            view! { <StatePill label=label tone=tone /> }
                        })
                        .collect_view()}
                </div>
            </Cell>
            <Cell label="does not truncate — it pushes">
                <StatePill label="Newer revision available" tone=StateTone::Attention />
            </Cell>
        </Story>
    }
}
