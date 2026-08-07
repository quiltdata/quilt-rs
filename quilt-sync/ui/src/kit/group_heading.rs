//! Names and counts a run of rows.

use leptos::prelude::*;

stylance::import_crate_style!(style, "src/kit/group_heading.module.scss");

#[component]
pub fn GroupHeading(
    #[prop(into)] title: String,
    /// How many rows follow. Always shown, including one — "1" is information, and
    /// hiding it would make a single-row group look like a header with a bug.
    count: usize,
    /// A shared cause affecting the whole group, such as
    /// `no access as analyst`. Only the bucket axis has one: a prefix spans
    /// buckets, so no cause is a property of the group.
    #[prop(optional, into)]
    annotation: Option<String>,
) -> impl IntoView {
    view! {
        <div class=style::root>
            <span class=style::title>{title}</span>
            {annotation
                .map(|note| view! { <span class=style::annotation>{format!("— {note}")}</span> })}
            <span class=style::count>{count}</span>
        </div>
    }
}
