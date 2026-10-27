//! The page frame: the [`Appbar`], and a width-capped column for the regions.
//!
//! # What it owns, and why that matters
//!
//! **The space between regions.** The state strip, the queue and the list region set
//! no outer margins of their own — this sets one gap and they inherit the rhythm. A
//! region that spaced itself would look right alone and wrong beside the others, and
//! there would be no single place to change how the page breathes.
//!
//! # `ui_locked` is not here, and is not coming
//!
//! v1 disables the whole page behind an overlay while an operation runs. **Rejected for
//! v2**, decided 2026-08-07 — and it is worth recording why, because it looks like a
//! missing feature and is not one.
//!
//! It exists in v1 because a single boolean was the only vocabulary available: with no way
//! to say *this row is busy* or *this region is loading*, locking everything is the only
//! honest thing left. v2 has that vocabulary — `busy` on a queue row's action, `SkeletonBox`
//! per region, `loading` on a `Button`, `spinning` on the appbar's refresh — so the
//! overlay's job is now done by indicators that are truthful about their scope.
//!
//! And the overlay has a real cost that its scope-accurate replacements do not: it stops
//! the user reading the page, which is exactly what people do while waiting.
//!
//! **The consequence, which is the part to be careful about.** The overlay was also a
//! safety net — it covered any operation whose progress nobody remembered to show. Without
//! it, *every* operation must own a visible indicator at its own scope, because there is
//! no longer a backstop that makes forgetting merely ugly instead of invisible.
//!
//! # What is deliberately not here yet
//!
//! **Breadcrumbs** — not here, and the first v2 page with a parent did not add
//! them. The package page's trail is a `BackLink` in its own header: a trail of
//! two is a back-link in costume, the second crumb restating the title an inch
//! below it, and it costs a band against that page's vertical budget. The trail
//! earns an appbar of its own at three, which is `/commit`. The banner host
//! arrived, and `ui_locked` is not arriving.

use std::cell::RefCell;
use std::rc::Rc;

use leptos::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;

use crate::kit::Appbar;

stylance::import_crate_style!(style, "src/kit/page_layout.module.scss");

#[component]
pub fn PageLayout(
    /// The page's name. Drawn only for a screen reader: the appbar carries the
    /// wordmark and the regions below are `h2`, so without this they hang from
    /// nothing and heading navigation has no top.
    #[prop(into)]
    heading: String,
    #[prop(optional)] actions: Option<AnyView>,
    /// A `Banner`, when there is one. In the flow directly under the appbar, so it
    /// pushes the page down rather than floating over it — the design bans anchored
    /// positioning, and a bar cannot be missed by someone looking at the bottom of a long
    /// list. A slot rather than the signal itself, so the layout stays ignorant of what
    /// kinds exist and who dismisses them.
    #[prop(optional)]
    banner: Option<AnyView>,
    /// The logo navigates in place of this page's history entry, for a page
    /// whose mount is its operation.
    #[prop(optional)]
    replace: bool,
    /// The page scrolls as one, under parts that stick: the appbar here, and
    /// whatever bands the page pins below it, each offset by the heights
    /// [`publish_height`] writes on this frame.
    #[prop(optional)]
    page_scroll: bool,
    children: Children,
) -> impl IntoView {
    view! {
        // `data-v2-page` marks the surface the v2 palette paints, which is what
        // `color-scheme` keys on: `qui-v2` marks a READER who opted in, and that
        // reader still visits v1's pages.
        <div class=style::root data-v2-page data-page-scroll=page_scroll.then_some("")>
            <h1 data-sr-only>{heading}</h1>
            <Appbar actions=actions replace=replace />
            {banner
                .map(|banner| {
                    view! { <div class=style::notice>{banner}</div> }
                })}
            <main class=style::main>{children()}</main>
        </div>
    }
}

type Observing = (
    web_sys::ResizeObserver,
    Closure<dyn FnMut()>,
    Rc<RefCell<Option<web_sys::HtmlElement>>>,
);

/// Keep `var` on the page frame at the height of `node`, so a sticky part
/// below it can take that as its offset. The variable goes with the element:
/// a part the page does not draw counts as no height at all.
///
/// The frame is found from the element when it is first measured, since an
/// element built inside a `Transition` is not in the page yet when it loads.
pub fn publish_height(node: NodeRef<leptos::html::Div>, var: &'static str) {
    let held: StoredValue<Option<Observing>, LocalStorage> = StoredValue::new_local(None);
    node.on_load(move |el| {
        let frame: Rc<RefCell<Option<web_sys::HtmlElement>>> = Rc::default();
        let found = Rc::clone(&frame);
        let target = el.clone();
        let callback = Closure::<dyn FnMut()>::new(move || {
            let mut found = found.borrow_mut();
            if found.is_none() {
                *found = target
                    .closest("[data-v2-page]")
                    .ok()
                    .flatten()
                    .map(JsCast::unchecked_into);
            }
            if let Some(frame) = found.as_ref() {
                let height = target.get_bounding_client_rect().height();
                let _ = frame.style().set_property(var, &format!("{height}px"));
            }
        });
        if let Ok(observer) = web_sys::ResizeObserver::new(callback.as_ref().unchecked_ref()) {
            observer.observe(&el);
            held.set_value(Some((observer, callback, frame)));
        }
    });
    on_cleanup(move || {
        held.try_update_value(|held| {
            if let Some((observer, _, frame)) = held.take() {
                observer.disconnect();
                if let Some(frame) = frame.borrow().as_ref() {
                    let _ = frame.style().remove_property(var);
                }
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::mount;
    use crate::test_support::sleep_ms;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::*;

    /// The stylesheet hangs the dark `color-scheme` on this attribute, so a page
    /// that lost it would put v1's ink on a dark canvas again.
    #[wasm_bindgen_test]
    fn the_frame_marks_itself_as_a_v2_surface() {
        let el = mount(|| view! { <PageLayout heading="Page">"body"</PageLayout> });
        assert!(
            el.query_selector("[data-v2-page]").unwrap().is_some(),
            "the v2 frame has to name itself"
        );
    }
    /// A sticky part below the measured one takes this as its offset, and a
    /// part that is gone must stop holding the room it took.
    #[wasm_bindgen_test]
    async fn a_measured_part_publishes_its_height_until_it_goes() {
        #[component]
        fn Band() -> impl IntoView {
            let band = NodeRef::new();
            publish_height(band, "--q-test-height");
            view! { <div node_ref=band style="height:40px"></div> }
        }
        let shown = RwSignal::new(true);
        let el = mount(move || {
            view! {
                <PageLayout heading="Page" page_scroll=true>
                    <Show when=move || shown.get()>
                        <Band />
                    </Show>
                </PageLayout>
            }
        });
        let frame: web_sys::HtmlElement = el
            .query_selector("[data-v2-page]")
            .unwrap()
            .expect("the frame")
            .unchecked_into();
        let height = || frame.style().get_property_value("--q-test-height").unwrap();
        sleep_ms(50).await;
        assert_eq!(height(), "40px");
        shown.set(false);
        sleep_ms(50).await;
        assert_eq!(height(), "", "no part, no height");
    }

    /// One `h1`, and only a screen reader sees it: the appbar carries the wordmark,
    /// so a visible one would be a second title.
    #[wasm_bindgen_test]
    fn the_page_has_exactly_one_top_level_heading() {
        let el = mount(|| view! { <PageLayout heading="QuiltSync">"body"</PageLayout> });
        let headings = el.query_selector_all("h1").unwrap();
        assert_eq!(headings.length(), 1);
        let h1 = headings.get(0).unwrap();
        let h1: web_sys::Element = h1.dyn_into().unwrap();
        assert_eq!(h1.text_content().as_deref(), Some("QuiltSync"));
        assert!(h1.has_attribute("data-sr-only"), "and it is not drawn");
    }
}
