//! The appbar: the logo that goes home, and the controls pushed right.
//!
//! # Why it is a unit of its own, and not part of the frame
//!
//! It used to be drawn inside [`PageLayout`](crate::kit::PageLayout), and it had
//! to leave before any v1 page could wear it. The obstacle was never the bar:
//! the frame stamps `data-v2-page`, the marker `_base.scss` hangs the dark
//! `color-scheme` on, and paints the page's ground and ink — so lending a v1
//! page the frame for its bar repainted the v1 page underneath.
//!
//! **So this carries no surface marker, and must not grow one.** It paints only
//! itself. The bar's own look needed no settling: its ground is the brand colour
//! in both themes, and v1's bar has always been that colour, in a stylesheet
//! with no dark theme.
//!
//! # What it owns
//!
//! Its own ground, ink and type. Inside the frame the frame's root would supply
//! the last two, but outside it the bar inherits v1's `body` — Roboto, and a
//! black that no theme switches — so it states them rather than borrowing them.
//!
//! No page policy. Which controls sit on the right is the caller's slot, and
//! what they do is the caller's too.

use leptos::prelude::*;

stylance::import_crate_style!(style, "src/kit/appbar.module.scss");

#[component]
pub fn Appbar(
    /// Controls, pushed to the right — Refresh and Settings as labelled
    /// `Button`s wherever the redesigned bar is drawn today. A slot rather than
    /// named props, because the bar has no opinion about which page needs which
    /// controls.
    ///
    /// An `Option` rather than an optional prop, so a host holding one of its
    /// own — the frame does — can pass it straight through.
    #[prop(default = None)]
    actions: Option<AnyView>,
) -> impl IntoView {
    view! {
        // `header` rather than a div: with the frame's `main` it is one of the
        // two landmarks a screen reader offers to skip between, and on a v1 page
        // it is the only one.
        <header class=style::appbar>
            <div class=style::bar>
                <a class=style::logo href="/">
                    // v1's own asset, on a bar that is v1's own colour. It is the
                    // only logo in the repo with an alpha channel, which is the
                    // whole of qhq-8mgw.22: `quilt-mark.png` was PNG colour-type 2
                    // with no alpha at all, so it carried an opaque square that was
                    // merely INVISIBLE while the bar was white, and showed its
                    // corners the moment dark theme landed.
                    //
                    // Alt text, not `aria-hidden`: it is the only content of a link,
                    // so hiding it would leave the link unnamed.
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

    /// qhq-8mgw.22. The appbar's ground is the brand colour in both themes, so
    /// its mark has to be the one asset in the repo that can sit on a coloured
    /// ground — `quilt.png`, which is PNG colour-type 6 and half transparent.
    ///
    /// `quilt-mark.png`, which this replaced, was colour-type 2: no alpha at
    /// all, so it carried an opaque square. That was invisible for as long as
    /// the square's white happened to match the bar, and no test could see the
    /// difference — which is how it survived until dark theme made the corners
    /// show. Pinning the filename is what a test CAN hold: the asset's own
    /// format is checked where assets are, not here.
    #[wasm_bindgen_test]
    fn the_appbar_mark_is_the_asset_with_an_alpha_channel() {
        let el = mount(|| view! { <Appbar /> });
        let img = el
            .query_selector("header img")
            .unwrap()
            .expect("the appbar draws a mark");
        let src = img.get_attribute("src").expect("the mark has a src");
        assert!(
            src.ends_with("/quilt.png"),
            "the bar is brand-coloured, so the mark must be the RGBA asset: {src}"
        );
        assert!(
            !img.get_attribute("alt").unwrap_or_default().is_empty(),
            "the mark is the only content of a link, so it has to name it"
        );
    }

    /// The reason this is a unit at all. A v1 page wears it, and the marker is
    /// what turns a surface dark — so a bar that carried one would repaint the
    /// v1 page under it, which is exactly what extracting it from the frame was
    /// for.
    #[wasm_bindgen_test]
    fn the_bar_carries_no_surface_marker() {
        let el = mount(
            || view! { <Appbar actions=Some(view! { <button>"Refresh"</button> }.into_any()) /> },
        );
        assert!(
            el.query_selector("[data-v2-page]").unwrap().is_none(),
            "the bar must not mark a v2 surface: v1 pages sit under it"
        );
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
