//! A fact about a row, as an outlined badge.
//!
//! Primer's `Label`, beside [`StateLabel`](super::StateLabel): that one reports a
//! state and picks its glyph from the tone; this one carries metadata, so it has no
//! tone and no fill. Its glyph, if any, says what the fact is about. The tone
//! glyphs (tick, dot, triangle, cross) stay with states only.

use leptos::prelude::*;

stylance::import_crate_style!(style, "src/kit/label.module.scss");

#[component]
pub fn Label(
    /// A content glyph from [`icons`](super::icons), such as `icons::download`.
    #[prop(optional)]
    icon: Option<fn() -> AnyView>,
    children: Children,
) -> impl IntoView {
    view! { <span class=style::root>{icon.map(|draw| draw())}<span>{children()}</span></span> }
}
