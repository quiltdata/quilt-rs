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

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::*;

    /// The attribute has to land on the ROOT element specifically — see `set`'s
    /// own doc for why anywhere else is inert. Reading it back off
    /// `document_element` is what makes that part of the assertion: a version
    /// that wrote to `body` would leave this `None`.
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
}
