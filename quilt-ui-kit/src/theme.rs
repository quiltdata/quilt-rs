//! Follow the OS colour scheme.
//!
//! `tokens.css` declares the dark palette under `:root[data-theme="dark"]`.
//! The gallery sets that attribute from its own button; this module sets it in
//! the app from the OS preference. This is the approach the tokens' own comment
//! asks for: *"following the OS means setting `data-theme` from Rust, not
//! duplicating 60 lines into a `prefers-color-scheme` block"*.
//!
//! `body` sets a colour but never a background: a page's ground comes from
//! `PageLayout`, which paints `--q-bgColor-page` itself.

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
/// palette: the failure mode is a light app, not a broken one.
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

/// The attribute `index.html` sets while the document has no page on it.
///
/// See `base.css`: it is what scopes the dark launch canvas to the empty
/// frame. A page paints its own ground and carries its own scheme, so the
/// marker has to be gone before anything is drawn at all.
pub fn stop_booting() {
    if let Some(root) = document().document_element() {
        drop(root.remove_attribute("data-booting"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::*;

    /// The launch marker must come off, and off the ROOT — the stylesheet hangs a
    /// dark canvas on it, which only the empty launch frame may have.
    #[wasm_bindgen_test]
    fn the_launch_marker_comes_off_before_anything_is_drawn() {
        let root = document().document_element().expect("a root element");
        root.set_attribute("data-booting", "").unwrap();

        stop_booting();

        assert!(
            !root.has_attribute("data-booting"),
            "the dark launch canvas must not outlive the empty frame"
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

    /// Native chrome paints from `color-scheme`, and a dark one may never reach the
    /// document root.
    ///
    /// The root's scheme decides the canvas, and the document stays light: a page
    /// paints its own ground and carries its own scheme. So every dark declaration
    /// has to name the page's surface — or `[data-booting]`, which is the frame
    /// before any page exists and is cleared by [`stop_booting`] in `main` before
    /// the mount.
    #[test]
    fn a_dark_scheme_never_reaches_the_document_root() {
        // Both global sheets the app links, because the rule this pins
        // was written in `tokens.css` and moved. `chrome.css` is excluded: it is
        // the gallery's own and never ships. A component module could hold a `:root`
        // rule in principle, but one would already be breaking the tier split.
        const SHEETS: [(&str, &str); 2] = [
            ("tokens.css", include_str!("../css/tokens.css")),
            ("base.css", include_str!("../css/base.css")),
        ];

        let declarations: Vec<(&str, &str, &str)> = SHEETS
            .iter()
            .flat_map(|(file, sheet)| {
                sheet
                    .match_indices("color-scheme:")
                    .map(move |(at, keyword)| {
                        let block = sheet[..at].rfind('{').expect("a rule around it");
                        let selector = sheet[..block].trim_end();
                        let start = selector.rfind(['}', '/']).map_or(0, |i| i + 1);
                        let value = sheet[at + keyword.len()..]
                            .split(';')
                            .next()
                            .expect("a terminated declaration")
                            .trim();
                        (*file, selector[start..].trim(), value)
                    })
            })
            .collect();
        assert!(!declarations.is_empty(), "native chrome follows nothing");

        for (file, selector, value) in &declarations {
            if *value == "dark" {
                // EVERY branch of the list, not the list as a string: a dark rule
                // is a list of two, and a substring check over the pair passes on
                // one of them while the other says `:root` — the leak this test
                // exists to catch.
                //
                // Splitting on `,` is enough because no selector here uses a
                // functional pseudo-class; `:is(...)` in one of these partials
                // would need a real parser.
                for branch in selector.split(',').map(str::trim) {
                    assert!(
                        branch.contains("[data-page]") || branch.contains("[data-booting]"),
                        "{file}: `{branch}` would darken the document's canvas"
                    );
                }
            } else {
                assert_eq!(*value, "light", "{file}: `{selector}` declares `{value}`");
            }
        }
        assert!(
            declarations.iter().any(|(_, _, value)| *value == "dark"),
            "nothing follows the dark theme"
        );
        assert!(
            declarations.iter().any(|(_, _, value)| *value == "light"),
            "the light scheme must be stated, or the UA default decides it"
        );
    }

    /// A page paints its own ground, so the reset in `base.css` paints none on
    /// `body`. Pinned on the stylesheet, where a bare rule was added once already.
    #[test]
    fn the_shared_reset_paints_no_ground() {
        const BASE: &str = include_str!("../css/base.css");
        let bare = BASE
            .split("\nbody {")
            .nth(1)
            .and_then(|rest| rest.split('}').next())
            .expect("a `body` rule");
        assert!(
            !bare.contains("background"),
            "the bare body rule must not paint a ground: {bare}"
        );
    }
}
