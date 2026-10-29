//! A page's regions in rows of two, or in one column when the page is narrow.
//!
//! The regions stay in the DOM in reading order and nothing moves them, so
//! what is drawn, what a screen reader reads and what Tab reaches are one
//! order at every width. The two regions in a row share its height.
//!
//! The page passes the narrowest a column may be. The switch depends on the
//! layout's own width, not the window's, so a gallery cell behaves like a
//! window of its width.
//!
//! With `fill_height` the layout fills its parent's flex column, so a region
//! can scroll inside its own box.

use leptos::prelude::*;

stylance::import_crate_style!(style, "src/kit/two_columns.module.scss");

#[component]
pub fn TwoColumns(
    /// The narrowest a column may be, in px. When two of them and the gap
    /// don't fit, the regions are one column.
    min_column: u32,
    /// The smaller gap, as on the main page's state strip.
    #[prop(optional)]
    tight: bool,
    /// Fill the height the parent's flex column leaves.
    #[prop(optional)]
    fill_height: bool,
    /// The regions, in reading order: wide, two to a row.
    children: Children,
) -> impl IntoView {
    let class = [
        Some(style::grid),
        tight.then_some(style::tight),
        fill_height.then_some(style::fill),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" ");
    view! {
        <div class=class style=format!("--two-columns-min:{min_column}px")>
            {children()}
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
            &[
                ("grid", style::grid),
                ("tight", style::tight),
                ("fill", style::fill),
            ],
        );
    }

    /// The kit's spacing tokens, which the test page does not load.
    const TOKENS: &str = "--q-space-3:12px; --q-space-4:16px";

    /// A card that asks for `height` px, named by its text.
    fn card(name: &'static str, height: u32) -> impl IntoView {
        view! {
            <section style=format!("min-height:{height}px")>
                <p style="margin:0">{name}</p>
            </section>
        }
    }

    /// Four cards, `a` tall, the layout `width` px wide.
    async fn layout(width: u32, min_column: u32, tight: bool) -> web_sys::Element {
        load_stylesheet();
        let el = mount(move || {
            view! {
                <div style=format!("{TOKENS}; width:{width}px")>
                    <TwoColumns min_column=min_column tight=tight>
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

    /// Each card's left, top, width and height, in DOM order.
    fn boxes(el: &web_sys::Element) -> Vec<(f64, f64, f64, f64)> {
        let found = el.query_selector_all("section").unwrap();
        (0..found.length())
            .map(|i| {
                let s: web_sys::Element = found.get(i).unwrap().unchecked_into();
                let r = s.get_bounding_client_rect();
                (r.left(), r.top(), r.width(), r.height())
            })
            .collect()
    }

    #[allow(clippy::float_cmp, reason = "box edges here are whole pixels")]
    fn is_one_column(drawn: &[(f64, f64, f64, f64)]) -> bool {
        drawn.iter().all(|b| b.0 == drawn[0].0) && drawn.windows(2).all(|w| w[0].1 < w[1].1)
    }

    /// Two columns when two minimums and the gap fit, one column a pixel
    /// narrower.
    #[wasm_bindgen_test]
    #[allow(clippy::float_cmp, reason = "box edges here are whole pixels")]
    async fn switches_where_two_minimums_and_the_gap_fit() {
        let drawn = boxes(&layout(975, 480, false).await);
        assert!(is_one_column(&drawn), "975: one column: {drawn:?}");

        let drawn = boxes(&layout(976, 480, false).await);
        let [a, b] = [drawn[0], drawn[1]];
        assert_eq!((a.2, b.2), (480.0, 480.0), "976: two halves: {drawn:?}");
        assert_eq!(b.0 - a.0, 496.0, "976: a 16px gap: {drawn:?}");
    }

    /// The tight gap counts in the switch too.
    #[wasm_bindgen_test]
    #[allow(clippy::float_cmp, reason = "box edges here are whole pixels")]
    async fn tight_gap_moves_the_switch_with_it() {
        let drawn = boxes(&layout(611, 300, true).await);
        assert!(is_one_column(&drawn), "611: one column: {drawn:?}");

        let drawn = boxes(&layout(612, 300, true).await);
        let [a, b] = [drawn[0], drawn[1]];
        assert_eq!((a.2, b.2), (300.0, 300.0), "612: two of 300: {drawn:?}");
        assert_eq!(b.0 - a.0, 312.0, "612: a 12px gap: {drawn:?}");
    }

    /// However wide, never three columns.
    #[wasm_bindgen_test]
    #[allow(clippy::float_cmp, reason = "box edges here are whole pixels")]
    async fn never_more_than_two_columns() {
        let drawn = boxes(&layout(1600, 300, false).await);
        let [a, b, c, d] = [drawn[0], drawn[1], drawn[2], drawn[3]];
        assert_eq!((a.2, b.2), (792.0, 792.0), "two halves: {drawn:?}");
        assert_eq!((c.0, d.0), (a.0, b.0), "c and d under a and b: {drawn:?}");
    }

    /// Narrower than one minimum, the one column still fits the layout.
    #[wasm_bindgen_test]
    #[allow(clippy::float_cmp, reason = "box edges here are whole pixels")]
    async fn narrower_than_the_minimum_still_fits() {
        let drawn = boxes(&layout(200, 300, false).await);
        assert!(is_one_column(&drawn), "one column: {drawn:?}");
        assert!(drawn.iter().all(|b| b.2 == 200.0), "200 wide: {drawn:?}");
    }

    /// Wide, a row's two cards share its height, however tall one of them
    /// asks to be.
    #[wasm_bindgen_test]
    #[allow(clippy::float_cmp, reason = "box edges here are whole pixels")]
    async fn a_row_shares_one_height() {
        let drawn = boxes(&layout(976, 480, false).await);
        let [a, b, c, d] = [drawn[0], drawn[1], drawn[2], drawn[3]];
        assert!(b.0 > a.0, "b right of a: {drawn:?}");
        assert_eq!((b.1, b.3), (a.1, a.3), "b shares a's row and height");
        assert_eq!((c.0, d.0), (a.0, b.0), "c and d under a and b");
        assert!(c.1 >= a.1 + 300.0, "the second row starts under the tall a");
        assert_eq!(c.1, d.1, "c and d share a row");
    }

    /// A form and a long list in a flex column of fixed height. The list
    /// region scrolls its rows in a box of its own.
    async fn filled(width: u32) -> web_sys::Element {
        load_stylesheet();
        let el = mount(move || {
            view! {
                <div style=format!(
                    "{TOKENS}; width:{width}px; height:400px; display:flex; flex-direction:column",
                )>
                    <TwoColumns min_column=300 fill_height=true>
                        <section>
                            <p style="margin:0; height:100px">"form"</p>
                        </section>
                        <section style="display:flex; flex-direction:column; min-height:80px; overflow:hidden">
                            <ul style="flex:1 1 0; min-height:0; overflow:auto; margin:0">
                                <li style="height:1000px">"rows"</li>
                            </ul>
                        </section>
                    </TwoColumns>
                </div>
            }
        });
        sleep_ms(0).await;
        el
    }

    /// Wide, the one row takes the parent's height, and the long list
    /// scrolls inside it instead of growing the row.
    #[wasm_bindgen_test]
    #[allow(clippy::float_cmp, reason = "box edges here are whole pixels")]
    async fn fill_height_wide_is_one_row_the_height_of_the_parent() {
        let el = filled(800).await;
        let drawn = boxes(&el);
        let [form, list] = [drawn[0], drawn[1]];
        assert_eq!(form.1, list.1, "side by side: {drawn:?}");
        assert_eq!((form.3, list.3), (400.0, 400.0), "both fill: {drawn:?}");
        let ul = el.query_selector("ul").unwrap().unwrap();
        assert!(ul.scroll_height() > ul.client_height(), "the rows scroll");
    }

    /// Narrow, the form keeps its height and the list takes the rest.
    #[wasm_bindgen_test]
    #[allow(clippy::float_cmp, reason = "box edges here are whole pixels")]
    async fn fill_height_narrow_gives_the_rest_to_the_last_region() {
        let el = filled(500).await;
        let drawn = boxes(&el);
        let [form, list] = [drawn[0], drawn[1]];
        assert_eq!(form.3, 100.0, "the form keeps its height: {drawn:?}");
        // Relative, and to the pixel: the views earlier tests left mounted
        // can put the form at a fractional top.
        assert_eq!(
            (list.1 - form.1).round(),
            100.0 + 16.0,
            "the list under it: {drawn:?}"
        );
        assert_eq!(
            list.3,
            400.0 - 100.0 - 16.0,
            "and takes the rest: {drawn:?}"
        );
    }
}
