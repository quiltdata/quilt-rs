//! `StateLabel` stories — the whole state vocabulary, which is the reason the
//! gallery exists at all.

use leptos::prelude::*;

use crate::Cell;
use crate::Story;
use crate::kit::StateLabel;
use crate::kit::StateTone;

/// Every state the page can put on a row, in the order the vocabulary lists them.
/// Reviewing this list *is* reviewing the vocabulary: words that read badly here
/// read badly on the page.
const STATES: &[(&str, StateTone)] = &[
    ("Latest", StateTone::Success),
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
        StateTone::Success => "Success",
        StateTone::Neutral => "Neutral",
        StateTone::Attention => "Attention",
        StateTone::Danger => "Danger",
    }
}

#[component]
pub fn StateLabelStories() -> impl IntoView {
    view! { <VocabularyStory /><GreyscaleStory /> }
}

#[component]
fn VocabularyStory() -> impl IntoView {
    view! {
        <Story
            title="StateLabel — the ten states"
            note="Read the words, not the colours. Each one is what the page says out loud, and \
                  the tone only sets how loudly. Steps 3 / 7 / 12 of the tone's scale, in that \
                  order: fill, edge, text. Text is step 12 rather than the scale's own text \
                  step because 11 on 3 measures 4.21:1 for green and 4.25:1 for amber — under \
                  AA. Step 12 measures 10.5:1 or better in both themes."
        >
            {STATES
                .iter()
                .map(|&(state, tone)| {
                    view! {
                        <Cell label=tone_name(tone)>
                            <StateLabel tone=tone>{state}</StateLabel>
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
            title="StateLabel — tone without hue"
            note="Tier 1 lightness-matches step 11 across hues deliberately, so desaturating \
                  the four tones leaves four near-identical boxes. That is why each tone \
                  carries a silhouette — tick, dot, triangle, cross — and why the tone picks \
                  it rather than the caller. Compare the two cells: the greyscale one must \
                  still be readable at a glance. If it is not, the glyphs are wrong."
        >
            <Cell label="in colour" wide=true>
                <div style=row>
                    <StateLabel tone=StateTone::Success>"Latest"</StateLabel>
                    <StateLabel tone=StateTone::Neutral>"2 files changed"</StateLabel>
                    <StateLabel tone=StateTone::Attention>"Not published yet"</StateLabel>
                    <StateLabel tone=StateTone::Danger>"No access"</StateLabel>
                </div>
            </Cell>
            <Cell label="hue removed — the glyph is all that is left" wide=true>
                <div style=format!("{row}; {grey}")>
                    <StateLabel tone=StateTone::Success>"Latest"</StateLabel>
                    <StateLabel tone=StateTone::Neutral>"2 files changed"</StateLabel>
                    <StateLabel tone=StateTone::Attention>"Not published yet"</StateLabel>
                    <StateLabel tone=StateTone::Danger>"No access"</StateLabel>
                </div>
            </Cell>
            <Cell label="beside a Latest run — what 43 rows actually look like" wide=true>
                <div style=row>
                    {(0..7)
                        .map(|i| {
                            let (state, tone) = if i == 3 {
                                ("Changed in both places", StateTone::Danger)
                            } else {
                                ("Latest", StateTone::Success)
                            };
                            view! { <StateLabel tone=tone>{state}</StateLabel> }
                        })
                        .collect_view()}
                </div>
            </Cell>
            <Cell label="an interpolated count — why children, not a label prop">
                <StateLabel tone=StateTone::Danger>
                    {format!("conflicts in {} files", 2)}
                </StateLabel>
            </Cell>
            <Cell label="does not truncate — it pushes">
                <StateLabel tone=StateTone::Attention>"Newer revision available"</StateLabel>
            </Cell>
        </Story>
    }
}
