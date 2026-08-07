//! One state, named and toned. The vocabulary's only visual form.
//!
//! # The name
//!
//! Primer's, and for its reason rather than for the alignment. Primer has two
//! neighbours: `Label` carries arbitrary metadata and lets the caller pick a
//! colour, while `StateLabel` reports a state and **selects its own icon from the
//! state**. The second is this component's contract — see [`StateTone::glyph`] —
//! so it takes the second name. `Pill` named the shape, which would have become a
//! lie the moment the shape changed.
//!
//! The tone words are Primer's `Label` variants (`success`, `attention`,
//! `danger`), which the tier-2 tokens already used. Primer has a fifth, `severe`,
//! between attention and danger; nothing in the ten states needs it, so no orange
//! scale is vendored for a tone with no occupant.
//!
//! # The words are the meaning; the tone is emphasis
//!
//! Colour is never the message. Every label carries its words, so `No access` says
//! what it is with the stylesheet switched off — the tone only decides how loudly.
//! That ordering is what makes the ten states safe to render forty-three times on
//! one page.
//!
//! # Four tones, not ten
//!
//! Ten states collapse to four because the page only ever asks four questions of a
//! row: is it fine (`Success`), is it merely reporting a number (`Neutral`), does
//! it want you (`Attention`), or is it broken (`Danger`). A fifth tone would have
//! to answer a question nothing asks.

use leptos::prelude::*;

stylance::import_crate_style!(style, "src/kit/state_label.module.scss");

/// How loudly a state is stated.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StateTone {
    /// Nothing to do. `Latest`.
    Success,
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
            Self::Success => style::success,
            Self::Neutral => style::neutral,
            Self::Attention => style::attention,
            Self::Danger => style::danger,
        }
    }

    /// The tone's silhouette. Selected by the tone, never passed in — which is the
    /// whole of Primer's distinction between `StateLabel` and `Label`. A caller
    /// free to pass its own glyph is a caller free to put a tick on a `Danger`.
    ///
    /// Public because the silhouette belongs to the **tone**, not to this component:
    /// `CauseRow` states a tone too, and two components drawing two different ticks
    /// for `Success` would undo the greyscale channel this exists for.
    ///
    /// Carries no class, so each consumer sizes and colours it with its own
    /// `.root svg` rule — 12px here, 14px in a cause row.
    ///
    /// `aria-hidden`, because it repeats the words — announcing "image, tick,
    /// Latest" is worse than "Latest".
    #[must_use]
    pub fn glyph(self) -> AnyView {
        match self {
            Self::Success => view! {
                <svg viewBox="0 0 16 16" aria-hidden="true" fill="none"
                    stroke="currentColor" stroke-width="2.2" stroke-linecap="round"
                    stroke-linejoin="round">
                    <path d="M3.2 8.6 6.3 11.7 12.8 4.6" />
                </svg>
            }
            .into_any(),
            Self::Neutral => view! {
                <svg viewBox="0 0 16 16" aria-hidden="true" fill="currentColor">
                    <circle cx="8" cy="8" r="3.4" />
                </svg>
            }
            .into_any(),
            Self::Attention => view! {
                <svg viewBox="0 0 16 16" aria-hidden="true" fill="currentColor">
                    <path d="M8 1.9 15.1 14.1H0.9Z" />
                </svg>
            }
            .into_any(),
            Self::Danger => view! {
                <svg viewBox="0 0 16 16" aria-hidden="true" fill="none"
                    stroke="currentColor" stroke-width="2.2" stroke-linecap="round">
                    <path d="M4 4 12 12M12 4 4 12" />
                </svg>
            }
            .into_any(),
        }
    }
}

#[component]
pub fn StateLabel(
    tone: StateTone,
    /// The state, in the page's words — `Latest`, `Newer revision available`,
    /// `Changed in both places`. From the vocabulary, not a status enum
    /// stringified, and not a sentence.
    ///
    /// Children rather than a `label` prop, as in Primer. Two reasons beyond
    /// matching: `label` means *accessible name* everywhere else in this kit, and
    /// three of the ten states interpolate a count, which children take without a
    /// `format!` at the call site.
    children: Children,
    /// The light phase's guess, not yet confirmed by the heavy walk. Draws a dashed edge
    /// and settles to solid when the real value arrives.
    ///
    /// **Dashed rather than dimmed**, which the design record left open. Dimming costs
    /// contrast on text that was measured to 9.2:1 at worst and would drop it below AA at
    /// any useful opacity; a dashed edge costs none. That matters because a provisional
    /// state is still *informative* — it is the light phase's answer, right most of the
    /// time — so making it harder to read to say "not final" trades the wrong thing.
    #[prop(optional, into)]
    provisional: MaybeProp<bool>,
) -> impl IntoView {
    let is_provisional = Signal::derive(move || provisional.get().unwrap_or(false));

    // One computed string: Leptos rejects two `class` attributes on an element.
    let class = move || {
        let base = format!("{} {}", style::root, tone.class());
        if is_provisional.get() {
            format!("{base} {}", style::provisional)
        } else {
            base
        }
    };

    view! {
        // A `span`, with no `role` and no `aria-live`. It is text that happens to
        // have a box around it; the row's own semantics carry it. Announcing
        // changes would mean forty-three live regions.
        <span class=class>{tone.glyph()}<span>{children()}</span></span>
    }
}
