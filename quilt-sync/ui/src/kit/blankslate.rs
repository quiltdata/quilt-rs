//! Nothing to show, and what to do about it.

use leptos::prelude::*;

stylance::import_crate_style!(style, "src/kit/blankslate.module.scss");

#[component]
pub fn Blankslate(
    #[prop(into)] heading: String,
    #[prop(into)] description: String,
    /// Offered only when there is something to do. "No results" has no action —
    /// the user already knows how to change their search, and a button there would
    /// be filler.
    #[prop(optional)]
    primary_action: Option<AnyView>,
) -> impl IntoView {
    view! {
        <div class=style::root>
            // `h3`: a blankslate stands where a card's rows would, under its `h2`.
            <h3 class=style::heading>{heading}</h3>
            <span class=style::description>{description}</span>
            {primary_action}
        </div>
    }
}
