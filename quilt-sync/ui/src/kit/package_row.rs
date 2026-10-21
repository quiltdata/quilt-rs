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
//!
//! # One tooltip for the whole row
//!
//! Resting on the row, or reaching it with Tab, shows its name in full and
//! what the row does not draw: the host, what it keeps and its size. One
//! surface per row rather than a list's shared
//! [`NameTooltip`](super::NameTooltip), because it says more than the name.
//! So nothing inside the row carries a `title`: the browser's box would cover
//! the kit's.

use leptos::prelude::*;

use super::Align;
use super::Label;
use super::LabelVariant;
use super::RelativeTime;
use super::SkeletonBox;
use super::StateLabel;
use super::Tooltip;
use super::countdown::EpochMillis;
use super::icons;
use super::state_label::StateTone;
use crate::util::format_size;

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
    /// What is on disk, as an outlined badge before the state label. `None` draws no marker.
    #[prop(optional, into)]
    downloads: MaybeProp<RowDownloads>,
    /// The host of the account the package belongs to, for the tooltip. `None`
    /// leaves its line out.
    #[prop(optional, into)]
    host: MaybeProp<String>,
    /// What the package takes on this computer, for the tooltip. `None` leaves
    /// its line out.
    #[prop(optional, into)]
    size: MaybeProp<RowSize>,
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

    // Ellipsised by the stylesheet, so the tooltip's title holds all of it.
    let title = namespace.clone();
    let lines =
        Signal::derive(move || tooltip_lines(host.get().as_deref(), downloads.get(), size.get()));
    let row = move |id: String| {
        view! {
            <a class=class href=href aria-describedby=id>
                <span class=style::namespace>{namespace}</span>
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
                                    <span>"not recorded"</span>
                                }
                                    .into_any()
                            },
                            |at| view! { <RelativeTime at=at untitled=true /> }.into_any(),
                        )}
                </span>
                // Between the time and the state, so the state labels stay aligned at
                // the right edge.
                {move || {
                    downloads
                        .get()
                        .and_then(|downloads| {
                            downloads.marker().map(|words| (words, downloads.variant()))
                        })
                        .map(|(words, variant)| {
                            view! {
                                <span class=style::downloads>
                                    <Label icon=icons::download variant=variant>
                                        {words}
                                    </Label>
                                </span>
                            }
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
        .into_any()
    };

    view! { <Tooltip text=title lines=lines align=Align::Start wide=true fill=true trigger=row /> }
}

/// How much of the disk a package takes, as far as its row tells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowSize {
    /// Nothing stored for the package yet.
    NotMeasured,
    Measuring,
    Measured {
        bytes: u64,
    },
}

/// The tooltip's lines under the name, each left out when its fact is.
fn tooltip_lines(
    host: Option<&str>,
    downloads: Option<RowDownloads>,
    size: Option<RowSize>,
) -> Vec<String> {
    let host = host
        .filter(|host| !host.is_empty())
        .map(|host| format!("Host: {host}"));
    let scope = downloads.map(|downloads| match downloads {
        RowDownloads::EntirePackage { .. } => "Keeps every file in the package.".to_string(),
        RowDownloads::IndividualFiles { .. } => "Keeps only the files you picked.".to_string(),
    });
    let size = size.map(|size| match size {
        RowSize::NotMeasured => "Size not measured yet.".to_string(),
        RowSize::Measuring => "Measuring size\u{2026}".to_string(),
        RowSize::Measured { bytes } => format!("Takes {} on this computer.", format_size(bytes)),
    });
    [host, scope, size].into_iter().flatten().collect()
}

/// Which files a package keeps on disk, as far as its row tells.
///
/// A UI-side type: the main page maps the status check's counts onto it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowDownloads {
    /// The whole package, with `to_download` files not downloaded yet.
    EntirePackage { to_download: usize },
    /// Only the files someone picked: `downloaded` of the package's `total`.
    IndividualFiles { downloaded: usize, total: usize },
}

impl RowDownloads {
    /// The marker's words. A whole package with nothing left to download has
    /// none: being complete is the quiet case.
    #[must_use]
    pub fn marker(self) -> Option<String> {
        match self {
            Self::EntirePackage { to_download: 0 } => None,
            Self::EntirePackage { to_download } => Some(format!("{to_download} to download")),
            Self::IndividualFiles { downloaded, total } => {
                Some(format!("{downloaded} of {total} {}", files(total)))
            }
        }
    }
}

impl RowDownloads {
    /// Files left to download are something the reader can act on, so they are
    /// drawn louder than a count of picked files.
    #[must_use]
    pub fn variant(self) -> LabelVariant {
        match self {
            Self::EntirePackage { to_download } if to_download > 0 => LabelVariant::Accent,
            Self::EntirePackage { .. } | Self::IndividualFiles { .. } => LabelVariant::Outline,
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
        downloads.marker()
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
    fn only_files_to_download_are_filled() {
        use super::LabelVariant;
        assert_eq!(
            RowDownloads::EntirePackage { to_download: 3 }.variant(),
            LabelVariant::Accent
        );
        assert_eq!(
            RowDownloads::IndividualFiles {
                downloaded: 12,
                total: 340
            }
            .variant(),
            LabelVariant::Outline
        );
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
    use crate::test_support::{
        blur, describing_tooltip, focus_report, keyboard_focus, mount, unmount_earlier,
    };
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

    fn row_with(host: Option<&'static str>, size: Option<RowSize>) -> web_sys::Element {
        mount(move || {
            view! {
                <PackageRow
                    namespace="a-long-owner-name/a-much-longer-package-name-than-the-column"
                    href="/x"
                    state=Signal::stored("Latest".to_string())
                    tone=Signal::stored(StateTone::Success)
                    downloads=RowDownloads::IndividualFiles { downloaded: 12, total: 340 }
                    host=host.map(str::to_string)
                    size=size
                />
            }
        })
    }

    /// The tooltip's title, then each muted line.
    fn tooltip_text(el: &web_sys::Element) -> Vec<String> {
        let surface = describing_tooltip(&el.query_selector("a").unwrap().unwrap())
            .expect("the row names its tooltip");
        let lines = surface.query_selector_all("[class*=line]").unwrap();
        let first = surface.first_child().unwrap().text_content().unwrap();
        std::iter::once(first)
            .chain((0..lines.length()).map(|i| lines.item(i).unwrap().text_content().unwrap()))
            .collect()
    }

    /// The tooltip replaces every `title`: one inside the row would cover it.
    #[wasm_bindgen_test]
    fn nothing_in_the_row_carries_a_title() {
        let el = mount(|| {
            view! {
                <PackageRow
                    namespace="user/alpha"
                    href="/x"
                    changed_at=js_sys::Date::now()
                    state=Signal::stored("Latest".to_string())
                    tone=Signal::stored(StateTone::Success)
                    downloads=RowDownloads::EntirePackage { to_download: 3 }
                />
            }
        });
        assert_eq!(
            el.query_selector_all("[title]").unwrap().length(),
            0,
            "a title inside the row covers its tooltip"
        );
    }

    /// The whole namespace survives the ellipsis as the tooltip's title.
    #[wasm_bindgen_test]
    fn the_tooltip_names_the_package_then_host_scope_and_size() {
        let el = row_with(
            Some("nightly.quilttest.com"),
            Some(RowSize::Measured {
                bytes: 1_200_000_000,
            }),
        );
        assert_eq!(
            tooltip_text(&el),
            [
                "a-long-owner-name/a-much-longer-package-name-than-the-column",
                "Host: nightly.quilttest.com",
                "Keeps only the files you picked.",
                "Takes 1.2\u{a0}GB on this computer.",
            ]
        );
    }

    /// Tab reaching the row's link opens its tooltip at once.
    #[wasm_bindgen_test]
    async fn keyboard_focus_on_the_row_opens_it() {
        unmount_earlier();
        let el = row_with(None, None);
        let link = el.query_selector("a").unwrap().unwrap();
        let tip = describing_tooltip(&link).expect("the row names its tooltip");
        keyboard_focus(&link);
        leptos::task::tick().await;
        assert!(
            tip.matches(":popover-open").unwrap(),
            "keyboard focus opened nothing; {}",
            focus_report(&link)
        );
        blur(&link);
    }

    #[wasm_bindgen_test]
    fn no_host_leaves_its_line_out() {
        let el = row_with(None, Some(RowSize::Measuring));
        assert_eq!(
            tooltip_text(&el),
            [
                "a-long-owner-name/a-much-longer-package-name-than-the-column",
                "Keeps only the files you picked.",
                "Measuring size\u{2026}",
            ]
        );
    }
}

#[cfg(test)]
mod tooltip_tests {
    use super::RowDownloads;
    use super::RowSize;
    use super::tooltip_lines;

    #[test]
    fn the_scope_says_what_the_package_keeps() {
        assert_eq!(
            tooltip_lines(
                None,
                Some(RowDownloads::EntirePackage { to_download: 0 }),
                None
            ),
            ["Keeps every file in the package."]
        );
        assert_eq!(
            tooltip_lines(
                None,
                Some(RowDownloads::IndividualFiles {
                    downloaded: 1,
                    total: 2
                }),
                None
            ),
            ["Keeps only the files you picked."]
        );
    }

    #[test]
    fn the_size_line_has_a_word_for_each_state() {
        let size = |size| tooltip_lines(None, None, Some(size));
        assert_eq!(size(RowSize::NotMeasured), ["Size not measured yet."]);
        assert_eq!(size(RowSize::Measuring), ["Measuring size\u{2026}"]);
        assert_eq!(
            size(RowSize::Measured {
                bytes: 1_200_000_000
            }),
            ["Takes 1.2\u{a0}GB on this computer."]
        );
    }

    #[test]
    fn a_missing_fact_leaves_its_line_out() {
        assert_eq!(tooltip_lines(None, None, None), Vec::<String>::new());
        assert_eq!(tooltip_lines(Some(""), None, None), Vec::<String>::new());
        assert_eq!(
            tooltip_lines(Some("nightly.quilttest.com"), None, None),
            ["Host: nightly.quilttest.com"]
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
