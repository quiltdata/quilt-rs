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

    /// Narrow is the default and draws by `order`; the container query makes
    /// two stacks that each keep their own height.
    #[test]
    fn the_stylesheet_switches_on_the_layouts_own_width() {
        assert!(STYLE.contains("container-type: inline-size"));
        assert!(STYLE.contains("@container (min-width: 976px)"));
        assert!(
            STYLE.contains("display: contents"),
            "narrow, the stacks add no box"
        );
        assert!(STYLE.contains("order: var(--q-narrow-order)"));
        assert!(
            STYLE.contains("align-items: flex-start"),
            "wide, the stacks keep their own heights"
        );
    }
}
