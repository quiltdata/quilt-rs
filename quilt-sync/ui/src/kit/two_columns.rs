//! Two columns of regions on a wide page, one column on a narrow one.
//!
//! # Why two stacks and not a grid
//!
//! The columns stack **independently**: a tall region on the left leaves no
//! hole under a short one on the right. A grid shares row heights between its
//! columns, so it cannot do that.
//!
//! # Why the narrow order is the page's to choose
//!
//! Narrow, the page is one column, and that column is not "left, then right".
//! Settings, for one, puts Accounts third in one column but at the top of the
//! right one when wide. So each region says where it goes in the one column
//! ([`ColumnItem::new`]).
//!
//! # Why the regions move in the DOM rather than by `order`
//!
//! CSS `order` would draw the one column in the page's order and leave focus
//! and a screen reader in the DOM's, so Tab would go down past Accounts and
//! come back up for it. Instead the layout moves the regions themselves: wide,
//! each sits in its own column; narrow, all of them sit in the first column in
//! the page's order. What is drawn, what is read and what Tab reaches are then
//! one order at every width.
//!
//! Moving an element out of the document drops its focus, so a region holding
//! focus gets it back after the move. The regions are moved whole, so what is
//! inside them, and the reactivity drawing it, is untouched.
//!
//! # Where it switches
//!
//! At the layout's own width rather than the viewport's, as the package page's
//! container query does: a gallery cell is not the window, and the layout
//! should answer for the room it has. A `ResizeObserver` measures it, because
//! the switch moves elements and CSS cannot; it also sets `data-wide`, so the
//! stylesheet keys on the same decision instead of a second breakpoint.

use leptos::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::Closure;

stylance::import_crate_style!(style, "src/kit/two_columns.module.scss");

/// The layout's own width from which it is two columns: a 1024px window, less
/// the page column's 16px padding a side, less 16 for a classic scrollbar,
/// since a page tall enough to need two columns scrolls. Where scrollbars
/// overlay the page it switches at a 1008px window, which nobody can tell.
pub const TWO_COLUMNS_FROM: i32 = 976;

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

/// A region's wrapper, and where it goes in each arrangement.
#[derive(Clone, Copy)]
struct Placed {
    narrow: u8,
    right: bool,
    node: NodeRef<leptos::html::Div>,
}

fn wrap(items: Vec<ColumnItem>, right: bool, placed: &mut Vec<Placed>) -> Vec<AnyView> {
    items
        .into_iter()
        .map(|item| {
            let node = NodeRef::new();
            placed.push(Placed {
                narrow: item.narrow,
                right,
                node,
            });
            view! { <div node_ref=node>{item.view}</div> }.into_any()
        })
        .collect()
}

/// Put every region where `wide` says, keeping focus where it was.
fn arrange(
    frame: &web_sys::HtmlElement,
    left: &web_sys::Element,
    right: &web_sys::Element,
    placed: &[Placed],
    wide: bool,
) {
    let focused = document()
        .active_element()
        .filter(|el| frame.contains(Some(el)));
    let mut order: Vec<Placed> = placed.to_vec();
    if !wide {
        order.sort_by_key(|p| p.narrow);
    }
    for p in order {
        let Some(node) = p.node.get_untracked() else {
            continue;
        };
        let column = if wide && p.right { right } else { left };
        let _ = column.append_child(&node);
    }
    if wide {
        let _ = frame.set_attribute("data-wide", "");
    } else {
        let _ = frame.remove_attribute("data-wide");
    }
    if let Some(el) = focused.and_then(|el| el.dyn_into::<web_sys::HtmlElement>().ok())
        && document().active_element().as_ref() != Some(&el)
    {
        let _ = el.focus();
    }
}

type Observing = (web_sys::ResizeObserver, Closure<dyn FnMut()>);

#[component]
pub fn TwoColumns(
    /// The left column, top to bottom.
    left: Vec<ColumnItem>,
    /// The right column, top to bottom.
    right: Vec<ColumnItem>,
) -> impl IntoView {
    let mut placed = Vec::new();
    let left = wrap(left, false, &mut placed);
    let right = wrap(right, true, &mut placed);

    let frame = NodeRef::<leptos::html::Div>::new();
    let left_column = NodeRef::<leptos::html::Div>::new();
    let right_column = NodeRef::<leptos::html::Div>::new();
    let shown: StoredValue<Option<bool>, LocalStorage> = StoredValue::new_local(None);
    let update = move || {
        let (Some(frame), Some(l), Some(r)) = (
            frame.get_untracked(),
            left_column.get_untracked(),
            right_column.get_untracked(),
        ) else {
            return;
        };
        // Unattached, the frame has no width yet; the observer reports the real one.
        if !frame.is_connected() {
            return;
        }
        let wide = frame.client_width() >= TWO_COLUMNS_FROM;
        if shown.get_value() != Some(wide) {
            arrange(&frame, &l, &r, &placed, wide);
            shown.set_value(Some(wide));
        }
    };
    let held: StoredValue<Option<Observing>, LocalStorage> = StoredValue::new_local(None);
    frame.on_load(move |el| {
        let callback = Closure::<dyn FnMut()>::new(update);
        if let Ok(observer) = web_sys::ResizeObserver::new(callback.as_ref().unchecked_ref()) {
            observer.observe(&el);
            held.set_value(Some((observer, callback)));
        }
    });
    on_cleanup(move || {
        held.try_update_value(|held| {
            if let Some((observer, _)) = held.take() {
                observer.disconnect();
            }
        });
    });

    view! {
        <div class=style::frame node_ref=frame>
            <div class=style::columns>
                <div class=style::column node_ref=left_column>{left}</div>
                <div class=style::column node_ref=right_column>{right}</div>
            </div>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{load_module_style, mount, sleep_ms, unmount_earlier};
    use wasm_bindgen_test::*;

    fn load_stylesheet() {
        load_module_style(
            include_str!("two_columns.module.scss"),
            &[
                ("frame", style::frame),
                ("columns", style::columns),
                ("column", style::column),
            ],
        );
    }

    /// A card `height` px tall holding a field, named by its text.
    fn card(name: &'static str, height: u32) -> AnyView {
        view! {
            <section style=format!("height:{height}px")>
                <p style="margin:0">{name}</p>
                <input aria-label=name />
            </section>
        }
        .into_any()
    }

    /// Two columns of cards, `a` tall; narrow, they read a, b, c, d.
    async fn layout(width: u32) -> (web_sys::Element, web_sys::HtmlElement) {
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
        sleep_ms(50).await;
        let sized: web_sys::HtmlElement = el.first_element_child().unwrap().unchecked_into();
        (el, sized)
    }

    /// Each card's name, left and top, in DOM order: what Tab and a screen
    /// reader follow.
    fn cards(el: &web_sys::Element) -> Vec<(String, f64, f64)> {
        let found = el.query_selector_all("p").unwrap();
        (0..found.length())
            .map(|i| {
                let p: web_sys::Element = found.get(i).unwrap().unchecked_into();
                let rect = p.get_bounding_client_rect();
                (p.text_content().unwrap(), rect.left(), rect.top())
            })
            .collect()
    }

    fn names(cards: &[(String, f64, f64)]) -> Vec<&str> {
        cards.iter().map(|c| c.0.as_str()).collect()
    }

    /// Under 976px: one column, and the DOM, so Tab, follows what is drawn.
    #[wasm_bindgen_test]
    #[allow(clippy::float_cmp, reason = "box edges here are whole pixels")]
    async fn narrow_reads_and_draws_one_column_in_the_pages_order() {
        let (el, _) = layout(960).await;
        let drawn = cards(&el);
        assert_eq!(names(&drawn), ["a", "b", "c", "d"]);
        assert!(
            drawn.iter().all(|c| c.1 == drawn[0].1),
            "one column: {drawn:?}"
        );
        assert!(
            drawn.windows(2).all(|w| w[0].2 < w[1].2),
            "top to bottom: {drawn:?}"
        );
    }

    /// From 976px: two columns, each stacking on its own, so the right one
    /// does not wait for the tall card on the left; the DOM reads left, then right.
    #[wasm_bindgen_test]
    #[allow(clippy::float_cmp, reason = "box edges here are whole pixels")]
    async fn wide_draws_two_columns_that_stack_on_their_own() {
        let (el, _) = layout(976).await;
        let drawn = cards(&el);
        assert_eq!(names(&drawn), ["a", "c", "b", "d"]);
        let [a, c, b, d] = [&drawn[0], &drawn[1], &drawn[2], &drawn[3]];
        assert!(b.1 > a.1, "b sits right of a: {drawn:?}");
        assert_eq!(
            (b.2, c.1, d.1),
            (a.2, a.1, b.1),
            "b level with a; c under a; d under b"
        );
        assert!(c.2 >= a.2 + 300.0, "c under the tall a");
        assert!(d.2 < a.2 + 300.0, "d does not wait for a: {drawn:?}");
    }

    /// Crossing the width moves the cards, and a field being typed in keeps focus.
    #[wasm_bindgen_test]
    async fn a_focused_field_keeps_focus_across_the_switch() {
        // An earlier test's open modal would make this field inert.
        unmount_earlier();
        let (el, sized) = layout(960).await;
        let field: web_sys::HtmlElement = el
            .query_selector("input[aria-label=d]")
            .unwrap()
            .unwrap()
            .unchecked_into();
        field.focus().unwrap();

        sized.style().set_property("width", "976px").unwrap();
        sleep_ms(50).await;
        assert_eq!(names(&cards(&el)), ["a", "c", "b", "d"], "now wide");
        assert_eq!(document().active_element(), Some(field.clone().into()));

        sized.style().set_property("width", "960px").unwrap();
        sleep_ms(50).await;
        assert_eq!(names(&cards(&el)), ["a", "b", "c", "d"], "narrow again");
        assert_eq!(document().active_element(), Some(field.into()));
    }
}
