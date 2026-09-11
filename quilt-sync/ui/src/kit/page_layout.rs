//! The page frame: an appbar and a width-capped column for the regions.
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
//! **Breadcrumbs** — the main page is the root, so its trail is one item, and a breadcrumb
//! of one is decoration. The first v2 page with a parent adds them. That is the only thing
//! left; the banner host arrived, and `ui_locked` is not arriving.

use leptos::prelude::*;

stylance::import_crate_style!(style, "src/kit/page_layout.module.scss");

#[component]
pub fn PageLayout(
    /// Appbar controls, pushed to the right — on the main page, refresh and settings
    /// as Framed `IconButton`s. A slot rather than named props, because the appbar has
    /// no opinion about which page needs which controls.
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
        <div class=style::root>
            // `header` and `main` rather than divs: they are the two landmarks a
            // screen reader offers to skip between, and they cost nothing.
            <header class=style::appbar>
                <div class=style::bar>
                    <a class=style::logo href="/">
                        // v1's own asset, on a bar that is now v1's own colour. It is
                        // the only logo in the repo with an alpha channel, which is the
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
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::*;

    fn mount<N: IntoView + 'static>(f: impl FnOnce() -> N + Send + 'static) -> web_sys::Element {
        let doc = web_sys::window().unwrap().document().unwrap();
        let container: web_sys::HtmlElement =
            doc.create_element("div").unwrap().dyn_into().unwrap();
        doc.body().unwrap().append_child(&container).unwrap();
        leptos::mount::mount_to(container.clone(), f).forget();
        container.into()
    }

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
        let el = mount(|| view! { <PageLayout>"body"</PageLayout> });
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
}
