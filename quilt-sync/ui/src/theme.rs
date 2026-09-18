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
    remember_v2(on);
}

/// Where the flag is kept for the *next* launch, read by `index.html`'s inline
/// script before the first paint.
const V2_KEY: &str = "quiltsync.main-page-v2";

/// Record the flag for the next launch.
///
/// Last session's answer, never this one's: `/` reads the real setting every
/// time and [`set_v2`] corrects the marker either way. All this decides is which
/// palette the empty launch window wears, and being one launch behind costs a
/// reader who has just switched the flag exactly one boot in the old palette.
///
/// Every failure is ignored. A webview with no storage, a quota, a private mode
/// — each leaves the next launch opening light, which is the fallback anyway.
fn remember_v2(on: bool) {
    let Some(Ok(Some(storage))) = web_sys::window().map(|w| w.local_storage()) else {
        return;
    };
    drop(storage.set_item(V2_KEY, if on { "1" } else { "0" }));
}

/// The attribute `index.html` sets while the document has no page on it.
///
/// See `_base.scss`: it is what scopes the dark launch canvas to the empty
/// frame. A v2 reader opens v1's pages too, and those draw on a light canvas —
/// so the marker has to be gone before anything is drawn at all.
pub fn stop_booting() {
    if let Some(root) = document().document_element() {
        drop(root.remove_attribute("data-booting"));
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

    /// The launch marker must come off, and off the ROOT — the stylesheet hangs a
    /// dark canvas on it, and a canvas that stayed dark would sit under the black
    /// ink of every v1 page this reader opens.
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
    /// The root's scheme decides the canvas. [`V2_CLASS`] marks a reader who opted
    /// in, not a page that is showing, and that reader still opens v1's pages — where
    /// a dark canvas sits under ink no theme switches. So every dark declaration has
    /// to name a surface the v2 palette actually paints — or `[data-booting]`, which
    /// is the frame before any page exists and is cleared by [`stop_booting`] in
    /// `main` before the mount.
    #[test]
    fn a_dark_scheme_never_reaches_the_document_root() {
        // Both global partials the app bundle carries, because the rule this pins
        // was written in `_tokens.scss` and moved. `_chrome.scss` is excluded: it is
        // the gallery's own and never ships. A component module could hold a `:root`
        // rule in principle, but one would already be breaking the tier split.
        const SHEETS: [(&str, &str); 2] = [
            (
                "_tokens.scss",
                include_str!("../assets/css/kit/_tokens.scss"),
            ),
            ("_base.scss", include_str!("../assets/css/kit/_base.scss")),
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
                        branch.contains("[data-v2-page]")
                            || branch.contains("[data-home-frame]")
                            || branch.contains("[data-booting]"),
                        "{file}: `{branch}` would darken the canvas a v1 page draws on"
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

    /// The reset in `app.scss` reaches v1's pages, whose ink no theme switches,
    /// and `follow_os` cannot see the flag — so a themed ground on `body` would
    /// go dark under v1's black text, for a v2 reader on Settings as much as for
    /// a v1 one at home. The one ground it may paint is the frame `/` shows
    /// before it knows its page, and only behind `V2_CLASS`. Pinned on the
    /// stylesheet, where the bare rule was added once already.
    #[test]
    fn the_shared_reset_paints_a_ground_only_for_a_v2_reader_s_home_frame() {
        const BASE: &str = include_str!("../assets/css/kit/_base.scss");
        let rule = |selector: &str| {
            BASE.split(selector)
                .nth(1)
                .and_then(|rest| rest.split('}').next())
                .unwrap_or_else(|| panic!("a `{selector}` rule"))
                .to_owned()
        };
        let bare = rule("\nbody {");
        assert!(
            !bare.contains("background"),
            "the bare body rule must not paint a ground: {bare}"
        );
        assert!(
            !BASE.contains(&format!(":root.{V2_CLASS} body")),
            "no rule may put a ground on body for a v2 reader either"
        );
        let frame = rule(&format!("\n:root.{V2_CLASS} [data-home-frame] {{"));
        assert!(
            frame.contains("background: var(--q-bgColor-page)"),
            "the v2 reader's home frame must have its ground: {frame}"
        );
    }
}
