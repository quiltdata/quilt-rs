//! A page's regions in rows of two on a wide page, one column on a narrow one.
//!
//! # Rows, and one order
//!
//! The regions fill the rows left to right, top to bottom, and narrow they
//! are the same sequence in one column. So what is drawn, what a screen
//! reader reads and what Tab reaches are one order at every width, and a page
//! chooses its layout by the order of its children alone. The two regions in
//! a row share its height, so each row has one bottom edge, as the main
//! page's state strip does.
//!
//! # Where it switches
//!
//! A container query on the layout's own width rather than a viewport media
//! query, as on the package page: a gallery cell is not the window, and the
//! layout should answer for the room it has. See `two_columns.module.scss`
//! for the number.

use leptos::prelude::*;

stylance::import_crate_style!(style, "src/kit/two_columns.module.scss");

#[component]
pub fn TwoColumns(
    /// The regions, in reading order: wide, two to a row.
    children: Children,
) -> impl IntoView {
    view! {
        <div class=style::frame>
            <div class=style::grid>{children()}</div>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{load_module_style, mount, sleep_ms};
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::*;

    fn load_stylesheet() {
        load_module_style(
            include_str!("two_columns.module.scss"),
            &[("frame", style::frame), ("grid", style::grid)],
        );
    }

    /// A card that asks for `height` px, named by its text.
    fn card(name: &'static str, height: u32) -> impl IntoView {
        view! {
            <section style=format!("min-height:{height}px")>
                <p style="margin:0">{name}</p>
            </section>
        }
    }

    /// Four cards, `a` tall, the layout `width` px wide.
    async fn layout(width: u32) -> web_sys::Element {
        load_stylesheet();
        let el = mount(move || {
            view! {
                <div style=format!("width:{width}px")>
                    <TwoColumns>
                        {card("a", 300)}
                        {card("b", 50)}
                        {card("c", 50)}
                        {card("d", 50)}
                    </TwoColumns>
                </div>
            }
        });
        sleep_ms(0).await;
        el
    }

    /// Each card's left, top and height, in DOM order.
    fn boxes(el: &web_sys::Element) -> Vec<(f64, f64, f64)> {
        let found = el.query_selector_all("section").unwrap();
        (0..found.length())
            .map(|i| {
                let s: web_sys::Element = found.get(i).unwrap().unchecked_into();
                let r = s.get_bounding_client_rect();
                (r.left(), r.top(), r.height())
            })
            .collect()
    }

    /// Under 976px: one column, top to bottom in the DOM's order.
    #[wasm_bindgen_test]
    #[allow(clippy::float_cmp, reason = "box edges here are whole pixels")]
    async fn narrow_is_one_column_in_reading_order() {
        let drawn = boxes(&layout(960).await);
        assert!(
            drawn.iter().all(|b| b.0 == drawn[0].0),
            "one column: {drawn:?}"
        );
        assert!(
            drawn.windows(2).all(|w| w[0].1 < w[1].1),
            "top to bottom: {drawn:?}"
        );
    }

    /// From 976px: two to a row, left to right, and a row's two cards share
    /// its height, however tall one of them asks to be.
    #[wasm_bindgen_test]
    #[allow(clippy::float_cmp, reason = "box edges here are whole pixels")]
    async fn wide_is_rows_of_two_that_share_a_height() {
        let drawn = boxes(&layout(976).await);
        let [a, b, c, d] = [drawn[0], drawn[1], drawn[2], drawn[3]];
        assert!(b.0 > a.0, "b right of a: {drawn:?}");
        assert_eq!((b.1, b.2), (a.1, a.2), "b shares a's row and height");
        assert_eq!((c.0, d.0), (a.0, b.0), "c and d under a and b");
        assert!(c.1 >= a.1 + 300.0, "the second row starts under the tall a");
        assert_eq!(c.1, d.1, "c and d share a row");
    }
}
