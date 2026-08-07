//! A placeholder for content that is genuinely unknown.
//!
//! # Only for unknown, never for provisional
//!
//! The light phase already returns a status and the heavy walk merely *corrects* it, so
//! a `StateLabel` is never unknown — skeletonising one would hide information we already
//! have and then reveal the same value. Skeletons belong to the window **before the light
//! phase resolves**, where the row count and the contents are genuinely not known yet.
//!
//! A row whose state is provisional renders dimmed and settles instead. Rows settling at
//! different times *is* the activity signal.
//!
//! # It is the region that is busy, not the placeholder
//!
//! Each `SkeletonBox` is `aria-hidden`, because a bar has nothing to announce. The
//! **composing region** sets `aria-busy="true"` and drops it when the content arrives.
//! That split is easy to get backwards, and getting it backwards means a screen reader
//! reads out a dozen nameless boxes.
//!
//! # Height is the whole job
//!
//! A skeleton row that is not exactly as tall as the real row makes the page jump when it
//! settles — which is worse than no skeleton, because the reflow arrives at the moment the
//! user has started reading. The gallery puts a skeleton row directly above the real row
//! it stands in for, so any mismatch is visible rather than theoretical.

use leptos::prelude::*;

stylance::import_crate_style!(style, "src/kit/skeleton_box.module.scss");

#[component]
pub fn SkeletonBox(
    /// Any CSS width — `"40%"`, `"120px"`. Required, because a placeholder with no width
    /// is not a placeholder. Percentages are usually right: a skeleton stands in for a
    /// namespace of unknown length, and a fixed width would claim to know it.
    #[prop(into)]
    width: String,
    /// Defaults to a text bar. Override for a block — a card, an avatar, a chip.
    #[prop(optional, into)]
    height: Option<String>,
) -> impl IntoView {
    let height = height.unwrap_or_else(|| "12px".to_string());

    view! {
        // Two `style:` bindings rather than one formatted `style` attribute. Leptos takes
        // each by value, which is what a prop has to be for a `'static` view — and clippy
        // is right that a `String` read only inside `format!` was never consumed.
        <span class=style::root style:width=width style:height=height aria-hidden="true" />
    }
}
