//! One installed package, as a list row.
//!
//! # The whole row is one link
//!
//! An `<a>`, not a `div` with a click handler, and that is the difference from
//! [`FileRow`](super::FileRow) rather than a detail of it. A package is a *place
//! you go*; a file is a *thing you act on*. So this row has one destination and no
//! controls at all — which is what lets it be a real anchor, with middle-click,
//! open-in-new-tab, the status bar preview and keyboard activation for free.
//!
//! `FileRow` cannot do that: it has two destinations and three actions, and nested
//! `<a>` is invalid, so it pays for its children with a `role="button"` div.
//!
//! # Plain at rest
//!
//! No link colour, no chevron, no underline — not even on hover, where the row's
//! background tint is the whole affordance. Link colouring is a document
//! convention; this is an application, where a row is an object. And a tint on
//! every row at rest stops signalling anything: forty tinted lines read as the
//! list's typeface, not as forty invitations.

use leptos::prelude::*;

use super::RelativeTime;
use super::SkeletonBox;
use super::StateLabel;
use super::countdown::EpochMillis;
use super::state_label::StateTone;

stylance::import_crate_style!(style, "src/kit/package_row.module.scss");

#[component]
pub fn PackageRow(
    /// `owner/name`. Truncates from the **right**, unlike `FileRow`'s path: a
    /// namespace is distinguished by its start, and there is no filename at the end
    /// worth saving.
    #[prop(into)]
    namespace: String,
    /// The package page. A real `href`, so the browser owns the navigation.
    #[prop(into)]
    href: String,
    /// When the package last changed. `None` prints an explicit word rather than
    /// leaving the column blank — it should not happen, and a blank cell is
    /// indistinguishable from a cell that failed to render.
    #[prop(optional, into)]
    changed_at: MaybeProp<EpochMillis>,
    /// The state, in the page's words, and its tone. Two props rather than one
    /// struct: the caller maps a DTO status to this pair in one place, and a struct
    /// would only move the pairing without checking it.
    ///
    /// Both are `Signal`s, not plain values: a row settles by writing its own
    /// state, and with plain values the only producer of a new one is `MainPage`'s
    /// `LocalResource` — so every settle would re-run `PackageList` and rebuild
    /// every row to move one of them.
    #[prop(into)]
    state: Signal<String>,
    #[prop(into)] tone: Signal<StateTone>,
    /// Passed through to the state label: the light phase's guess, awaiting the heavy walk.
    ///
    /// The LIST shows provisional states; the QUEUE does not, which is why `QueueRow` has
    /// no such prop. Adding a row to "needs your attention" on a guess and removing it a
    /// second later is worse than an empty queue for that second — the queue is where the
    /// user decides, and a decision offered and withdrawn is not a decision.
    #[prop(optional, into)]
    provisional: MaybeProp<bool>,
    /// What is on disk, as a word beside the state label. `None` draws no marker.
    #[prop(optional, into)]
    downloads: MaybeProp<RowDownloads>,
) -> impl IntoView {
    // One computed string: Leptos rejects two `class` attributes on an element, and
    // the row dims as a whole rather than the label dimming alone — see the
    // stylesheet for the measurement behind the value.
    let class = move || {
        let mut out = String::from(style::root);
        if provisional.get().unwrap_or(false) {
            out.push(' ');
            out.push_str(style::provisional);
        }
        out
    };

    // Ellipsised by the stylesheet, so the whole value rides in `title`.
    let full_namespace = namespace.clone();
    view! {
        <a class=class href=href>
            <span class=style::namespace title=full_namespace>{namespace}</span>
            <span class=style::time>
                {move || changed_at
                    .get()
                    .map_or_else(
                        || {
                            view! {
                                // "unknown" read as a bare word with no column header to
                                // explain it — a reader could not tell what it referred
                                // to. This says which fact is missing. Still a word and
                                // not a blank: a blank cell is indistinguishable from one
                                // that failed to render.
                                <span title="This package has no recorded change time">
                                    "not recorded"
                                </span>
                            }
                                .into_any()
                        },
                        |at| view! { <RelativeTime at=at /> }.into_any(),
                    )}
            </span>
            {move || {
                downloads
                    .get()
                    .and_then(RowDownloads::marker)
                    .map(|(words, hint)| {
                        view! { <span class=style::downloads title=hint>{words}</span> }
                    })
            }}
            // Wrapped in its own closure, not read at the top level, so this patches
            // in place on every settle without `PackageRow` itself needing to be
            // called again.
            //
            // `state.get()`/`tone.get()` are read HERE, before `<StateLabel>` is built,
            // and passed in as plain values — not as `{state.get()}`/`tone.get()`
            // written directly at the call site. `#[component]` expands to a call
            // wrapped in `untrack_with_diagnostics`, so a read deferred into
            // `StateLabel`'s `Children` closure (evaluated inside that wrapper) would
            // never register as a dependency of this effect, and the row would freeze
            // on its first render.
            {move || {
                let words = state.get();
                let current_tone = tone.get();
                view! {
                    <StateLabel tone=current_tone provisional=provisional>{words}</StateLabel>
                }
            }}
        </a>
    }
}

/// Which files a package keeps on disk, as far as its row tells.
///
/// A UI-side type: nothing maps the backend onto it yet, so only the gallery draws it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowDownloads {
    /// The whole package, with `to_download` files not downloaded yet.
    EntirePackage { to_download: usize },
    /// Only the files someone picked: `downloaded` of the package's `total`.
    IndividualFiles { downloaded: usize, total: usize },
}

impl RowDownloads {
    /// The marker's words and its hover text. A whole package with nothing
    /// left to download has none: being complete is the quiet case.
    #[must_use]
    pub fn marker(self) -> Option<(String, String)> {
        match self {
            Self::EntirePackage { to_download: 0 } => None,
            Self::EntirePackage { to_download: 1 } => Some((
                "1 to download".to_string(),
                "1 file is not downloaded yet".to_string(),
            )),
            Self::EntirePackage { to_download } => Some((
                format!("{to_download} to download"),
                format!("{to_download} files are not downloaded yet"),
            )),
            Self::IndividualFiles { downloaded, total } => Some((
                format!("{downloaded} of {total} {}", files(total)),
                "Only picked files are downloaded".to_string(),
            )),
        }
    }
}

fn files(count: usize) -> &'static str {
    if count == 1 { "file" } else { "files" }
}

#[cfg(test)]
mod marker_tests {
    use super::RowDownloads;

    fn words(downloads: RowDownloads) -> Option<String> {
        downloads.marker().map(|(words, _)| words)
    }

    #[test]
    fn a_complete_whole_package_has_no_marker() {
        assert_eq!(words(RowDownloads::EntirePackage { to_download: 0 }), None);
    }

    #[test]
    fn files_left_to_download_are_counted() {
        assert_eq!(
            words(RowDownloads::EntirePackage { to_download: 1 }).as_deref(),
            Some("1 to download")
        );
        assert_eq!(
            words(RowDownloads::EntirePackage { to_download: 3 }).as_deref(),
            Some("3 to download")
        );
    }

    #[test]
    fn the_hint_agrees_with_the_count() {
        let hint = |to_download| {
            RowDownloads::EntirePackage { to_download }
                .marker()
                .unwrap()
                .1
        };
        assert_eq!(hint(1), "1 file is not downloaded yet");
        assert_eq!(hint(3), "3 files are not downloaded yet");
    }

    #[test]
    fn picked_files_count_against_the_package() {
        let picked = |downloaded, total| words(RowDownloads::IndividualFiles { downloaded, total });
        assert_eq!(picked(12, 340).as_deref(), Some("12 of 340 files"));
        assert_eq!(picked(0, 1).as_deref(), Some("0 of 1 file"));
        assert_eq!(picked(1, 1).as_deref(), Some("1 of 1 file"));
    }
}

/// The same row with its content unknown.
///
/// It reuses `PackageRow`'s own `.root` class rather than restating the padding, which is
/// the only way the two are guaranteed to be the same height — and equal height is the
/// whole point. A skeleton a few pixels shorter than the row it stands in for makes the
/// list jump at the exact moment the user starts reading it.
///
/// `.skeleton` switches off the affordance: no pointer cursor and no hover tint, because
/// nothing here responds to a click.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::mount;
    use wasm_bindgen_test::*;

    /// The row's own root, not a descendant — which is the point of these two.
    fn root_class(el: &web_sys::Element) -> String {
        el.query_selector("a")
            .unwrap()
            .expect("the row is an anchor")
            .get_attribute("class")
            .unwrap_or_default()
    }

    fn row(provisional: bool) -> web_sys::Element {
        mount(move || {
            view! {
                <PackageRow
                    namespace="user/alpha"
                    href="/x"
                    state=Signal::stored("Latest".to_string())
                    tone=Signal::stored(StateTone::Success)
                    provisional=provisional
                />
            }
        })
    }

    #[wasm_bindgen_test]
    fn a_provisional_row_dims_itself_and_not_only_its_label() {
        // `StateLabel` already dashes its own edge, and every existing assertion
        // on `[class*=provisional]` is satisfied by that label alone — so nothing
        // pinned the ROW's dim, which is the channel a reader actually notices.
        // These two look at the root element specifically.
        //
        // The pair is the unit: either alone would pass against a class applied
        // unconditionally or never.
        assert!(
            root_class(&row(true)).contains("provisional"),
            "the row carries the dim"
        );
        assert!(
            !root_class(&row(false)).contains("provisional"),
            "a settled row carries nothing"
        );
    }

    fn downloads_text(downloads: Option<RowDownloads>) -> Option<String> {
        let el = mount(move || {
            view! {
                <PackageRow
                    namespace="user/alpha"
                    href="/x"
                    state=Signal::stored("Latest".to_string())
                    tone=Signal::stored(StateTone::Success)
                    downloads=downloads
                />
            }
        });
        el.query_selector("[class*=downloads]")
            .unwrap()
            .map(|span| span.text_content().unwrap_or_default())
    }

    #[wasm_bindgen_test]
    fn the_marker_draws_only_when_it_has_words() {
        assert_eq!(downloads_text(None), None, "no scope, no marker");
        assert_eq!(
            downloads_text(Some(RowDownloads::EntirePackage { to_download: 0 })),
            None,
            "a complete whole package stays quiet"
        );
        assert_eq!(
            downloads_text(Some(RowDownloads::IndividualFiles {
                downloaded: 12,
                total: 340
            }))
            .as_deref(),
            Some("12 of 340 files")
        );
    }

    /// A namespace longer than its column: `title` holds all of it.
    #[wasm_bindgen_test]
    fn the_truncating_namespace_carries_its_whole_value() {
        let el = mount(|| {
            view! {
                <PackageRow
                    namespace="a-long-owner-name/a-much-longer-package-name-than-the-column"
                    href="/x"
                    state=Signal::stored("Latest".to_string())
                    tone=Signal::stored(StateTone::Success)
                />
            }
        });
        let span = el
            .query_selector("[class*=namespace]")
            .unwrap()
            .expect("the namespace");
        assert_eq!(
            span.get_attribute("title").as_deref(),
            Some("a-long-owner-name/a-much-longer-package-name-than-the-column"),
            "the whole namespace has to survive the ellipsis"
        );
    }
}

#[component]
pub fn PackageRowSkeleton() -> impl IntoView {
    view! {
        <div class=format!("{} {}", style::root, style::skeleton)>
            <span class=style::namespace>
                <SkeletonBox width="38%" />
            </span>
            <span class=style::time>
                <SkeletonBox width="100%" />
            </span>
            // 88px is roughly `Latest` in a state label. A percentage would be wrong here:
            // the label's width is set by its words, not by the row.
            <SkeletonBox width="88px" height="22px" />
        </div>
    }
}
