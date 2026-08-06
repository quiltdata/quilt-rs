//! One state, named and toned. The vocabulary's only visual form.
//!
//! # The label is the meaning; the tone is emphasis
//!
//! Colour is never the message. Every pill carries its words, so `No access` says
//! what it is with the stylesheet switched off — the tone only decides how loudly.
//! That ordering is what makes the ten states safe to render forty-three times on
//! one page.
//!
//! # Four tones, not ten
//!
//! Ten states collapse to four because the page only ever asks four questions of a
//! row: is it fine (`Ok`), is it merely reporting a number (`Neutral`), does it
//! want you (`Attention`), or is it broken (`Danger`). A fifth tone would have to
//! answer a question nothing asks.

use leptos::prelude::*;

stylance::import_crate_style!(style, "src/kit/state_pill.module.scss");

/// How loudly a state is stated.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StateTone {
    /// Nothing to do. `Latest`.
    Ok,
    /// A fact, not a problem. `2 files changed`.
    Neutral,
    /// Waiting on you. `Newer revision available`, `Not published yet`.
    Attention,
    /// Something is wrong and the row cannot fix it. `No access`, `conflicts in 2
    /// files`.
    Danger,
}

impl StateTone {
    fn class(self) -> &'static str {
        match self {
            Self::Ok => style::ok,
            Self::Neutral => style::neutral,
            Self::Attention => style::attention,
            Self::Danger => style::danger,
        }
    }

    /// The tone's silhouette. Intrinsic rather than a prop: a caller free to pass
    /// its own glyph is a caller free to put a tick on a `Danger` pill.
    ///
    /// `aria-hidden`, because it repeats the label — announcing "image, tick,
    /// Latest" is worse than "Latest".
    fn glyph(self) -> AnyView {
        match self {
            Self::Ok => view! {
                <svg class=style::glyph viewBox="0 0 16 16" aria-hidden="true" fill="none"
                    stroke="currentColor" stroke-width="2.2" stroke-linecap="round"
                    stroke-linejoin="round">
                    <path d="M3.2 8.6 6.3 11.7 12.8 4.6" />
                </svg>
            }
            .into_any(),
            Self::Neutral => view! {
                <svg class=style::glyph viewBox="0 0 16 16" aria-hidden="true"
                    fill="currentColor">
                    <circle cx="8" cy="8" r="3.4" />
                </svg>
            }
            .into_any(),
            Self::Attention => view! {
                <svg class=style::glyph viewBox="0 0 16 16" aria-hidden="true"
                    fill="currentColor">
                    <path d="M8 1.9 15.1 14.1H0.9Z" />
                </svg>
            }
            .into_any(),
            Self::Danger => view! {
                <svg class=style::glyph viewBox="0 0 16 16" aria-hidden="true" fill="none"
                    stroke="currentColor" stroke-width="2.2" stroke-linecap="round">
                    <path d="M4 4 12 12M12 4 4 12" />
                </svg>
            }
            .into_any(),
        }
    }
}

#[component]
pub fn StatePill(
    /// The state, in the page's words. From the vocabulary — `Latest`,
    /// `Newer revision available`, `Changed in both places` — not a status enum
    /// stringified, and not a sentence.
    #[prop(into)]
    label: String,
    tone: StateTone,
) -> impl IntoView {
    // One computed string: Leptos rejects two `class` attributes on an element.
    let class = format!("{} {}", style::root, tone.class());

    view! {
        // A `span`, with no `role` and no `aria-live`. It is text that happens to
        // have a box around it; the row's own semantics carry it. Announcing
        // changes would mean forty-three live regions.
        <span class=class>{tone.glyph()}<span>{label}</span></span>
    }
}
