//! Titled container for a list of related rows.

use leptos::prelude::*;

stylance::import_crate_style!(style, "src/kit/card.module.scss");

#[component]
pub fn Card(
    #[prop(into)] title: String,
    /// Rows. The card draws a hairline between any two of them, so children need
    /// not know they are in a list.
    children: Children,
) -> impl IntoView {
    view! {
        // `h2`: the page's regions are h2, and a card is a region. If a caller
        // ever needs a different level, that is a prop — not a hard-coded guess
        // repeated at each call site.
        <section class=style::root>
            <h2 class=style::title>{title}</h2>
            <div class=style::body>{children()}</div>
        </section>
    }
}
