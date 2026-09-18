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
    /// Reactive, so the queue can update its count without rebuilding the card
    /// — and with it the keyed row list underneath, whose whole purpose is to
    /// survive a settle (`pages/main_page/queue.rs`).
    #[prop(optional, into)]
    count: MaybeProp<usize>,
    /// The name for a card with no visible title, drawn as a heading only screen
    /// readers reach. A `section` with no accessible name is not exposed as a region
    /// at all, so the list card was unreachable by both region and heading navigation.
    #[prop(optional, into)]
    label: Option<String>,
    /// Rows are list items. The hairline is `.body > * + *`, so it divides `li`
    /// siblings exactly as it divided `div` ones.
    #[prop(optional)]
    list: bool,
    /// The rows are skeletons and the real ones are still being read. `SkeletonBox`
    /// hides itself from the accessibility tree and its doc says the composing region
    /// states this; nothing did, so a reader got an empty card and a silent swap.
    #[prop(optional, into)]
    busy: Signal<bool>,
    /// Rows. The card draws a hairline between any two of them, so children need not
    /// know they are in a list — pass a single wrapper element to opt out, as the queue
    /// does, where dividers would make a list of decisions read as a table.
    children: Children,
) -> impl IntoView {
    let heading_id = super::unique_id("card-title");
    let labelled_by = heading_id.clone();
    // Visible or not, the card has one heading and the section is named by it.
    let (heading, hidden) = match (title, label) {
        (Some(title), _) => (Some(title), false),
        (None, Some(label)) => (Some(label), true),
        (None, None) => (None, false),
    };
    let named = heading.is_some().then_some(labelled_by);

    view! {
        // `h2`: the page's regions are h2, and a card is a region. If a caller ever
        // needs a different level, that is a prop — not a hard-coded guess repeated at
        // each call site.
        <section
            class=style::root
            aria-labelledby=named
            aria-busy=move || busy.get().then_some("true")
        >
            {heading
                .map(|heading| {
                    let class = if hidden { None } else { Some(style::title) };
                    view! {
                        <h2 id=heading_id class=class data-sr-only=hidden.then_some("")>
                            {heading}
                            {move || {
                                count
                                    .get()
                                    .map(|count| {
                                        view! {
                                            // Read aloud, `Needs your attention` and
                                            // `(19)` abut without it.
                                            <span data-sr-only>", "</span>
                                            <span class=style::count>{format!("({count})")}</span>
                                        }
                                    })
                            }}
                        </h2>
                    }
                })}
            {if list {
                view! { <ul class=style::body role="list">{children()}</ul> }.into_any()
            } else {
                view! { <div class=style::body>{children()}</div> }.into_any()
            }}
        </section>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::mount;
    use wasm_bindgen_test::*;

    /// A `section` with no accessible name is not a region at all, so every card
    /// points at its own heading.
    #[wasm_bindgen_test]
    fn a_card_is_named_by_its_heading() {
        let el = mount(|| view! { <Card title="Autosync">"rows"</Card> });
        let section = el.query_selector("section").unwrap().expect("a section");
        let id = section
            .get_attribute("aria-labelledby")
            .expect("a card names itself");
        let heading = el
            .query_selector(&format!("#{id}"))
            .unwrap()
            .expect("the id it points at");
        assert_eq!(heading.tag_name(), "H2");
        assert_eq!(heading.text_content().as_deref(), Some("Autosync"));
    }

    /// The list card has no visible title — the view toggle names it — so the name
    /// is a heading only a screen reader reaches.
    #[wasm_bindgen_test]
    fn a_card_with_no_visible_title_still_has_a_heading() {
        let el = mount(|| view! { <Card label="Packages">"rows"</Card> });
        let heading = el.query_selector("h2").unwrap().expect("a heading");
        assert!(heading.has_attribute("data-sr-only"), "and it is not drawn");
        assert_eq!(
            el.query_selector("section")
                .unwrap()
                .unwrap()
                .get_attribute("aria-labelledby")
                .as_deref(),
            heading.get_attribute("id").as_deref(),
        );
    }

    /// Read aloud, the title and the count abut without a separator between them.
    #[wasm_bindgen_test]
    fn the_count_is_separated_from_the_title() {
        let el = mount(|| view! { <Card title="Needs your attention" count=19>"rows"</Card> });
        let text = el
            .query_selector("h2")
            .unwrap()
            .unwrap()
            .text_content()
            .unwrap();
        assert!(
            !text.contains("attention("),
            "the count runs into the title: {text}"
        );
        assert!(text.contains("(19)"), "{text}");
    }

    /// Rows announce as a list when the caller says they are one.
    #[wasm_bindgen_test]
    fn a_list_card_holds_its_rows_in_a_list() {
        let el = mount(|| view! { <Card list=true><li>"one"</li></Card> });
        let list = el.query_selector("ul > li").unwrap().expect("a list item");
        assert_eq!(
            list.parent_element()
                .unwrap()
                .get_attribute("role")
                .as_deref(),
            Some("list"),
            "WebKit drops the semantics with `list-style: none`, so it is stated"
        );
    }

    /// `SkeletonBox` hides itself from the accessibility tree, so without this a
    /// reader gets an empty card and hears nothing when the rows arrive.
    #[wasm_bindgen_test]
    fn a_card_of_skeletons_says_it_is_busy() {
        let el = mount(|| view! { <Card label="Packages" busy=true>"skeletons"</Card> });
        assert_eq!(
            el.query_selector("section")
                .unwrap()
                .unwrap()
                .get_attribute("aria-busy")
                .as_deref(),
            Some("true")
        );

        let settled = mount(|| view! { <Card label="Packages">"rows"</Card> });
        assert!(
            settled
                .query_selector("section")
                .unwrap()
                .unwrap()
                .get_attribute("aria-busy")
                .is_none(),
            "and drops it once they land"
        );
    }
}
