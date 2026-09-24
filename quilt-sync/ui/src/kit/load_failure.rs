//! A read that did not answer, and the one way to ask again.
//!
//! # Not a `Blankslate`
//!
//! A [`Blankslate`](super::Blankslate) says there is nothing to show. This says
//! we could not find out. `No files yet` over a read that never returned
//! manufactures a state the page does not know, which is worse than a page that
//! says less.
//!
//! # The sentence is the UI's; a detail may be the backend's
//!
//! `words` is fixed copy the caller owns. A `Result<_, String>` error text is not
//! a word from the vocabulary: it cannot be reviewed, it cannot be changed
//! without a backend release, and no mechanical test can see it. So it never
//! stands in for the sentence, and by default it is for the log.
//!
//! Where the spec calls for the reason to be shown, a caller passes it as
//! `detail`, as resolve mode does for a refused comparison. It is drawn under
//! the sentence, muted, and before the button: the reader learns what failed
//! and why, then what to do about it.
//!
//! # `Try again` is not a prop
//!
//! Every caller writes the same two words, and a component that let them differ
//! would be four surfaces disagreeing about what the button is called. What each
//! caller does own is *which* read runs again: the failed one, never the page's.
//!
//! # Where it stands decides whether it carries its own air
//!
//! Every caller but one puts this inside a padded container — a `Card`, a pane
//! section, an overlay surface — so it draws no padding and starts at the left,
//! like the prose above it. The exception is a box with no padding of its own,
//! where this stands exactly where rows would: there it has to supply what the
//! container does not, and it should read like the state it is standing in for.
//! [`Blankslate`](super::Blankslate) occupies that same slot when there is
//! nothing to show, and the two saying *nothing here* and *could not find out*
//! in two different shapes is the kind of difference a reader notices and cannot
//! explain. `centred` is that case.
//!
//! # Announcing belongs to the region
//!
//! This draws; it does not speak. A region that swaps content in asynchronously
//! is the thing that knows when the swap happened, so the live region and the
//! `aria-busy` it drops are the caller's — the same split
//! [`SkeletonBox`](super::SkeletonBox) draws for the other end of the same wait.

use leptos::prelude::*;

use super::Button;

stylance::import_crate_style!(style, "src/kit/load_failure.module.scss");

#[component]
pub fn LoadFailure(
    /// What failed, in the UI's own words — `Could not load your revisions.`
    #[prop(into)]
    words: String,
    /// Why, when the spec calls for the reason to be shown — resolve mode's
    /// refused comparison. Drawn muted, beneath `words` and above the button.
    #[prop(optional, into)]
    detail: Option<String>,
    /// Runs the read that failed. Required: a failure with no way out is a dead
    /// end, and the surface that has one cannot be recovered without a reload.
    on_retry: Callback<()>,
    /// Centred, with padding of its own, for a container that has none — the
    /// installed package's file list, where this stands where the rows would.
    /// It matches `Blankslate`'s compact padding, because in that slot the two
    /// are the same kind of statement.
    #[prop(optional)]
    centred: bool,
) -> impl IntoView {
    let class = if centred {
        format!("{} {}", style::root, style::centred)
    } else {
        String::from(style::root)
    };

    view! {
        <div class=class>
            <div class=style::text>
                <p class=style::words>{words}</p>
                {detail.map(|text| view! { <p class=style::detail>{text}</p> })}
            </div>
            <Button on_click=move |_| on_retry.run(())>"Try again"</Button>
        </div>
    }
}
