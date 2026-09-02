//! Indeterminate progress: something is happening and we cannot say how far along.
//!
//! # Not the page's loading state
//!
//! It used to be. It is not any more — [`SkeletonBox`](super::SkeletonBox) is, because a
//! skeleton holds the space the content will occupy and a spinner tells you nothing
//! except that you are waiting. A spinner is for work whose *shape* is unknown, not for
//! content whose shape we can predict.
//!
//! Which leaves it two jobs: inline beside a label that names the work, and filling a
//! region that cannot be skeletonised because its contents are not a list of rows.
//!
//! # `Button` has its own
//!
//! Deliberately not this one. `Button` draws its spinner as a `::before` on the leading
//! slot so a button with an icon does not change width when work starts — replacing that
//! with an element would put a layout property at the mercy of a child. The ring CSS is
//! duplicated in the two modules, ten lines, and that is cheaper than coupling a
//! button's width to another component.

use leptos::prelude::*;

stylance::import_crate_style!(style, "src/kit/spinner.module.scss");

/// Inline beside text, or centred in a region of its own.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SpinnerVariant {
    /// 14px, sits on the text baseline next to a label that says what is happening.
    #[default]
    Inline,
    /// 24px, centred with room around it, for a region whose content is not rows.
    Region,
}

#[component]
pub fn Spinner(
    #[prop(optional)] variant: SpinnerVariant,
    /// What is happening, for screen readers — `Signing in`, `Checking for revisions`.
    ///
    /// `None` makes it **decorative**, which is correct when visible text beside it
    /// already says the same thing: announcing "Publishing… busy Publishing" is worse
    /// than announcing it once. So an `Inline` spinner usually passes nothing and a
    /// `Region` spinner, which has no text beside it, always passes something.
    ///
    /// Rendered as off-screen **text inside** the live region, not as `aria-label` on it.
    /// A live region announces its *content*; a label names it. With `role="status"` and
    /// nothing inside, the "work has started" announcement — the entire point — may never
    /// fire. Primer reached the same conclusion and lists `aria-label` as deprecated on its
    /// `Spinner` in favour of `srText`.
    #[prop(optional, into)]
    aria_label: Option<String>,
) -> impl IntoView {
    let class = match variant {
        SpinnerVariant::Inline => style::inline.to_string(),
        SpinnerVariant::Region => format!("{} {}", style::inline, style::region),
    };

    view! {
        {match aria_label {
            // `status`, not `alert`: work starting is not an interruption. It is polite,
            // so it waits for a pause rather than cutting across what is being read.
            Some(label) => {
                view! {
                    <span class=class role="status">
                        <span data-sr-only>{label}</span>
                    </span>
                }
                    .into_any()
            }
            None => view! { <span class=class aria-hidden="true"></span> }.into_any(),
        }}
    }
}
