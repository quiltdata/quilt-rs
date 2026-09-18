//! One revision: what it says, when, and whether anyone else can see it.
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
//! when they could not tell two revisions apart. A `href` may carry one, because
//! an address is not a name.
//!
//! # Published is a glyph, not a word
//!
//! A list of revisions is a column where most rows say the same thing, and a
//! word repeated four times down a 280px pane is four times the width for one
//! bit. So the fact is a cloud, and the row that has not left this machine wears
//! the slashed one — a pair, rather than a glyph and its absence, because
//! nothing distinguishes an absent glyph from a component that was not told.
//! The word itself is still there for anyone not reading pixels.
//!
//! # Two facts, two props
//!
//! `published` says whether the revision reached the platform; `href` says where
//! to read it. They are separate because a published revision in a bucket with
//! no catalog host has nowhere to point — it earns the cloud and stays text.
//! The reverse never holds: a revision this copy has not sent has no address.

use leptos::prelude::*;

use super::RelativeTime;
use super::countdown::EpochMillis;
use super::icons;

stylance::import_crate_style!(style, "src/kit/revision_row.module.scss");

#[component]
pub fn RevisionRow(
    /// The commit message. Quoted by the component rather than by the caller, so
    /// four call sites cannot quote it four ways.
    #[prop(into)]
    message: String,
    /// When this copy obtained it.
    at: EpochMillis,
    /// Whether it reached the platform. `None` draws no glyph at all, for the
    /// callers that are not showing a list — the current revision under the
    /// pane's own heading, and the two sides of a resolve, where the header and
    /// the section labels have already said it.
    #[prop(optional)]
    published: Option<bool>,
    /// Where to read it. The message becomes a link; the whole row does not,
    /// because the time beside it is this copy's fact and not the platform's.
    ///
    /// **Inside the app the caller intercepts the click** and hands the address
    /// to the browser — following it in place would navigate the webview and
    /// take the app with it.
    ///
    /// `optional_no_strip` rather than `optional`: callers hold an `Option`
    /// already, because `util::catalog_url` answers `None` for a bucket with no
    /// catalog host.
    #[prop(optional_no_strip)]
    href: Option<String>,
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
    // The whole value rides in `title`, since the visible one is ellipsised. An
    // empty message loses the tooltip with it: there is nothing to put in one.
    let full = message;
    let (text, class, title) = match quoted {
        Some(text) => (text, String::from(style::message), Some(full)),
        // A revision published without a message is still a revision that was
        // published, so the placeholder is a link like any other row. Dropping
        // the link with the words would be the match arm deciding something
        // nobody meant.
        None => (String::from("No message"), String::from(style::empty), None),
    };
    let body = match href {
        Some(href) => {
            let class = format!("{} {class}", style::link);
            view! { <a class=class href=href title=title>{text}</a> }.into_any()
        }
        None => view! { <span class=class title=title>{text}</span> }.into_any(),
    };

    let glyph = published.map(|published| {
        let (icon, words) = if published {
            (icons::cloud(), "Published")
        } else {
            (icons::cloud_offline(), "Not published")
        };
        view! {
            // `title` for the pointer, the word itself for everyone else: a glyph
            // carrying a fact nothing else on the row states cannot be the only
            // place that fact exists.
            <span class=style::glyph title=words>
                {icon}
                <span data-sr-only>{words}</span>
            </span>
        }
    });

    view! {
        <div class=style::root>
            {glyph}
            <div class=style::body>
                {body}
                <span class=style::when>
                    <RelativeTime at=at />
                </span>
            </div>
        </div>
    }
}
