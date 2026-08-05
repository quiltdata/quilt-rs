//! Nothing to show, and what to do about it.

use leptos::prelude::*;

stylance::import_crate_style!(style, "src/kit/empty_state.module.scss");

#[component]
pub fn EmptyState(
    #[prop(into)] title: String,
    #[prop(into)] body: String,
    /// Offered only when there is something to do. "No results" has no action —
    /// the user already knows how to change their search, and a button there would
    /// be filler.
    #[prop(optional)]
    action: Option<AnyView>,
) -> impl IntoView {
    view! {
        <div class=style::root>
            <span class=style::title>{title}</span>
            <span class=style::body>{body}</span>
            {action}
        </div>
    }
}
