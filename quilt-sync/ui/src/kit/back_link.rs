//! A link to the page above this one.
//!
//! # It is an *up* link, not Back
//!
//! It always goes to the parent route, whatever the history holds — someone can
//! arrive at a package from the queue, from a deep link, or from `/commit`, and
//! all three leave by the same door. That is exactly why it names its
//! destination instead of saying "Back": browser Back and this control do
//! different things, and only one of them can promise where it lands.
//!
//! # The rule for the label
//!
//! **It follows the destination's own name.** If the page above ever renames
//! itself, this changes with it; the two must not be allowed to drift, because
//! the reader only finds out they disagree after the navigation.
//!
//! # Why not breadcrumbs
//!
//! A trail of two is this control wearing a costume — the second crumb restates
//! the page heading an inch below itself — and it costs a band of height the
//! installed-package page does not have. Breadcrumbs earn themselves at three.

use leptos::prelude::*;

use super::icons;

stylance::import_crate_style!(style, "src/kit/back_link.module.scss");

#[component]
pub fn BackLink(
    /// Where the page above lives.
    #[prop(into)]
    href: String,
    /// The destination's own name — `Packages`, not `Back`.
    #[prop(into)]
    label: String,
    /// Replace the current history entry instead of pushing one, for leaving a
    /// mode that Back should not return to. The router reads `a.replace`.
    #[prop(optional)]
    replace: bool,
) -> impl IntoView {
    view! {
        <a class=style::root href=href prop:replace=replace>
            <span class=style::chevron>{icons::chevron_left()}</span>
            {label}
        </a>
    }
}
