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
            <span class=style::heading>{heading}</span>
            <span class=style::description>{description}</span>
            {primary_action}
        </div>
    }
}
