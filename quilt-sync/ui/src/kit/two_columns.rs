//! Two columns of regions on a wide page, one column on a narrow one.
//!
//! # Why two stacks and not a grid
//!
//! The columns stack **independently**: a tall region on the left leaves no
//! hole under a short one on the right. A grid shares row heights between its
//! columns, so it cannot do that without JavaScript measuring every card.
//!
//! # Why the narrow order is the page's to choose
//!
//! Narrow, the page is one column, and that column is not "left, then right".
//! Settings, for one, puts Accounts third in one column but at the top of the
//! right one when wide. So each region says where it goes in the one column
//! ([`ColumnItem::new`]), and narrow, the two stacks get `display: contents`
//! and their regions are drawn by `order`.
//!
//! The DOM is the wide order, so narrow, focus and reading order are the left
//! column, then the right one, wherever `order` draws them. The regions are
//! independent cards, so nothing reads wrong; only the tab order may jump.
//!
//! # Where it switches
//!
//! A container query on the layout's own width rather than a viewport media
//! query, as on the package page: a gallery cell is not the window, and the
//! layout should answer for the room it has. See `two_columns.module.scss` for
//! the number.

use leptos::prelude::*;

stylance::import_crate_style!(style, "src/kit/two_columns.module.scss");

/// One region, and its place in the narrow layout's one column.
pub struct ColumnItem {
    narrow: u8,
    view: AnyView,
}

impl ColumnItem {
    /// `narrow` is the region's position in the one column, from 1.
    pub fn new(narrow: u8, view: impl IntoView + 'static) -> Self {
        Self {
            narrow,
            view: view.into_any(),
        }
    }
}

fn stack(items: Vec<ColumnItem>) -> impl IntoView {
    view! {
        <div class=style::column>
            {items
                .into_iter()
                .map(|item| {
                    view! {
                        <div class=style::item style=format!("--q-narrow-order: {}", item.narrow)>
                            {item.view}
                        </div>
                    }
                })
                .collect_view()}
        </div>
    }
}

#[component]
pub fn TwoColumns(
    /// The left column, top to bottom.
    left: Vec<ColumnItem>,
    /// The right column, top to bottom.
    right: Vec<ColumnItem>,
) -> impl IntoView {
    view! {
        <div class=style::frame>
            <div class=style::columns>{stack(left)} {stack(right)}</div>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::mount;
    use wasm_bindgen_test::*;

    const STYLE: &str = include_str!("two_columns.module.scss");

    /// The DOM is the wide order, and each region carries its narrow place.
    #[wasm_bindgen_test]
    fn each_region_carries_its_place_in_the_one_column() {
        let el = mount(|| {
            view! {
                <TwoColumns
                    left=vec![ColumnItem::new(1, view! { <p>"a"</p> }), ColumnItem::new(3, view! { <p>"c"</p> })]
                    right=vec![ColumnItem::new(2, view! { <p>"b"</p> })]
                />
            }
        });
        let items = el.query_selector_all("p").unwrap();
        let drawn: Vec<(String, String)> = (0..items.length())
            .map(|i| {
                let p: web_sys::Element =
                    wasm_bindgen::JsCast::unchecked_into(items.get(i).unwrap());
                let place = p.parent_element().unwrap().get_attribute("style").unwrap();
                (p.text_content().unwrap(), place)
            })
            .collect();
        assert_eq!(
            drawn,
            [
                ("a".into(), "--q-narrow-order: 1".into()),
                ("c".into(), "--q-narrow-order: 3".into()),
                ("b".into(), "--q-narrow-order: 2".into()),
            ]
        );
    }

    /// The stylesheet, with its classes renamed to the hashed ones the
    /// component draws. The test page loads no stylesheet of its own, and this
    /// one is plain CSS, so it can be loaded as it is.
    fn load_stylesheet() {
        let css = STYLE
            .replace(".columns", "\u{0}")
            .replace(".frame", &format!(".{}", style::frame))
            .replace(".column", &format!(".{}", style::column))
            .replace(".item", &format!(".{}", style::item))
            .replace('\u{0}', &format!(".{}", style::columns));
        let doc = web_sys::window().unwrap().document().unwrap();
        let tag = doc.create_element("style").unwrap();
        tag.set_text_content(Some(&css));
        doc.head().unwrap().append_child(&tag).unwrap();
    }

    /// A card `height` px tall, named by its text.
    fn card(name: &'static str, height: u32) -> AnyView {
        view! { <p style=format!("margin:0; height:{height}px")>{name}</p> }.into_any()
    }

    /// Each card's (left, top), by name, with the layout `width` px wide.
    fn drawn(width: u32) -> Vec<(String, f64, f64)> {
        load_stylesheet();
        let el = mount(move || {
            view! {
                <div style=format!("width:{width}px")>
                    <TwoColumns
                        left=vec![ColumnItem::new(1, card("a", 300)), ColumnItem::new(3, card("c", 50))]
                        right=vec![ColumnItem::new(2, card("b", 50)), ColumnItem::new(4, card("d", 50))]
                    />
                </div>
            }
        });
        let cards = el.query_selector_all("p").unwrap();
        let mut out: Vec<_> = (0..cards.length())
            .map(|i| {
                let p: web_sys::Element =
                    wasm_bindgen::JsCast::unchecked_into(cards.get(i).unwrap());
                let rect = p.get_bounding_client_rect();
                (p.text_content().unwrap(), rect.left(), rect.top())
            })
            .collect();
        out.sort_by(|x, y| x.0.cmp(&y.0));
        out
    }

    /// Under 976px: one column, in the page's order rather than the DOM's.
    #[wasm_bindgen_test]
    #[allow(clippy::float_cmp, reason = "box edges here are whole pixels")]
    fn narrow_draws_one_column_in_the_pages_order() {
        let cards = drawn(960);
        assert!(
            cards.iter().all(|c| c.1 == cards[0].1),
            "one column: {cards:?}"
        );
        let mut by_top = cards.clone();
        by_top.sort_by(|x, y| x.2.total_cmp(&y.2));
        let order: Vec<_> = by_top.into_iter().map(|c| c.0).collect();
        assert_eq!(order, ["a", "b", "c", "d"]);
    }

    /// From 976px: two columns, each stacking on its own, so the right one
    /// does not wait for the tall card on the left.
    #[wasm_bindgen_test]
    fn wide_draws_two_columns_that_stack_on_their_own() {
        let cards = drawn(976);
        let [a, b, c, d] = [&cards[0], &cards[1], &cards[2], &cards[3]];
        assert!(b.1 > a.1, "b sits right of a: {cards:?}");
        assert_eq!(
            (b.2, c.1, d.1),
            (a.2, a.1, b.1),
            "b level with a; c under a; d under b"
        );
        assert!(c.2 >= a.2 + 300.0, "c under the tall a");
        assert!(d.2 < a.2 + 300.0, "d does not wait for a: {cards:?}");
    }
}
