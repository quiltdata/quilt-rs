//! One revision: what it says, and when.
//!
//! # No label, no location
//!
//! A caller that wants `YOURS` above it wraps it in a
//! [`PaneSection`](super::PaneSection) — the label belongs to the block, which
//! is why the same component serves the current revision, the two sides of a
//! resolve, and every row of the revisions overlay, where there are no labels at
//! all.
//!
//! The bucket is likewise absent. It is a fact about the *package*, so inside a
//! list of revisions it would repeat itself identically on every row.
//!
//! # No hash
//!
//! A revision is named by its message and its time. The identity the page
//! exposes is deliberately human, and a hash is the thing users were reading
//! when they could not tell two revisions apart.

use leptos::prelude::*;

use super::RelativeTime;
use super::countdown::EpochMillis;

stylance::import_crate_style!(style, "src/kit/revision_row.module.scss");

#[component]
pub fn RevisionRow(
    /// The commit message. Quoted by the component rather than by the caller, so
    /// four call sites cannot quote it four ways.
    #[prop(into)]
    message: String,
    /// When this copy obtained it.
    at: EpochMillis,
) -> impl IntoView {
    // An empty message is reachable — nothing stops a publish without one — and
    // it must not render as a pair of bare quotes.
    let quoted = {
        let trimmed = message.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(format!("\u{201c}{trimmed}\u{201d}"))
        }
    };
    // The whole value rides in `title`, since the visible one is ellipsised.
    let full = message;
    let body = match quoted {
        Some(text) => view! { <span class=style::message title=full>{text}</span> }.into_any(),
        None => view! { <span class=style::empty>"No message"</span> }.into_any(),
    };

    view! {
        <div class=style::root>
            {body}
            <span class=style::when>
                <RelativeTime at=at />
            </span>
        </div>
    }
}
