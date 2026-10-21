//! A fact about a row, as a badge.
//!
//! Primer's `Label`, beside [`StateLabel`](super::StateLabel): that one reports a
//! state and picks its glyph from the tone; this one carries metadata, so it has no
//! state tone. Its glyph, if any, says what the fact is about. The tone glyphs
//! (tick, dot, triangle, cross) stay with states only.
//!
//! [`LabelVariant`] only sets how loud the fact is: an outline by default, or the
//! accent fill for a fact the reader may want to act on.

use leptos::prelude::*;

stylance::import_crate_style!(style, "src/kit/label.module.scss");

/// How loud a [`Label`] is. Not a state tone: it says nothing about health.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LabelVariant {
    /// Grey outline, no fill.
    #[default]
    Outline,
    /// Filled with the accent colour, for a fact the reader may act on.
    Accent,
}

impl LabelVariant {
    fn class(self) -> &'static str {
        match self {
            Self::Outline => style::outline,
            Self::Accent => style::accent,
        }
    }
}

#[component]
pub fn Label(
    /// A content glyph from [`icons`](super::icons), such as `icons::download`.
    #[prop(optional)]
    icon: Option<fn() -> AnyView>,
    #[prop(optional)] variant: LabelVariant,
    children: Children,
) -> impl IntoView {
    let class = format!("{} {}", style::root, variant.class());
    view! { <span class=class>{icon.map(|draw| draw())}<span>{children()}</span></span> }
}
