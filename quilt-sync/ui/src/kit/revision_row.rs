//! One revision: what it says, when, and whether anyone else can see it.
//!
//! # No label, no location
//!
//! A caller that wants `YOURS` above it wraps it in a
//! [`PaneSection`](super::PaneSection) — the label belongs to the block, which
//! is why the same component serves the current revision, the two sides of a
//! resolve, and every row of the revisions overlay, where there are no labels at
//! all.
//!
//! The bucket is likewise absent. It is a fact about the *package*, so inside a
//! list of revisions it would repeat itself identically on every row.
//!
//! # No hash
//!
//! A revision is named by its message and its time. The identity the page
//! exposes is deliberately human, and a hash is the thing users were reading
//! when they could not tell two revisions apart. A `href` may carry one, because
//! an address is not a name.
//!
//! # Published is a glyph, not a word
//!
//! A list of revisions is a column where most rows say the same thing, and a
//! word repeated four times down a 280px pane is four times the width for one
//! bit. So the fact is a cloud, and the row that has not left this machine wears
//! the slashed one — a pair, rather than a glyph and its absence, because
//! nothing distinguishes an absent glyph from a component that was not told.
//! The word itself is still there for anyone not reading pixels.
//!
//! # Two facts, two props
//!
//! `published` says whether the revision reached the platform; `catalog` says
//! where to read it and what opens it. They are separate because a published
//! revision in a bucket with no catalog host has nowhere to point — it earns the
//! cloud and no link. The reverse never holds: a revision this copy has not
//! sent has no address.
//!
//! # The link is an icon, not the message
//!
//! The message is always text. In a list of revisions most rows are published,
//! so a message that linked would make the column one long run of accent — and
//! the cloud at the head of the row has already said which ones are published.
//! What the colour would have added is only "this goes somewhere", and a glyph
//! saying *open elsewhere* at the row's end says that more plainly, as an action
//! rather than as a style. A row with nowhere to point draws nothing there and
//! keeps no room for it.
//!
//! # The address and the opener arrive together
//!
//! [`CatalogLink`] carries both, because a link this app *follows* would replace
//! the running application: the webview has no chrome to come back from. Every
//! other catalog link in the app hands the URL to `open_in_web_browser` from a
//! click handler, and the kit cannot do that itself — it holds no commands. So
//! the icon is a real anchor, the navigation is cancelled, and what the caller
//! gave it is called. A caller cannot supply one without the other, which is the
//! only way the rule survives the next call site.

use leptos::prelude::*;

use super::RelativeTime;
use super::countdown::EpochMillis;
use super::icons;

stylance::import_crate_style!(style, "src/kit/revision_row.module.scss");

/// `MouseEvent.button` for the middle one. `auxclick` carries the right button
/// under the same event, and that gesture is asking for the context menu.
const MIDDLE_BUTTON: i16 = 1;

/// The catalog link's name. The same words for the ear and the pointer.
const OPEN_LABEL: &str = "Open in catalog";

/// Where a revision is read, and what opens it.
///
/// One value and not two props, so a call site cannot draw a link it has no way
/// to open. `open` is handed the address at click time — in the app it goes to
/// `open_in_web_browser`, which is how every other catalog link in this codebase
/// reaches a browser.
#[derive(Clone)]
pub struct CatalogLink {
    href: String,
    open: Callback<String>,
}

impl CatalogLink {
    #[must_use]
    pub fn new(href: impl Into<String>, open: Callback<String>) -> Self {
        Self {
            href: href.into(),
            open,
        }
    }
}

#[component]
pub fn RevisionRow(
    /// The commit message. Quoted by the component rather than by the caller, so
    /// four call sites cannot quote it four ways.
    #[prop(into)]
    message: String,
    /// When this copy obtained it.
    at: EpochMillis,
    /// Whether it reached the platform. `None` draws no glyph at all, for the
    /// callers that are not showing a list — the current revision under the
    /// pane's own heading, and the two sides of a resolve, where the header and
    /// the section labels have already said it.
    #[prop(optional)]
    published: Option<bool>,
    /// Where to read it, and what opens it. Draws the open icon at the row's
    /// end; the message and the time stay text, and the time could not be the
    /// link in any case — it is this copy's fact and not the platform's.
    ///
    /// `optional_no_strip` rather than `optional`: callers hold an `Option`
    /// already, because `util::catalog_url` answers `None` for a bucket with no
    /// catalog host.
    #[prop(optional_no_strip)]
    catalog: Option<CatalogLink>,
) -> impl IntoView {
    // An empty message is reachable — nothing stops a publish without one — and
    // it must not render as a pair of bare quotes.
    let quoted = {
        let trimmed = message.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(format!("\u{201c}{trimmed}\u{201d}"))
        }
    };
    // The whole value rides in `title`, since the visible one is ellipsised. An
    // empty message loses the tooltip with it: there is nothing to put in one.
    let body = match quoted {
        Some(text) => view! { <span class=style::message title=message>{text}</span> }.into_any(),
        None => view! { <span class=style::empty>"No message"</span> }.into_any(),
    };

    let link = catalog.map(|CatalogLink { href, open }| {
        let followed = href.clone();
        let middled = href.clone();
        view! {
            // A real anchor, so the address is there to copy and the pointer
            // says where it goes — and then the navigation is cancelled,
            // because following it would replace the application. `auxclick`
            // as well as `click`: a middle button does not raise the latter,
            // and a new webview window is the same loss by another door.
            //
            // `auxclick` fires for **every** non-primary button, so the
            // middle one is checked for by number. Right-clicking asks for
            // the context menu and nothing else, and the menu is where the
            // address gets copied.
            //
            // The glyph has no words, so the anchor carries them: `aria-label`
            // for a screen reader, `title` for the pointer.
            <a
                class=style::open
                href=href
                aria-label=OPEN_LABEL
                title=OPEN_LABEL
                on:click=move |ev| {
                    ev.prevent_default();
                    open.run(followed.clone());
                }
                on:auxclick=move |ev| {
                    if ev.button() == MIDDLE_BUTTON {
                        ev.prevent_default();
                        open.run(middled.clone());
                    }
                }
            >
                {icons::link_external()}
            </a>
        }
    });

    let glyph = published.map(|published| {
        let (icon, words) = if published {
            (icons::cloud(), "Published")
        } else {
            (icons::cloud_offline(), "Not published")
        };
        view! {
            // `title` for the pointer, the word itself for everyone else: a glyph
            // carrying a fact nothing else on the row states cannot be the only
            // place that fact exists.
            <span class=style::glyph title=words>
                {icon}
                <span data-sr-only>{words}</span>
            </span>
        }
    });

    view! {
        <div class=style::root>
            {glyph}
            <div class=style::body>
                {body}
                <span class=style::when>
                    <RelativeTime at=at />
                </span>
            </div>
            {link}
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{element_saying, mount};
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::*;

    const HREF: &str = "https://test.quilt.dev/b/test/packages/team/dataset/tree/published-hash";

    fn published(open: Callback<String>) -> web_sys::Element {
        mount(move || {
            view! {
                <RevisionRow
                    message="Sent"
                    at=1_758_500_000_000.0
                    published=true
                    catalog=Some(CatalogLink::new(HREF, open))
                />
            }
        })
    }

    #[wasm_bindgen_test]
    fn a_catalog_row_links_from_a_named_icon_and_not_its_message() {
        let el = published(Callback::new(|_: String| ()));

        let links = el.query_selector_all("a").unwrap();
        assert_eq!(
            links.length(),
            1,
            "one link; markup was {}",
            el.inner_html()
        );
        let link = el
            .query_selector(&format!("a[href=\"{HREF}\"]"))
            .unwrap()
            .expect("the link goes to the revision's address");
        assert_eq!(
            link.get_attribute("aria-label").as_deref(),
            Some("Open in catalog")
        );
        assert_eq!(
            link.get_attribute("title").as_deref(),
            Some("Open in catalog")
        );
        assert_eq!(
            link.text_content().unwrap_or_default().trim(),
            "",
            "the link is the glyph; the message is not inside it"
        );

        let message = element_saying(&el, "\u{201c}Sent\u{201d}");
        assert!(
            message.closest("a").unwrap().is_none(),
            "the message is text; markup was {}",
            el.inner_html()
        );
    }

    #[wasm_bindgen_test]
    fn a_row_without_a_catalog_draws_no_link() {
        let el = mount(|| {
            view! {
                <RevisionRow message="Kept here" at=1_758_400_000_000.0 published=false />
            }
        });
        assert!(
            el.query_selector("a").unwrap().is_none(),
            "markup was {}",
            el.inner_html()
        );
        element_saying(&el, "Not published");
    }

    #[wasm_bindgen_test]
    fn an_empty_message_is_a_placeholder_beside_the_link() {
        let el = mount(|| {
            view! {
                <RevisionRow
                    message=""
                    at=1_758_500_000_000.0
                    published=true
                    catalog=Some(CatalogLink::new(HREF, Callback::new(|_: String| ())))
                />
            }
        });
        let placeholder = element_saying(&el, "No message");
        assert!(placeholder.closest("a").unwrap().is_none());
        assert!(
            el.query_selector("[aria-label='Open in catalog']")
                .unwrap()
                .is_some(),
            "an unnamed published revision is still reachable"
        );
    }

    #[wasm_bindgen_test]
    async fn the_icon_opens_through_the_callback_without_navigating() {
        let opened: RwSignal<Option<String>> = RwSignal::new(None);
        let el = published(Callback::new(move |url: String| opened.set(Some(url))));

        let before = web_sys::window().unwrap().location().href().unwrap();
        el.query_selector("[aria-label='Open in catalog']")
            .unwrap()
            .expect("the catalog link")
            .unchecked_into::<web_sys::HtmlElement>()
            .click();
        leptos::task::tick().await;

        assert_eq!(opened.get_untracked().as_deref(), Some(HREF));
        assert_eq!(
            web_sys::window().unwrap().location().href().unwrap(),
            before,
            "the test page did not navigate"
        );
    }
}
