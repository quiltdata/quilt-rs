//! The appbar: the home logo on the left, the [`ActivityLine`] centered, and a
//! slot for controls on the right.
//!
//! Drawn by both the v2 frame and the v1 `Layout`, so it must never carry
//! `data-v2-page`: that marker darkens the surface it is on, and v1 pages sit
//! under this bar.

use leptos::prelude::*;

use crate::kit::ActivityLine;

stylance::import_crate_style!(style, "src/kit/appbar.module.scss");

#[component]
pub fn Appbar(
    #[prop(default = None)] actions: Option<AnyView>,
    /// The logo navigates in place of the current history entry.
    #[prop(optional)]
    replace: bool,
) -> impl IntoView {
    view! {
        <header class=style::appbar>
            <div class=style::bar>
                <a class=style::logo href="/" prop:replace=replace>
                    // The only logo asset with an alpha channel (qhq-8mgw.22).
                    <img src="/assets/img/quilt.png" alt="QuiltSync home" />
                </a>
                <ActivityLine />
                {actions.map(|actions| view! { <span class=style::actions>{actions}</span> })}
            </div>
        </header>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::Activities;
    use crate::test_support::mount;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::*;

    /// qhq-8mgw.22: `quilt-mark.png` has no alpha and shows an opaque square
    /// on the brand ground.
    #[wasm_bindgen_test]
    fn the_appbar_mark_is_the_asset_with_an_alpha_channel() {
        let el = mount(|| view! { <Appbar /> });
        let img = el
            .query_selector("header img")
            .unwrap()
            .expect("the appbar draws a mark");
        let src = img.get_attribute("src").expect("the mark has a src");
        assert!(src.ends_with("/quilt.png"), "{src}");
        assert!(
            !img.get_attribute("alt").unwrap_or_default().is_empty(),
            "the mark is the only content of a link, so it has to name it"
        );
    }

    #[wasm_bindgen_test]
    fn the_bar_carries_no_surface_marker() {
        let el = mount(
            || view! { <Appbar actions=Some(view! { <button>"Refresh"</button> }.into_any()) /> },
        );
        assert!(el.query_selector("[data-v2-page]").unwrap().is_none());
    }

    #[wasm_bindgen_test]
    fn the_logo_goes_home_and_the_controls_are_drawn() {
        let el = mount(
            || view! { <Appbar actions=Some(view! { <button>"Refresh"</button> }.into_any()) /> },
        );
        let logo = el.query_selector("header a").unwrap().expect("a logo link");
        assert_eq!(logo.get_attribute("href").as_deref(), Some("/"));
        let button = el
            .query_selector("header button")
            .unwrap()
            .expect("the slot is drawn");
        assert_eq!(button.text_content().as_deref(), Some("Refresh"));
    }

    /// The gallery and every test mount the bar with no `Activities` above it.
    #[wasm_bindgen_test]
    fn without_activities_nothing_is_drawn() {
        let el = mount(
            || view! { <Appbar actions=Some(view! { <button>"Refresh"</button> }.into_any()) /> },
        );
        assert!(
            el.query_selector("[role=status]").unwrap().is_none(),
            "markup was {}",
            el.inner_html()
        );
    }

    #[wasm_bindgen_test]
    fn the_line_sits_between_the_logo_and_the_controls() {
        let el = mount(|| {
            provide_context(Activities::new());
            view! { <Appbar actions=Some(view! { <button>"Refresh"</button> }.into_any()) /> }
        });
        let all = el
            .query_selector_all("header img, header [role=status], header button")
            .unwrap();
        let order: Vec<String> = (0..all.length())
            .map(|i| {
                let node: web_sys::Element = all.item(i).unwrap().unchecked_into();
                if node.get_attribute("role").as_deref() == Some("status") {
                    "line".to_owned()
                } else {
                    node.tag_name().to_lowercase()
                }
            })
            .collect();
        assert_eq!(order, ["img", "line", "button"]);
    }
}
