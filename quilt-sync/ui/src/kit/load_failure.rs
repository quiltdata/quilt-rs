//! A read that did not answer, and the one way to ask again.
//!
//! # Not a `Blankslate`
//!
//! A [`Blankslate`](super::Blankslate) says there is nothing to show. This says
//! we could not find out. `No files yet` over a read that never returned
//! manufactures a state the page does not know, which is worse than a page that
//! says less.
//!
//! # The sentence is the UI's, never the backend's
//!
//! `words` is fixed copy the caller owns. A `Result<_, String>` error text is not
//! a word from the vocabulary: it cannot be reviewed, it cannot be changed
//! without a backend release, and no mechanical test can see it. The backend's
//! text is for the log.
//!
//! # `Try again` is not a prop
//!
//! Every caller writes the same two words, and a component that let them differ
//! would be four surfaces disagreeing about what the button is called. What each
//! caller does own is *which* read runs again: the failed one, never the page's.
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
    /// Runs the read that failed. Required: a failure with no way out is a dead
    /// end, and the surface that has one cannot be recovered without a reload.
    on_retry: Callback<()>,
) -> impl IntoView {
    view! {
        <div class=style::root>
            <p class=style::words>{words}</p>
            <Button on_click=move |_| on_retry.run(())>"Try again"</Button>
        </div>
    }
}
