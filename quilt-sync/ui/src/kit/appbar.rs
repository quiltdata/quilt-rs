//! The appbar: the home logo, and a slot for controls on the right.
//!
//! Drawn by both the v2 frame and the v1 `Layout`, so it must never carry
//! `data-v2-page`: that marker darkens the surface it is on, and v1 pages sit
//! under this bar.

use leptos::prelude::*;

stylance::import_crate_style!(style, "src/kit/appbar.module.scss");

#[component]
pub fn Appbar(#[prop(default = None)] actions: Option<AnyView>) -> impl IntoView {
    view! {
        <header class=style::appbar>
            <div class=style::bar>
                <a class=style::logo href="/">
                    // The only logo asset with an alpha channel (qhq-8mgw.22).
                    <img src="/assets/img/quilt.png" alt="QuiltSync home" />
                </a>
                {actions.map(|actions| view! { <span class=style::actions>{actions}</span> })}
            </div>
        </header>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::mount;
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
}
