//! Bordered surface for a titled list of related rows.
//!
//! The page's four regions are all one of these: the two state-strip blocks, the
//! attention queue, and the list. That is what makes the page read as a page rather
//! than as two widgets followed by loose text — and it is not only cosmetic, since the
//! rows were designed against a card's `--q-bgColor-default`, where their hairlines and
//! hover tint were measured.

use leptos::prelude::*;

stylance::import_crate_style!(style, "src/kit/card.module.scss");

#[component]
pub fn Card(
    /// Optional, because the list card has none: its `SegmentedControl` names the view, and a
    /// card titled `Packages` above a Packages / Recent files switch says it twice.
    #[prop(optional, into)]
    title: Option<String>,
    /// How many rows the card holds, beside the title. Must be **derived** from the
    /// rows rendered, never written — the design mock labels its queue `(17)` above
    /// 11 + 3 + 5 = 19 rows, which is what a hand-written count does the moment the
    /// rows change.
    #[prop(optional)]
    count: Option<usize>,
    /// Rows. The card draws a hairline between any two of them, so children need not
    /// know they are in a list — pass a single wrapper element to opt out, as the queue
    /// does, where dividers would make a list of decisions read as a table.
    children: Children,
) -> impl IntoView {
    view! {
        // `h2`: the page's regions are h2, and a card is a region. If a caller ever
        // needs a different level, that is a prop — not a hard-coded guess repeated at
        // each call site.
        <section class=style::root>
            {title
                .map(|title| {
                    view! {
                        <h2 class=style::title>
                            {title}
                            {count
                                .map(|count| {
                                    view! { <span class=style::count>{format!("({count})")}</span> }
                                })}
                        </h2>
                    }
                })}
            <div class=style::body>{children()}</div>
        </section>
    }
}
