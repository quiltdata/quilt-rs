//! `TwoColumns` stories.

use leptos::prelude::*;

use crate::Cell;
use crate::Story;
use crate::kit::Card;
use crate::kit::TwoColumns;

/// A card of `lines` lines, so a row's two cards differ in what they hold.
fn card(title: &'static str, lines: usize) -> impl IntoView {
    view! {
        <Card title=title>
            {(1..=lines).map(|n| view! { <div>{format!("Line {n}")}</div> }).collect_view()}
        </Card>
    }
}

/// Five cards, numbered in reading order.
fn layout(width: u32) -> AnyView {
    view! {
        <div style=format!("width:{width}px; max-width:100%")>
            <TwoColumns min_column=480>
                {card("1 · tall", 6)}
                {card("2", 2)}
                {card("3", 1)}
                {card("4", 3)}
                {card("5 · alone in its row", 1)}
            </TwoColumns>
        </div>
    }
    .into_any()
}

#[component]
pub fn TwoColumnsStories() -> impl IntoView {
    view! {
        <Story
            title="TwoColumns"
            note="A page's regions two to a row while each column gets the page's minimum, \
                  and one column below that, in the same order. These cells use Settings' \
                  minimum: the narrowest width with two columns, then a pixel narrower. A \
                  row's two cards share its height."
        >
            <Cell full=true label="976px — rows of two">
                {layout(976)}
            </Cell>
            <Cell full=true label="975px — one column, in the same order">
                {layout(975)}
            </Cell>
        </Story>
    }
}
