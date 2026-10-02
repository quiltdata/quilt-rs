//! The installed-package page's context pane, in the states it can hold.
//!
//! # What this scene is for
//!
//! The pane is the page's *other* half, and every one of its parts already has a
//! Core cell. What no Core cell can show is the three of them at 280px with a
//! popover hanging off one — whether the surface fits the pane it belongs to,
//! whether two blocks labelled by two different components look like siblings,
//! and whether the conditional download action reads as part of the choice above
//! it or as a control that wandered in.
//!
//! # The pane is a `Card` inside an `<aside>`
//!
//! `Card` is a `<section>` with an `<h2>`, and this pane has no visible title and
//! is not a section of the page's prose — so the landmark and the name live on
//! the `<aside>`, and the `Card` inside it is a box drawing a border. One
//! element is spent to say what the pane is; the alternative was a level prop on
//! `Card`, which would have put the pane's structure into every other caller's
//! signature.
//!
//! # Two blocks, two components, one label style
//!
//! `Revision` is a [`PaneSection`](crate::kit::PaneSection) label and `Keeping`
//! is a [`ChoiceGroup`](crate::kit::ChoiceGroup) label, because a `PaneSection`
//! titled `Keeping` around a `ChoiceGroup` titled `Keeping` says it twice. They
//! are drawn from the same rule and match to the pixel — but only one of them is
//! a heading, so heading navigation finds `Revision` and walks straight past
//! `Keeping`. That is the pane's shape reporting a kit gap, not a scene's
//! mistake.

use std::collections::BTreeSet;
use std::sync::Arc;

use leptos::context::Provider;
use leptos::prelude::*;
use quilt_uri::Namespace;

use crate::Cell;
use crate::Scene;
use crate::commands::CurrentRevisionData;
use crate::commands::ResolveData;
use crate::commands::RevisionHistoryData;
use crate::commands::RevisionHistoryRow;
use crate::gallery::file_pane::MARKED;
use crate::kit::Align;
use crate::kit::AnchoredOverlay;
use crate::kit::Button;
use crate::kit::Card;
use crate::kit::CatalogLink;
use crate::kit::Choice;
use crate::kit::ChoiceGroup;
use crate::kit::DiffersId;
use crate::kit::LoadFailure;
use crate::kit::PaneSection;
use crate::kit::RevisionRow;
use crate::kit::SkeletonBox;
use quilt_sync_ui::util::format_size;
use quilt_sync_ui::util::thousands;

/// The design's fixed pane width. Written here rather than taken from a token
/// because it is this page's bet, not the system's: the file list grows and the
/// pane does not.
const PANE: &str = "width:280px";

const NAMESPACE: &str = "user/plate-07";
/// A fact about the package, so it sits in the section and never on a row —
/// inside the revisions surface it would repeat identically all the way down.
const BUCKET: &str = "s3://quilt-lab-plates";

const TOTAL: usize = 56;

const HOUR: f64 = 3_600_000.0;
const DAY: f64 = 24.0 * HOUR;

fn ago(ms: f64) -> f64 {
    js_sys::Date::now() - ms
}

/// The revisions this copy holds, newest first — which is what `list_revisions`
/// returns and what the trigger counts. Four, because a list of one or two
/// never shows whether the surface scrolls.
///
/// The newest has not been sent: that is the ordinary shape of this list, since
/// the revision somebody is working on is the one they have not published, and a
/// list where every row is identical on the one axis the glyph draws would prove
/// nothing about the glyph.
fn revisions() -> Vec<(&'static str, f64, Option<CatalogLink>)> {
    held()
        .into_iter()
        .map(|(message, at, hash)| (message, at, hash.map(catalog)))
        .collect()
}

/// [`revisions`] by the hash its catalog address carries, `None` for the one
/// this copy has not sent — the one list both the hand-built cells and the live
/// pane's answer draw from.
fn held() -> Vec<(&'static str, f64, Option<&'static str>)> {
    vec![
        ("Add Caihong folder-upload note", ago(2.0 * HOUR), None),
        (
            "Re-run plate 7 with the corrected layout",
            ago(3.0 * DAY),
            Some("c41d8f"),
        ),
        ("", ago(9.0 * DAY), Some("9a2b71")),
        ("Initial upload", ago(26.0 * DAY), Some("06e3ad")),
    ]
}

/// The live pane's answer: the same four revisions, as the backend sends them.
/// Published is having a catalog address, as it is in every row above. Nothing
/// is measured, so the pane offers no removal: that is the old revisions scene.
fn history(
    _namespace: String,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<RevisionHistoryData, String>>>> {
    let rows = held()
        .into_iter()
        .map(|(message, at, hash)| RevisionHistoryRow {
            hash: hash.unwrap_or("local").to_string(),
            message: Some(message.to_string()),
            obtained_at: at,
            published: hash.is_some(),
            catalog_url: hash.map(catalog_href),
            kept: Vec::new(),
            frees: None,
        })
        .collect();
    Box::pin(async move {
        Ok(RevisionHistoryData {
            rows,
            removable_frees: None,
        })
    })
}

/// Where a published revision is read. The hash is banned from the page's words
/// and belongs in an address, which is not a name.
///
/// The opener does nothing here. In the app it is `open_in_web_browser`; a
/// gallery has no Tauri host to hand an address to, and letting the anchor
/// follow itself would take the gallery with it.
fn catalog(revision: &str) -> CatalogLink {
    CatalogLink::new(catalog_href(revision), Callback::new(|_url: String| ()))
}

fn catalog_href(revision: &str) -> String {
    format!("https://quilt-lab.example/b/quilt-lab-plates/packages/{NAMESPACE}/tree/{revision}/")
}

fn scopes() -> Vec<Choice> {
    vec![
        Choice::new("pick", "Files I pick"),
        Choice::new("all", "The whole package"),
    ]
}

/// The trigger owes `aria-expanded` and `aria-controls`; the overlay hands the id
/// over so it can point at what it opens.
fn trigger(open: RwSignal<bool>) -> impl FnOnce(String) -> AnyView {
    move |surface_id| {
        view! {
            <Button
                on_click=move |_| open.update(|o| *o = !*o)
                aria_expanded=open
                aria_controls=surface_id
            >
                "Revisions you have (4)"
            </Button>
        }
        .into_any()
    }
}

/// The surface, filled.
fn revision_list() -> AnyView {
    view! {
        <PaneSection>
            {revisions()
                .into_iter()
                .map(|(message, at, catalog)| {
                    view! {
                        <RevisionRow
                            message=message
                            at=at
                            published=catalog.is_some()
                            catalog=catalog
                        />
                    }
                })
                .collect_view()}
        </PaneSection>
    }
    .into_any()
}

/// The surface before `list_revisions` answers. Four bars for four rows, at the
/// widths messages actually have, so the surface does not resize under the
/// reader's cursor when it fills.
fn revision_skeleton() -> AnyView {
    view! {
        <PaneSection>
            <SkeletonBox width="200px" />
            <SkeletonBox width="228px" />
            <SkeletonBox width="164px" />
            <SkeletonBox width="188px" />
        </PaneSection>
    }
    .into_any()
}

/// What the page read knows about the size.
#[derive(Clone, Copy)]
enum Size {
    /// A row's size could not be read.
    Unread,
    /// Bytes in the revision, and in the files of it that are here.
    Known { total: u64, here: u64 },
}

/// What `Keeping` reports: the revision's files, how many are not here, and
/// their bytes.
#[derive(Clone, Copy)]
struct Held {
    files: usize,
    pending: usize,
    size: Size,
}

const MB: u64 = 1_000_000;

/// The standalone cells' package, two files short or complete: 56 files,
/// 3.4 MB, of which 1.9 MB is here when two are not. Not the page's: beside the
/// file pane the region reads [`on_page`] instead.
const fn plate(pending: usize) -> Held {
    Held {
        files: TOTAL,
        pending,
        size: Size::Known {
            total: 34 * MB / 10,
            here: if pending == 0 {
                34 * MB / 10
            } else {
                19 * MB / 10
            },
        },
    }
}

/// The file pane's own package, as the region says it on the page: the two
/// panes are on screen together there, so the sizes come from the same rows.
fn on_page() -> Held {
    let kept = crate::gallery::file_pane::kept();
    Held {
        files: kept.files,
        pending: kept.pending,
        size: Size::Known {
            total: kept.total,
            here: kept.here,
        },
    }
}

/// A revision with no files at all.
const EMPTY: Held = Held {
    files: 0,
    pending: 0,
    size: Size::Known { total: 0, here: 0 },
};

/// 140,000 files, 12,400 of them here.
const HUGE: Held = Held {
    files: 140_000,
    pending: 127_600,
    size: Size::Known {
        total: 1_200_000 * MB,
        here: 86_300 * MB,
    },
};

/// Two files short, and a row's size could not be read.
const UNREAD: Held = Held {
    size: Size::Unread,
    ..plate(2)
};

/// The count, with the size joined to it: the count already answers *how much
/// of this package is here*, and the size is the same question in bytes. A
/// size that cannot be read says nothing, not a dash: the count is still true.
/// An empty revision says so instead of "All files are downloaded", which is
/// true of nothing.
fn counted(held: Held) -> String {
    let present = held.files.saturating_sub(held.pending);
    match held.size {
        Size::Known { .. } if held.files == 0 => "This revision has no files".to_string(),
        Size::Known { total, .. } if held.pending == 0 => {
            format!("All files are downloaded · {}", format_size(total))
        }
        Size::Known { total, .. } if present == 0 => {
            format!("No files are downloaded · {}", format_size(total))
        }
        Size::Known { total, here } => format!(
            "{} of {} files · {} of {} downloaded",
            thousands(present),
            thousands(held.files),
            format_size(here),
            format_size(total),
        ),
        Size::Unread if held.pending == 0 => "All files are downloaded".to_string(),
        Size::Unread => format!(
            "{} of {} downloaded",
            thousands(present),
            thousands(held.files)
        ),
    }
}

/// What the choice means, in the present tense.
///
/// Two clauses from v1's own band, kept apart on purpose: the count reports the
/// present, and the standing line promises something about files that do not
/// exist yet. Only whole-package scope may make that promise — under
/// individual-file scope the next revision can add a file and falsify it.
fn consequence(scope: RwSignal<String>, held: Held) -> Signal<String> {
    Signal::derive(move || {
        let counted = counted(held);
        if scope.get() == "all" {
            format!("{counted} — files added later are downloaded too.")
        } else {
            format!("{counted}.")
        }
    })
}

/// The backlog action's words, `Download 2 files · 1.5 MB`: the bytes are the
/// missing files', total less downloaded, so a size that cannot be read leaves
/// the count alone. The live cell's section takes them from the page read's size.
fn download_words(held: Held) -> String {
    let files = if held.pending == 1 {
        "Download 1 file".to_string()
    } else {
        format!("Download {} files", thousands(held.pending))
    };
    match held.size {
        Size::Known { total, here } => {
            format!("{files} · {}", format_size(total.saturating_sub(here)))
        }
        Size::Unread => files,
    }
}

/// The backlog action. Conditional and counted, which is what lets it exist at
/// all: a standing `Download all files` with no extent was removed on purpose,
/// and this appears only where the scope change would otherwise strand files
/// already listed.
fn download(scope: RwSignal<String>, held: Held) -> AnyView {
    view! {
        {move || {
            (scope.get() == "all" && held.pending > 0)
                .then(|| view! { <Button on_click=|_| ()>{download_words(held)}</Button> })
        }}
    }
    .into_any()
}

/// The pane in its default mode.
///
/// The two blocks are direct children of the `Card`, which is what draws the rule
/// between them; `PaneSection` supplies the air on either side of it. A flush
/// hairline is right for rows, which carry their own padding — against a section
/// that has none it lands on the last control and reads as its underline.
fn pane(
    open: RwSignal<bool>,
    body: AnyView,
    scope: RwSignal<String>,
    held: Held,
    on_page: bool,
) -> AnyView {
    let sections = view! {
        <PaneSection label="Revision">
            <RevisionRow message="Add Caihong folder-upload note" at=ago(2.0 * HOUR) />
            <span style="color:var(--q-fgColor-muted); font-size:var(--q-text-body)">
                {BUCKET}
            </span>
            <AnchoredOverlay
                trigger=trigger(open)
                open=open
                aria_label="Revisions you have"
                align=Align::End
            >
                {body}
            </AnchoredOverlay>
        </PaneSection>
        <PaneSection>
            <ChoiceGroup
                label="Keeping"
                caption=consequence(scope, held)
                options=scopes()
                selected=scope
            />
            {download(scope, held)}
        </PaneSection>
    };

    // On the page the width is a class, not an inline style: stacked under 800px
    // the pane stops sharing a row with anything and takes the column, and a
    // container query cannot outrank an attribute.
    view! {
        <aside
            aria-label="About this package"
            class=on_page.then_some("g-ip-contextpane")
            style=(!on_page).then_some(PANE)
        >
            <Card>{sections}</Card>
        </aside>
    }
    .into_any()
}

/// The pane where the page puts it: last in a row, with the file list holding
/// everything to its left.
///
/// The three overlay cells are drawn inside this and the others are not, because
/// the surface is the one part of the pane whose behaviour is about the pane's
/// *position*. A 280px pane floating alone in a cell would let a popover open
/// rightwards into empty gallery, which is the one direction the real page does
/// not have.
fn in_page(pane: AnyView) -> AnyView {
    view! {
        <div style="display:flex; gap:var(--q-space-3); width:992px; max-width:100%">
            <div style="flex:1; min-width:0; display:flex; align-items:center; \
                        justify-content:center; min-height:180px; \
                        border:1px dashed var(--q-borderColor-muted); \
                        border-radius:var(--q-radius); color:var(--q-fgColor-muted); \
                        font-size:var(--q-text-body)">
                "the file list"
            </div>
            {pane}
        </div>
    }
    .into_any()
}

/// The pane under `?resolve=1`: the app's own `ResolvePane`, fed the fixture
/// and commands that answer at once.
///
/// The two buttons are deliberately not mirrors. One publishes what is already
/// here; the other replaces local files and is the one that opens the
/// confirmation, whose verb is the kit's `ButtonVariant::Danger`
/// (`kit/confirm_dialog.rs`). What the pane avoids is a Danger *pane* button:
/// before the confirmation nothing has been risked, so the weight is carried by
/// the arrangement and by the dialog's own copy.
///
/// The exit is **inert in a gallery**. Leaving resolve is a navigation — the
/// real page drops `?resolve=1` and the router redraws — and there is no router
/// here to answer it, so every caller points the `BackLink` at the cell the
/// pane is already inside: an anchor to anything further away scrolls, and a
/// link that says it does nothing should not move the page.
fn resolve(exit: &str, resolve: ResolveData) -> AnyView {
    let differing: BTreeSet<String> = MARKED.iter().map(|&key| key.to_string()).collect();
    view! {
        <crate::pages::ResolvePane
            namespace=Namespace::try_from(NAMESPACE).expect("a scene namespace")
            uri=None
            revision=CurrentRevisionData {
                hash: "e7f2a9".to_string(),
                message: Some("Re-run plate 7 with the corrected layout".to_string()),
                obtained_at: ago(0.4 * HOUR),
            }
            resolve=resolve
            marks=Signal::stored(Some(Arc::new(differing)))
            back_href=exit.to_string()
            w=crate::pages::Wiring::new()
            commands=crate::pages::ResolveCommands {
                certify: |_, _| Box::pin(async { Ok(String::new()) }),
                reset: |_, _| Box::pin(async { Ok(String::new()) }),
            }
        />
    }
    .into_any()
}

fn compared() -> ResolveData {
    ResolveData::Compared {
        published_message: Some("Add Caihong folder-upload note".to_string()),
        differing: MARKED.iter().map(|&key| key.to_string()).collect(),
        unpublished: 2,
        uncommitted: 1,
    }
}

fn refused() -> ResolveData {
    ResolveData::Refused {
        reason: "The bucket did not answer.".to_string(),
    }
}

/// What the cells are for, and what looking at them settled.
const NOTE: &str = "280px holding two blocks: what the page says about the package rather \
    than about its files. \
    \
    Keeping's count carries the package's size: the sum of the current revision's \
    manifest rows, and of those whose files are here. It is the logical size, not disk \
    usage: on APFS the object store shares blocks with the working files. The pane's words \
    say downloaded, about files; only the revisions surface's frees is about this disk. A \
    size that cannot be read leaves the count alone. \
    \
    Flip a radio in a hand-built cell and the caption and the download action \
    answer together; the live cell's answer only once a stored choice is re-read, \
    which the gallery never does. \
    Click a trigger: the surface hangs leftwards over the file list, which is \
    the only direction the page has for it. In the list a published revision \
    wears a cloud and ends in an icon that opens the catalog; the unsent one \
    wears the slashed cloud and no icon. \
    \
    In resolve mode both choices fit on one line at 280px, each with the \
    sentence that says what it does beneath it. They are a block of their own \
    under the comparison, ruled off from Published, and each hint sits close \
    under its button and well clear of the next.";

/// The region itself, for the whole-page scene.
///
/// The revisions surface opens leftwards over the file list, which is the one
/// part of this pane whose behaviour is about where the pane sits — so on the
/// page it is drawn by the real arrangement rather than by a cell imitating it.
#[component]
#[allow(
    clippy::needless_pass_by_value,
    reason = "a component's props are owned; `resolve` borrows the href from there"
)]
pub fn ContextPaneRegion(
    /// `?resolve=1`: the pane swaps to the choice between two revisions.
    #[prop(optional)]
    resolving: bool,
    /// The standing scope, shared with whatever else on the page reads it.
    scope: RwSignal<String>,
    /// Where resolve mode's exit points. The page's own anchor, so that a link
    /// with no router behind it does not scroll somebody somewhere else.
    #[prop(into, optional)]
    exit: String,
) -> impl IntoView {
    let open = RwSignal::new(false);
    if resolving {
        resolve(&exit, compared())
    } else {
        pane(open, revision_list(), scope, on_page(), true)
    }
}

#[component]
#[allow(clippy::too_many_lines, reason = "one cell per state, read as a list")]
pub fn ContextPaneScene() -> impl IntoView {
    // One open signal per pane: a popover of `auto` type closes any other, so
    // sharing one between two cells would make the second trigger reopen the
    // first cell's surface.
    let resting = RwSignal::new(false);
    let outstanding = RwSignal::new(false);
    let settled = RwSignal::new(false);
    let listed = RwSignal::new(false);
    let waiting_surface = RwSignal::new(false);
    let failed = RwSignal::new(false);
    let emptied = RwSignal::new(false);
    let huge = RwSignal::new(false);
    let unread = RwSignal::new(false);

    let pick = RwSignal::new("pick".to_string());
    let whole = RwSignal::new("all".to_string());
    let complete = RwSignal::new("all".to_string());
    let listing = RwSignal::new("pick".to_string());
    let waiting = RwSignal::new("pick".to_string());
    let broken = RwSignal::new("pick".to_string());
    let empty_scope = RwSignal::new("pick".to_string());
    let huge_scope = RwSignal::new("pick".to_string());
    let unread_scope = RwSignal::new("all".to_string());

    view! {
        <Scene
            title="The context pane"
            note=NOTE
        >
            <Cell
                wide=true
                label="live — current revision, bucket, history and keeping; the page's own \
                       section, with the size PackageContextData carries and a file deleted here"
            >
                <crate::pages::CurrentRevisionPane
                    data=crate::commands::PackageContextData {
                        revision: crate::commands::CurrentRevisionData {
                            hash: "b5e013".to_string(),
                            message: Some("Add Caihong folder-upload note".to_string()),
                            obtained_at: ago(2.0 * HOUR),
                        },
                        bucket: Some("quilt-lab-plates".to_string()),
                        revision_count: 4,
                        keeping: crate::commands::KeepingData {
                            scope: crate::commands::KeepingScope::EntirePackage,
                            total: TOTAL,
                            remote_only: vec!["plate/b.csv".to_string(), "plate/c.csv".to_string()],
                            deleted_here: 1,
                        },
                        size: Some(crate::commands::PackageSize {
                            total: 34 * MB / 10,
                            downloaded: 19 * MB / 10,
                        }),
                        resolve: None,
                    }
                    namespace=NAMESPACE
                    fetch=history
                    open_catalog=Callback::new(|_: String| ())
                    w=crate::pages::Wiring::new()
                    commands=crate::pages::KeepingCommands {
                        store: |_, _| Box::pin(async { Ok(()) }),
                        download: |_, _| Box::pin(async { Ok(()) }),
                    }
                />
            </Cell>
            <Cell wide=true label="at rest — files I pick, two outstanding">
                {pane(resting, revision_list(), pick, plate(2), false)}
            </Cell>
            <Cell
                wide=true
                label="the whole package, two files outstanding — the button says the bytes left, \
                       total less downloaded"
            >
                {pane(outstanding, revision_list(), whole, plate(2), false)}
            </Cell>
            <Cell wide=true label="the whole package, nothing outstanding — no action">
                {pane(settled, revision_list(), complete, plate(0), false)}
            </Cell>
            <Cell wide=true label="an empty revision — no files, and the count says so">
                {pane(emptied, revision_list(), empty_scope, EMPTY, false)}
            </Cell>
            <Cell wide=true label="a huge package — 1.2 TB in 140,000 files; the counts grouped">
                {pane(huge, revision_list(), huge_scope, HUGE, false)}
            </Cell>
            <Cell
                wide=true
                label="a size that cannot be read — the count alone, and the button counts files"
            >
                {pane(unread, revision_list(), unread_scope, UNREAD, false)}
            </Cell>
            <Cell full=true label="click the trigger — the revisions this copy holds">
                {in_page(pane(listed, revision_list(), listing, plate(2), false))}
            </Cell>
            <Cell full=true label="click it — the call has not answered yet">
                {in_page(pane(waiting_surface, revision_skeleton(), waiting, plate(2), false))}
            </Cell>
            <Cell full=true label="click it — the call failed">
                {in_page(
                    pane(
                        failed,
                        view! {
                            <LoadFailure
                                words="Could not load your revisions."
                                on_retry=Callback::new(|()| ())
                            />
                        }
                            .into_any(),
                        broken,
                        plate(2),
                        false,
                    ),
                )}
            </Cell>
            <Cell wide=true label="resolve mode, with the exit the design left open">
                <Provider value=DiffersId("resolve-differing-context-pane")>
                    {resolve("#contextpane", compared())}
                </Provider>
            </Cell>
            <Cell wide=true label="resolve mode — the comparison could not be read">
                {resolve("#contextpane", refused())}
            </Cell>
        </Scene>
    }
}

#[cfg(test)]
mod tests {
    use super::{EMPTY, HUGE, UNREAD, counted, download_words, on_page, plate};

    /// The words as read, with the no-break spaces shown as spaces.
    fn plain(words: &str) -> String {
        words.replace('\u{a0}', " ")
    }

    #[test]
    fn the_count_carries_the_size() {
        assert_eq!(
            plain(&counted(plate(2))),
            "54 of 56 files · 1.9 MB of 3.4 MB downloaded"
        );
        assert_eq!(
            plain(&counted(plate(0))),
            "All files are downloaded · 3.4 MB"
        );
        assert_eq!(counted(EMPTY), "This revision has no files");
        assert_eq!(
            plain(&counted(HUGE)),
            "12,400 of 140,000 files · 86.3 GB of 1.2 TB downloaded"
        );
        assert_eq!(counted(UNREAD), "54 of 56 downloaded");
    }

    #[test]
    fn the_backlog_action_says_the_bytes_left() {
        assert_eq!(
            plain(&download_words(plate(2))),
            "Download 2 files · 1.5 MB"
        );
        assert_eq!(download_words(UNREAD), "Download 2 files");
    }

    /// On the page the pane counts the file pane's own rows: its 17 not
    /// downloaded, and the plates' megabytes.
    #[test]
    fn on_the_page_the_sizes_are_the_file_panes() {
        assert_eq!(
            plain(&counted(on_page())),
            "35 of 52 files · 95.7 MB of 161.9 MB downloaded"
        );
        assert_eq!(
            plain(&download_words(on_page())),
            "Download 17 files · 66.2 MB"
        );
    }
}
