//! The name of a page region, with an optional count.
//!
//! Atomic on purpose. It is four declarations and a span, and making it a component
//! rather than a class is what stops the next region inventing its own spelling of
//! the same thing.

use leptos::prelude::*;

stylance::import_crate_style!(style, "src/kit/section_label.module.scss");

#[component]
pub fn SectionLabel(
    /// In sentence case — `Needs your attention`. The upper-casing is done in CSS,
    /// not here, and that is not a style preference: `text-transform` leaves the
    /// text itself intact, while typing `NEEDS YOUR ATTENTION` makes some screen
    /// readers spell it letter by letter as an initialism.
    #[prop(into)]
    text: String,
    /// How many items the region holds. `None` for a region whose size is not
    /// interesting — the healthy queue says `Everything is Latest` and counting to
    /// zero would be noise.
    ///
    /// It must be **derived** from the rows rendered, never written. The design
    /// mock's queue is labelled `(17)` above 11 + 3 + 5 = 19 rows, which is what a
    /// hand-written count does the moment the rows change.
    #[prop(optional)]
    count: Option<usize>,
) -> impl IntoView {
    view! {
        // `h2`, so the region appears in the heading outline a screen reader
        // navigates by. The page owns the single `h1`.
        <h2 class=style::root>
            {text}
            {count.map(|count| view! { <span class=style::count>{format!("({count})")}</span> })}
        </h2>
    }
}
