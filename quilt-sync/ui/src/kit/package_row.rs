//! One installed package, as a list row.
//!
//! # The whole row is one link
//!
//! An `<a>`, not a `div` with a click handler, and that is the difference from
//! [`FileRow`](super::FileRow) rather than a detail of it. A package is a *place
//! you go*; a file is a *thing you act on*. So this row has one destination and no
//! controls at all — which is what lets it be a real anchor, with middle-click,
//! open-in-new-tab, the status bar preview and keyboard activation for free.
//!
//! `FileRow` cannot do that: it has two destinations and three actions, and nested
//! `<a>` is invalid, so it pays for its children with a `role="button"` div.
//!
//! # Plain at rest
//!
//! No link colour, no chevron, no underline — not even on hover, where the row's
//! background tint is the whole affordance. Link colouring is a document
//! convention; this is an application, where a row is an object. And a tint on
//! every row at rest stops signalling anything: forty tinted lines read as the
//! list's typeface, not as forty invitations.

use leptos::prelude::*;

use super::RelativeTime;
use super::SkeletonBox;
use super::StateLabel;
use super::countdown::EpochMillis;
use super::state_label::StateTone;

stylance::import_crate_style!(style, "src/kit/package_row.module.scss");

#[component]
pub fn PackageRow(
    /// `owner/name`. Truncates from the **right**, unlike `FileRow`'s path: a
    /// namespace is distinguished by its start, and there is no filename at the end
    /// worth saving.
    #[prop(into)]
    namespace: String,
    /// The package page. A real `href`, so the browser owns the navigation.
    #[prop(into)]
    href: String,
    /// When the package last changed. `None` prints an explicit word rather than
    /// leaving the column blank — it should not happen, and a blank cell is
    /// indistinguishable from a cell that failed to render.
    #[prop(optional, into)]
    changed_at: MaybeProp<EpochMillis>,
    /// The state, in the page's words, and its tone. Two props rather than one
    /// struct: the caller maps a DTO status to this pair in one place, and a struct
    /// would only move the pairing without checking it.
    #[prop(into)]
    state: String,
    tone: StateTone,
    /// Passed through to the state label: the light phase's guess, awaiting the heavy walk.
    ///
    /// The LIST shows provisional states; the QUEUE does not, which is why `QueueRow` has
    /// no such prop. Adding a row to "needs your attention" on a guess and removing it a
    /// second later is worse than an empty queue for that second — the queue is where the
    /// user decides, and a decision offered and withdrawn is not a decision.
    #[prop(optional, into)]
    provisional: MaybeProp<bool>,
) -> impl IntoView {
    view! {
        <a class=style::root href=href>
            <span class=style::namespace>{namespace}</span>
            <span class=style::time>
                {move || changed_at
                    .get()
                    .map_or_else(
                        || {
                            view! {
                                // "unknown" read as a bare word with no column header to
                                // explain it — a reader could not tell what it referred
                                // to. This says which fact is missing. Still a word and
                                // not a blank: a blank cell is indistinguishable from one
                                // that failed to render.
                                <span title="This package has no recorded change time">
                                    "not recorded"
                                </span>
                            }
                                .into_any()
                        },
                        |at| view! { <RelativeTime at=at /> }.into_any(),
                    )}
            </span>
            <StateLabel tone=tone provisional=provisional>{state}</StateLabel>
        </a>
    }
}

/// The same row with its content unknown.
///
/// It reuses `PackageRow`'s own `.root` class rather than restating the padding, which is
/// the only way the two are guaranteed to be the same height — and equal height is the
/// whole point. A skeleton a few pixels shorter than the row it stands in for makes the
/// list jump at the exact moment the user starts reading it.
///
/// `.skeleton` switches off the affordance: no pointer cursor and no hover tint, because
/// nothing here responds to a click.
#[component]
pub fn PackageRowSkeleton() -> impl IntoView {
    view! {
        <div class=format!("{} {}", style::root, style::skeleton)>
            <span class=style::namespace>
                <SkeletonBox width="38%" />
            </span>
            <span class=style::time>
                <SkeletonBox width="100%" />
            </span>
            // 88px is roughly `Latest` in a state label. A percentage would be wrong here:
            // the label's width is set by its words, not by the row.
            <SkeletonBox width="88px" height="22px" />
        </div>
    }
}
