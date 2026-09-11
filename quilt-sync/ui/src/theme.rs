//! Follow the OS colour scheme.
//!
//! The dark palette has existed since the tokens were written and nothing in the
//! app could ever reach it: `_tokens.scss` declares it under
//! `:root[data-theme="dark"]`, the gallery set that attribute from its own
//! button, and the app set it from nowhere (qhq-8mgw.56). This is the missing
//! half, and the shape the tokens' own comment asks for — *"following the OS
//! means setting `data-theme` from Rust, not duplicating 60 lines into a
//! `prefers-color-scheme` block"*.
//!
//! # v1 is unaffected, and that is checked rather than hoped
//!
//! `data-theme` lands on the root, so it is in scope for v1's pages too. They do
//! not move: every one of the 345 custom-property reads across v1's stylesheets
//! is a `--q-ui-*` token of its own, none of which this switches, and `body`
//! sets a colour but never a background. v2's ground comes from `PageLayout`,
//! which paints `--q-bgColor-page` itself.

use leptos::prelude::document;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::Closure;

/// The media query the OS answers.
const DARK: &str = "(prefers-color-scheme: dark)";

/// Sets `data-theme` on the **root element**, which is the only place it works.
///
/// Tier-2 tokens are declared on `:root` as `var()` references to tier 1, and a
/// custom property's `var()` is resolved where the *declaration* sits — not
/// where it is used. So `--q-fgColor-default` is computed once against whichever
/// tier-1 values `:root` sees, and descendants inherit that already-resolved
/// colour. Putting `.dark` on a wrapper element therefore changes nothing.
pub fn set(dark: bool) {
    if let Some(root) = document().document_element() {
        let value = if dark { "dark" } else { "light" };
        drop(root.set_attribute("data-theme", value));
    }
}

/// Adopt the OS scheme now, and again whenever it changes.
///
/// The listener is deliberately never detached: it is registered once for the
/// life of the page, and dropping the closure would unregister it immediately.
///
/// A webview with no `matchMedia` leaves the attribute unset, which is the light
/// palette — the same thing every build did before this existed, so the failure
/// mode is the old behaviour rather than a broken one.
pub fn follow_os() {
    let Some(window) = web_sys::window() else {
        return;
    };
    let Ok(Some(query)) = window.match_media(DARK) else {
        return;
    };
    set(query.matches());

    let on_change = Closure::<dyn FnMut(web_sys::MediaQueryListEvent)>::new(
        move |event: web_sys::MediaQueryListEvent| set(event.matches()),
    );
    query.set_onchange(Some(on_change.as_ref().unchecked_ref()));
    on_change.forget();
}

/// The class marking that the reader has opted into v2's design system.
///
/// Not "the v2 page is showing": the toast layer is mounted outside the router
/// (`main.rs`), so it is a sibling of every page and cannot be scoped by one.
/// Only an ancestor of both can carry this, and the root is the ancestor of
/// everything.
pub const V2_CLASS: &str = "qui-v2";

/// Record whether `main_page_v2` is on, for the chrome that sits outside every
/// page and therefore cannot ask.
///
/// v1's stylesheets read only their own `--q-ui-*` tokens, which no theme
/// switches, so v1 stays light whatever the OS says. Anything shared between
/// the two — the toast layer is the only such thing today — must follow the
/// theme for a v2 reader and stay put for a v1 one, and this is what lets a
/// stylesheet tell them apart.
pub fn set_v2(on: bool) {
    if let Some(root) = document().document_element() {
        let list = root.class_list();
        let result = if on {
            list.add_1(V2_CLASS)
        } else {
            list.remove_1(V2_CLASS)
        };
        drop(result);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::*;

    /// The attribute has to land on the ROOT element specifically — see `set`'s
    /// own doc for why anywhere else is inert. Reading it back off
    /// `document_element` is what makes that part of the assertion: a version
    /// that wrote to `body` would leave this `None`.
    #[wasm_bindgen_test]
    fn the_v2_marker_goes_on_and_comes_off_the_root() {
        // Same element as the theme, and for a related reason: what reads this
        // is the toast layer, which is a sibling of every page rather than a
        // descendant of one. Toggling BOTH ways matters — a reader who turns
        // the flag off must stop getting v2's palette on shared chrome.
        let root = document().document_element().expect("a root element");

        set_v2(true);
        assert!(root.class_list().contains(V2_CLASS));

        set_v2(false);
        assert!(
            !root.class_list().contains(V2_CLASS),
            "turning the flag off must take the marker with it"
        );
    }

    #[wasm_bindgen_test]
    fn the_theme_lands_on_the_root_where_the_tokens_can_see_it() {
        let root = document().document_element().expect("a root element");

        set(true);
        assert_eq!(root.get_attribute("data-theme").as_deref(), Some("dark"));

        set(false);
        assert_eq!(
            root.get_attribute("data-theme").as_deref(),
            Some("light"),
            "and back — a one-way switch would strand a reader in dark"
        );
    }

    /// The reset in `app.scss` reaches v1's pages, whose ink no theme switches,
    /// and `follow_os` cannot see the flag — so a themed ground on `body` would
    /// go dark under v1's black text. Pinned on the stylesheet, where it was
    /// added once already.
    #[test]
    fn the_shared_reset_paints_no_ground() {
        const BASE: &str = include_str!("../assets/css/kit/_base.scss");
        let body = BASE
            .split("\nbody {")
            .nth(1)
            .and_then(|rest| rest.split('}').next())
            .expect("a body rule");
        assert!(
            !body.contains("background"),
            "the shared reset must not paint a ground: {body}"
        );
    }
}
