//! `TwoColumns` stories.

use leptos::prelude::*;

use crate::Cell;
use crate::Story;
use crate::kit::Card;
use crate::kit::ColumnItem;
use crate::kit::TwoColumns;

/// A card of `lines` lines, so the two stacks differ in height.
fn card(title: &'static str, lines: usize) -> AnyView {
    view! {
        <Card title=title>
            {(1..=lines).map(|n| view! { <div>{format!("Line {n}")}</div> }).collect_view()}
        </Card>
    }
    .into_any()
}

/// Five cards, numbered by their narrow place: wide, 1, 2, 5 on the left and
/// 3, 4 on the right.
fn layout(width: u32) -> AnyView {
    view! {
        <div style=format!("width:{width}px; max-width:100%")>
            <TwoColumns
                left=vec![
                    ColumnItem::new(1, card("1 · left, tall", 6)),
                    ColumnItem::new(2, card("2 · left", 2)),
                    ColumnItem::new(5, card("5 · left", 1)),
                ]
                right=vec![
                    ColumnItem::new(3, card("3 · right", 1)),
                    ColumnItem::new(4, card("4 · right", 2)),
                ]
            />
        </div>
    }
    .into_any()
}

#[component]
pub fn TwoColumnsStories() -> impl IntoView {
    view! {
        <Story
            title="TwoColumns"
            note="A page's regions in two columns from a 1024px window (976px of its own width), \
                  and in one column below that. The columns stack on their own: a tall card on \
                  the left leaves no hole on the right. Narrow, each region goes where the page \
                  numbered it, which need not be left then right — here 3 and 4 come from the \
                  right between the left's 2 and 5. Tab order is the DOM's, left then right."
        >
            <Cell full=true label="976px — two columns">
                {layout(976)}
            </Cell>
            <Cell full=true label="960px — one column, in the page's order">
                {layout(960)}
            </Cell>
        </Story>
    }
}
