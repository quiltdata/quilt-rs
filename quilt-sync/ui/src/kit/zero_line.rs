//! The healthy queue: one line where a region would be.
//!
//! # Why it is not a `Blankslate`
//!
//! With autosync working this is the common case, seen most days by most users — so
//! it is the state that decides whether the page reads as calm or as unfinished. A
//! full-height empty state here would push the package list below the fold to
//! announce that nothing is wrong, which is the loudest possible way to say it.
//!
//! Acceptance criterion 8: it **must stay one line**.

use leptos::prelude::*;

use super::SkeletonBox;
use super::state_label::StateTone;

stylance::import_crate_style!(style, "src/kit/zero_line.module.scss");

#[component]
pub fn ZeroLine(
    /// The whole line — `Everything is Latest — 43 packages`. One string rather than
    /// text plus a count, because the singular case is not a plural rule the
    /// component can apply: "1 package" and "43 packages" differ, and so might the
    /// phrasing the page wants around them.
    #[prop(into)]
    text: String,
) -> impl IntoView {
    view! {
        <p class=style::root>
            // The Success tone's own tick, from `StateTone`, rather than a second
            // one drawn here. Two components drawing two different ticks is how a
            // silhouette stops being a signal.
            {StateTone::Success.glyph()}
            <span class=style::text>{text}</span>
        </p>
    }
}

/// The zero line's geometry, while the page is still being read.
///
/// The queue had no placeholder in the shared fallback, so the list started high
/// and jumped down when the queue arrived. Shaped on the zero line rather than a
/// cause card: the all-clear is what most loads resolve to, and reserving a
/// card's height would trade this shift for an upward one on every healthy day
/// (qhq-8mgw.55).
#[component]
pub fn ZeroLineSkeleton() -> impl IntoView {
    view! {
        <p class=style::placeholder>
            <SkeletonBox width="14px" height="14px" />
            <span class=style::text>
                <SkeletonBox width="240px" />
            </span>
        </p>
    }
}
