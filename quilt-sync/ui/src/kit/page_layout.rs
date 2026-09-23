//! The page frame: the [`Appbar`], and a width-capped column for the regions.
//!
//! The bar is its own unit rather than part of this — see [`Appbar`] for why —
//! so what the frame adds is the **surface**: `data-v2-page`, the page's ground,
//! ink and type. That is the part a v1 page must never be wrapped in.
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

use leptos::prelude::*;

use crate::kit::Appbar;

stylance::import_crate_style!(style, "src/kit/page_layout.module.scss");

#[component]
pub fn PageLayout(
    /// The page's name. Drawn only for a screen reader: the appbar carries the
    /// wordmark and the regions below are `h2`, so without this they hang from
    /// nothing and heading navigation has no top.
    #[prop(into)]
    heading: String,
    /// Appbar controls, handed straight to the [`Appbar`]'s slot.
    #[prop(optional)]
    actions: Option<AnyView>,
    /// A `Banner`, when there is one. In the flow directly under the appbar, so it
    /// pushes the page down rather than floating over it — the design bans anchored
    /// positioning, and a bar cannot be missed by someone looking at the bottom of a long
    /// list. A slot rather than the signal itself, so the layout stays ignorant of what
    /// kinds exist and who dismisses them.
    #[prop(optional)]
    banner: Option<AnyView>,
    children: Children,
) -> impl IntoView {
    view! {
        // `data-v2-page` marks the surface the v2 palette paints, which is what
        // `color-scheme` keys on: `qui-v2` marks a READER who opted in, and that
        // reader still visits v1's pages.
        <div class=style::root data-v2-page>
            <h1 data-sr-only>{heading}</h1>
            <Appbar actions=actions />
            {banner
                .map(|banner| {
                    view! { <div class=style::notice>{banner}</div> }
                })}
            <main class=style::main>{children()}</main>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::mount;
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
