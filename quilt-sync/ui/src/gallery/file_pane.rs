//! The installed-package page's file pane, in the states it can hold.
//!
//! # What this scene is for
//!
//! Every part of the pane has a Core cell and the list and the toolbar each have
//! a Combined one. What none of them can show is the four parts stacked at the
//! width the page gives them: whether the box, its controls and its footer read
//! as one region, whether the states the page can reach are all drawable, and
//! what the pane does when it has nothing to draw.
//!
//! The pane is the page's growing half — the context pane is fixed at 280 and
//! this takes the rest — so the cells are `full` and the list is framed at the
//! **700px the pane gets at a 1024 window**, which is the width every claim
//! about this region is made at.
//!
//! # The list is a flush `Card`, and the footer is its last child
//!
//! Verdict 17: only the list and its footer are boxed, and the search row and
//! the toolbar sit bare above them, because they act on the list rather than
//! belonging to it. The rows carry their own padding, so the `Card` is `flush` —
//! the headings stick to its top edge on a scroll and the footer's hairline
//! reaches both borders, neither of which a padded card can do.
//!
//! `Card`'s own rule — *a hairline between any two children* — is what draws
//! that footer border, so the footer is composed here and is not a component.
//!
//! # The footer's arithmetic, on screen
//!
//! §9 measures the height floor with a footer: appbar 48, header 81, the control
//! rows, footer 49, leaving **264px of list**. But the footer exists only when
//! something is ticked, so the resting list is **313** and the first tick costs
//! about a row and a half of the list you are ticking from. The first two cells
//! are those two numbers, in that order.
//!
//! # What this scene draws ahead of its data
//!
//! One cell shows data the backend cannot produce yet, so that its wording is
//! the target the backend has to meet:
//!
//! - **The marked rows.** Nothing computes which files differ between two
//!   diverged revisions. `MergeData` carries only a namespace and a URI, and the
//!   working-tree changes compare local edits with the installed manifest, not
//!   the local revision with the published one.
//!
//! # What the render settled, none of it reasoned first
//!
//! - **The footer's arithmetic is exact.** 313px of list at rest, 264 with the
//!   footer, and the footer measures 49 — so the card is 315 either way and the
//!   pane does not change height when the first row is ticked. §9's number is
//!   the selecting state, as §5 worked out on paper and this confirms.
//! - **End-truncation destroys a path.** Under `Group: None` at 700px, five
//!   consecutive rows read
//!   `investigations/2026-09-15-installed-package-page/design-0…` and three of
//!   them are indistinguishable: the ellipsis eats the leaf, which is the only
//!   part that identifies the file. §6 left the truncation strategy open and
//!   this is the argument for settling it — the end of a path is what a reader
//!   needs and the start is what they can infer.
//! - **The third toolbar row belongs to the minimum window, and only to it.**
//!   The toolbar's content is 716px against the pane's 700, and the pane is the
//!   window less 324 — `page_layout`'s 32, the context pane's 280 and the gap.
//!   So it wraps at a **1024** window, which is the app's `minWidth`, and stops
//!   wrapping at **1040**: sixteen pixels of window buy the row back. Accepted
//!   on 2026-09-18 for that reason — the cost lands on the narrowest window
//!   rather than on the shipping one.
//! - **`Group: None` also changes how many rows the toolbar takes.** `Group:
//!   Base folder` is 166px and `Group: None` about 130, which is more than the
//!   16 the toolbar is short by — so the controls fit one line under `None` and
//!   wrap under the default. The wrap is not a property of the width alone.
//! - **A group of one is mostly heading.** The `Ignored` facet draws three
//!   `.DS_Store` rows under two headings holding one file each, so half the view
//!   is headings and all three rows have the same name. §6's *suppress the
//!   heading for a group of one* has its evidence, and this view is where it
//!   shows.
//! - **The loading state had to reserve two heights, not one.** The toolbar
//!   wraps to 68px and the list is 313, and a skeleton shorter than either moves
//!   the page under the reader at the moment data lands — the same jump the
//!   never-move-geometry rule is about, caused by an arrival rather than a
//!   hover.
//!
//! # Three things the cells settled by being drawn
//!
//! - **The facets make the toolbar content, so it cannot survive a load.** §7's
//!   rule is that chrome is never skeletonised — but these segments carry
//!   counts, and counts are data. Drawn without them the labels change on
//!   arrival, which breaks a control that selects by its option's own string;
//!   drawn with them the toolbar has to wait. So the loading cell skeletonises
//!   the toolbar, and the rule has an exception the design has to record.
//! - **The `[⋯]` cannot be one list.** §6 owes this: gated by where the file is,
//!   the menu is four different menus, and two of its items only exist for one
//!   state each — *Open file* wants a local file and *Stop keeping* wants bytes
//!   to delete. An ignored row needs *Stop ignoring*, which §3's list does not
//!   have at all although the `Ignored` facet is the view that reaches it.
//! - **A facet at zero has to stay.** It is the same argument that keeps the
//!   facets out of a `Select`: a segment that vanishes when it is empty teaches
//!   nothing and moves the segments beside it. `Segment::inert` is that state,
//!   and the eighth cell is a package with nothing changed.

use leptos::context::Provider;
use leptos::prelude::*;

use crate::Cell;
use crate::Scene;
use crate::commands::PullCheck;
use crate::commands::PullOutcome;
use crate::commands::PullPreview;
use crate::differs_caption;
use crate::kit::Blankslate;
use crate::kit::Button;
use crate::kit::ButtonVariant;
use crate::kit::Card;
use crate::kit::CheckState;
use crate::kit::Dialog;
use crate::kit::DiffersId;
use crate::kit::EntryAction;
use crate::kit::EntryGroup;
use crate::kit::EntryRow;
use crate::kit::EntrySelection;
use crate::kit::GroupSelection;
use crate::kit::ListToolbar;
use crate::kit::LoadFailure;
use crate::kit::MenuAction;
use crate::kit::Naming;
use crate::kit::SearchInput;
use crate::kit::Segment;
use crate::kit::SegmentedControl;
use crate::kit::Select;
use crate::kit::SelectAll;
use crate::kit::SkeletonBox;
use crate::kit::state_label::StateTone;
use crate::pages::downloaded_words;
use quilt_sync_ui::util::format_size;
use quilt_sync_ui::util::thousands;

/// The two files the resolve fixture has differing between the revisions. Both
/// are local and both are in the first screen of the list, because a mark the
/// reader has to scroll to proves nothing about the marking. The context pane's
/// resolve scenes count this same list, so the sentence describes these rows.
pub const MARKED: &[&str] = &[
    "README.md",
    "investigations/2026-09-15-installed-package-page/design-01.md",
];

/// The pane at a 1024 window: 1024 less `page_layout`'s 32 of padding, less the
/// context pane's fixed 280 and the gap between them. Every measurement this
/// scene reports is at this width.
const PANE: &str = "width:700px; max-width:100%; gap:var(--q-space-2)";

/// The list's viewport with nothing ticked, and with a footer under it. §9's
/// number is the second one, and it quotes it as the page's capacity — which is
/// the selecting state, not the resting one.
const LIST_RESTING: &str = "max-height:313px";
const LIST_WITH_FOOTER: &str = "max-height:264px";
/// The cap notice is 36px, and at the height floor the pane cannot grow by it —
/// so it comes out of the list rather than out of the page.
const LIST_UNDER_NOTICE: &str = "max-height:277px";

/// On the page, where the height is the window's to give, the pane's own
/// geometry moves to a class — `g-ip-filepane` — because the narrow arrangement
/// has to raise its `min-height` and an inline style cannot be overridden by a
/// container query.
const LIST_FILLING: &str = "flex:1; min-height:0";

// ── the package ─────────────────────────────────────────────

/// Where a file is, which decides its words, its box, its click and its menu.
///
/// Six, against the four the facets name. `New` and `Deleted` are in §3's row
/// table and in no facet's words, which is the question the eleventh cell's note
/// puts: `Changed` has to hold three of these or two of them are unreachable.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mark {
    /// Here and unchanged — most of the list, and silent.
    Here,
    /// Here, and different from the revision.
    Changed,
    /// Here, and not in the revision at all.
    New,
    /// In the revision, and gone from disk.
    Deleted,
    /// Not here. The only mark a tick can act on.
    Missing,
    /// Matched by a `.quiltignore` pattern, and shown by exactly one facet.
    Ignored,
    /// In the newer revision only: neither here nor in the installed one. The
    /// dry run names it before *Get latest*, by path and nothing else.
    Incoming,
}

impl Mark {
    /// The row's words and how loudly, or nothing at all.
    ///
    /// Two silences for two reasons: `Here` because the resting state is what
    /// most of the list is, and `Ignored` because the only view that shows it
    /// says so already.
    const fn state(self) -> Option<(&'static str, StateTone)> {
        match self {
            Self::Here | Self::Ignored => None,
            Self::Changed => Some(("Changed", StateTone::Attention)),
            Self::New => Some(("New", StateTone::Attention)),
            Self::Deleted => Some(("Deleted", StateTone::Danger)),
            Self::Missing => Some(("Not downloaded", StateTone::Neutral)),
            // Attention, the header's own tone for `Newer revision available`:
            // these rows are what that state is about.
            Self::Incoming => Some(("Incoming", StateTone::Attention)),
        }
    }

    /// Whether there is a local file to open. `Deleted` is the case the row
    /// exists for: it is in the revision, so it has a row, and there is nothing
    /// on disk behind it.
    const fn local(self) -> bool {
        matches!(self, Self::Here | Self::Changed | Self::New | Self::Ignored)
    }
}

/// One file, as the pane knows it.
struct File {
    path: String,
    /// Bytes, formatted by the page's own formatter so the column reads as the
    /// app's and not as a fixture's.
    bytes: u64,
    mark: Mark,
}

impl File {
    fn new(path: impl Into<String>, bytes: u64, mark: Mark) -> Self {
        Self {
            path: path.into(),
            bytes,
            mark,
        }
    }
}

/// The package, sized to the design's own numbers so the labels on screen are
/// the labels under discussion: 56 files, 3 ignored, leaving `All 53`, with
/// `Not downloaded 17` and `Ignored 3`.
///
/// `Changed` is **4** and not the design's 2: the four marks that are neither
/// resting nor missing nor ignored all have to land in one facet, and two of
/// them — `New` and `Deleted` — are in §3's row table and in nobody's words.
fn package() -> Vec<File> {
    let mut files = vec![
        File::new("README.md", 2_400, Mark::Here),
        File::new("quilt_summarize.json", 1_100, Mark::Here),
        File::new("manifest.jsonl", 44_000, Mark::Missing),
        File::new("notes/kickoff-thread.md", 12_800, Mark::Changed),
        File::new("notes/intake-upload.md", 8_200, Mark::Changed),
        File::new("notes/plate-07-rerun.md", 3_400, Mark::New),
        File::new("notes/superseded-layout.md", 2_900, Mark::Deleted),
        File::new(".DS_Store", 6_148, Mark::Ignored),
        File::new("notes/.DS_Store", 6_148, Mark::Ignored),
        File::new("raw/.DS_Store", 6_148, Mark::Ignored),
    ];
    for i in 1..=5u64 {
        files.push(File::new(
            format!("notes/handoff-{i:02}.md"),
            9_000 + i * 700,
            Mark::Here,
        ));
    }
    for i in 1..=3u64 {
        files.push(File::new(
            format!("investigations/2026-09-15-installed-package-page/design-{i:02}.md"),
            33_000 + i * 1_200,
            Mark::Here,
        ));
    }
    for i in 4..=5u64 {
        files.push(File::new(
            format!("investigations/2026-09-15-installed-package-page/verdicts-{i:02}.md"),
            31_000 + i * 900,
            Mark::Missing,
        ));
    }
    for i in 1..=22u64 {
        files.push(File::new(
            format!("raw/plate-{i:02}.csv"),
            4_100_000 + i * 21_000,
            Mark::Here,
        ));
    }
    for i in 23..=36u64 {
        files.push(File::new(
            format!("raw/plate-{i:02}.csv"),
            4_100_000 + i * 21_000,
            Mark::Missing,
        ));
    }
    files
}

/// What the context pane's Keeping line says about [`package`] when the page
/// draws the two side by side, so the pane's sizes are these rows' sizes.
pub(crate) struct Kept {
    /// The revision's files: every row but the new and the ignored ones, which
    /// are on disk and in no manifest.
    pub(crate) files: usize,
    /// The ones not downloaded, the rows the file pane can tick.
    pub(crate) pending: usize,
    /// The revision's bytes.
    pub(crate) total: u64,
    /// Those bytes less the not-downloaded rows'. A row deleted here still
    /// counts: the real caption's backlog is the paths this copy neither tracks
    /// nor has a local change at (`keeping_data` in the backend), and a delete
    /// is a local change waiting to be committed, not something Download fetches.
    pub(crate) here: u64,
}

/// [`Kept`], read off [`package`].
pub(crate) fn kept() -> Kept {
    let revision: Vec<File> = package()
        .into_iter()
        .filter(|f| !matches!(f.mark, Mark::New | Mark::Ignored | Mark::Incoming))
        .collect();
    let missing = || revision.iter().filter(|f| f.mark == Mark::Missing);
    let total = revision.iter().map(|f| f.bytes).sum::<u64>();
    Kept {
        files: revision.len(),
        pending: missing().count(),
        total,
        here: total - missing().map(|f| f.bytes).sum::<u64>(),
    }
}

/// The same package with nothing changed, for the cell about a facet that
/// matches nothing. Same files, three marks moved home — a clean copy is a state
/// the page reaches constantly, not a special fixture.
fn settled_package() -> Vec<File> {
    package()
        .into_iter()
        .map(|f| match f.mark {
            Mark::Changed | Mark::New => File {
                mark: Mark::Here,
                ..f
            },
            Mark::Deleted => File {
                mark: Mark::Missing,
                ..f
            },
            _ => f,
        })
        .collect()
}

// ── the facets ──────────────────────────────────────────────

/// A facet: the word it shows, and what it admits.
struct Facet {
    word: &'static str,
    admits: fn(Mark) -> bool,
}

/// The four. `All` excludes ignored files — verdict 21, and why the count is 53
/// of 56. `Changed` admits every mark that is neither resting nor missing,
/// because `New` and `Deleted` have no facet of their own and a row no facet
/// admits cannot be reached at all.
const FACETS: [Facet; 4] = [
    Facet {
        word: "All",
        admits: |m| !matches!(m, Mark::Ignored),
    },
    Facet {
        word: "Changed",
        admits: |m| matches!(m, Mark::Changed | Mark::New | Mark::Deleted),
    },
    Facet {
        word: "Not downloaded",
        admits: |m| matches!(m, Mark::Missing),
    },
    Facet {
        word: "Ignored",
        admits: |m| matches!(m, Mark::Ignored),
    },
];

/// `All 53`, and so on — counted over the whole package rather than the current
/// view, because the toolbar has to render while the list is narrowed and a
/// count that moved under the reader would break the control carrying it.
fn facet_labels(files: &[File]) -> Vec<(&'static str, String, usize)> {
    FACETS
        .iter()
        .map(|facet| {
            // The installed revision's count: an incoming row is drawn under
            // `All`, and is not yet one of the package's files.
            let n = files
                .iter()
                .filter(|f| f.mark != Mark::Incoming && (facet.admits)(f.mark))
                .count();
            (facet.word, format!("{} {n}", facet.word), n)
        })
        .collect()
}

/// What a facet label admits, found by its word — the label carries a count and
/// the count is not part of the question.
fn admits(label: &str) -> fn(Mark) -> bool {
    let word = label.rsplit_once(' ').map_or(label, |(word, _)| word);
    match FACETS.iter().find(|f| f.word == word) {
        Some(f) => f.admits,
        None => |_| true,
    }
}

// ── the rows ────────────────────────────────────────────────

/// A row, reduced to what drawing it needs. The index is its identity for the
/// whole cell, so a group's tri-state cannot disagree with the rows under it.
struct Row {
    path: String,
    /// The folder the file is in, `None` at the package root. Verdict 14: root
    /// files render first and ungrouped, because `(root)` names a directory that
    /// does not exist.
    folder: Option<String>,
    leaf: String,
    size: String,
    /// The manifest's `size`, which the footer sums over the ticked rows.
    bytes: u64,
    mark: Mark,
    /// Its slot in the selection vector, for the rows a tick can act on.
    pick: Option<usize>,
}

/// The package as rows, sorted by path the way `package_data.rs` sorts entries.
fn rows(files: &[File]) -> Vec<Row> {
    let mut sorted: Vec<&File> = files.iter().collect();
    sorted.sort_by(|a, b| a.path.cmp(&b.path));

    let mut next = 0;
    sorted
        .into_iter()
        .map(|f| {
            let pick = (f.mark == Mark::Missing).then(|| {
                let i = next;
                next += 1;
                i
            });
            let (folder, leaf) = match f.path.rfind('/') {
                Some(cut) => (
                    Some(f.path[..=cut].to_string()),
                    f.path[cut + 1..].to_string(),
                ),
                None => (None, f.path.clone()),
            };
            Row {
                path: f.path.clone(),
                folder,
                leaf,
                // The dry run sends paths only, so an incoming row has no size
                // to show, and a `0 B` would claim one.
                size: if f.mark == Mark::Incoming {
                    String::new()
                } else {
                    format_size(f.bytes)
                },
                bytes: f.bytes,
                mark: f.mark,
                pick,
            }
        })
        .collect()
}

/// The row-level confirm: whether it is open, and the file it is about.
///
/// **One value, because the two are never set apart.** Opening without setting
/// the subject would put the previous file's name in front of a reader about to
/// delete this one, which is the exact failure verdict 8's *name what goes* is
/// written against.
#[derive(Clone, Copy)]
struct Confirm {
    open: RwSignal<bool>,
    subject: RwSignal<String>,
}

impl Confirm {
    fn new(subject: &str) -> Self {
        Self {
            open: RwSignal::new(false),
            subject: RwSignal::new(subject.to_string()),
        }
    }

    fn ask(self, subject: String) {
        self.subject.set(subject);
        self.open.set(true);
    }
}

/// The `[⋯]`, gated by where the file is.
///
/// §6 owes this — *"the menu must be gated the same way, or it offers `Open
/// file` on a file that is not there"* — and gating turns one list into four.
/// Two items exist for one state each: `Open file` wants something on disk, and
/// `Stop keeping` wants bytes to delete, which a `Deleted` or `Missing` row has
/// none of.
///
/// `Stop ignoring` is not in §3's list at all, although the `Ignored` facet is
/// the view that reaches it and v1 has the popup. A menu that can ignore and
/// cannot un-ignore is a one-way door.
fn menu(mark: Mark, subject: String, confirm: Confirm) -> Vec<MenuAction> {
    let mut items = Vec::new();
    if mark.local() {
        items.push(MenuAction::new("Open file", Callback::new(|()| ())));
    }
    items.push(MenuAction::new("Open in catalog", Callback::new(|()| ())));
    items.push(MenuAction::new("Copy URI", Callback::new(|()| ())));
    items.push(if mark == Mark::Ignored {
        MenuAction::new("Stop ignoring", Callback::new(|()| ()))
    } else {
        MenuAction::new("Ignore", Callback::new(|()| ()))
    });
    if matches!(mark, Mark::Here | Mark::Changed | Mark::New) {
        items.push(
            MenuAction::new(
                "Stop keeping",
                // The dialog names the row it was opened from. A confirm that
                // names some other file is the defect verdict 8 exists to
                // prevent, and a fixture is where it would go unnoticed.
                Callback::new(move |()| confirm.ask(subject.clone())),
            )
            .danger(),
        );
    }
    items
}

/// One row, in the shape its mark puts it in.
///
/// The click follows the file: one that is here opens, one that is not here
/// ticks, and a row that can do neither takes no pointer. `Ignored` is the one
/// place the click and the menu disagree — §3 calls an ignored file unopenable
/// and the file is plainly on disk, so the row stays inert while the menu, which
/// names what it does rather than inferring it, offers `Open file`.
///
/// A file that is here carries the check, in every scope and whatever the rest
/// of the list is, so the column fills up as files land; a file that is not
/// carries a box when it can be picked, otherwise nothing.
fn row(
    r: &Row,
    flat: bool,
    picks: RwSignal<Vec<bool>>,
    marked: bool,
    boxes: bool,
    confirm: Confirm,
) -> AnyView {
    let name = if flat { r.path.clone() } else { r.leaf.clone() };
    let state = r.mark.state();
    let words = state.map(|(words, _)| words.to_string());
    let tone = state.map_or(StateTone::Neutral, |(_, tone)| tone);
    // Nothing to open, copy or ignore yet: the file is in no revision this
    // copy has, so its row has no `[⋯]` at all.
    let actions = if r.mark == Mark::Incoming {
        Vec::new()
    } else {
        menu(r.mark, format!("{} ({})", r.path, r.size), confirm)
    };

    match (r.pick, boxes) {
        (Some(i), true) => view! {
            <EntryRow
                name=name
                state=words
                tone=tone
                size=r.size.clone()
                differs=marked
                action=EntryAction::Select(
                    EntrySelection::new(
                        Signal::derive(move || picks.with(|p| p[i])),
                        Callback::new(move |next| picks.update(|p| p[i] = next)),
                    ),
                )
                actions=actions
            />
        }
        .into_any(),
        _ if r.mark.local() && r.mark != Mark::Ignored => view! {
            <EntryRow
                name=name
                state=words
                tone=tone
                size=r.size.clone()
                differs=marked
                have_mark=true
                action=EntryAction::Open(Callback::new(|()| ()))
                actions=actions
            />
        }
        .into_any(),
        _ => view! {
            <EntryRow
                name=name
                state=words
                tone=tone
                size=r.size.clone()
                differs=marked
                actions=actions
            />
        }
        .into_any(),
    }
}

// ── the pane ────────────────────────────────────────────────

/// Where the pane is being drawn, which is what decides how it takes its height.
///
/// Not a `fill: bool` beside the others: the two framings differ in more than a
/// flag's worth of behaviour, and naming the place says why the numbers differ.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Framing {
    /// A gallery cell, where the list is capped at §5's own numbers because
    /// those numbers are what the cell is about.
    Cell,
    /// The page, where the list takes what the window left and scrolls.
    Page,
}

/// What the list box holds when it is not holding rows.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Body {
    /// The rows the view leaves.
    Rows,
    /// The call has not answered.
    Loading,
    /// The call failed.
    Failed,
    /// The rows, under the sentence saying they are not all of them: the
    /// package's whole count, of which the rows are the first [`LOADED`].
    Capped(usize),
}

/// How the pane is standing when a cell draws it. One struct rather than nine
/// arguments, so a cell reads as the two or three things that make it different.
struct Pane {
    /// The radio group's name. Unique per cell — two `SegmentedControl`s sharing
    /// one become a single group across both cells.
    name: &'static str,
    files: Vec<File>,
    /// The facet's word, not its label: the count belongs to the fixture.
    facet: &'static str,
    query: &'static str,
    grouped: bool,
    /// `Keeping → The whole package`, which takes the per-file choice away:
    /// no boxes, no select-all, and the footer can never appear. What the
    /// slot shows instead follows the files: a caption once all are here.
    whole: bool,
    /// Resolve mode: the files that differ between the two revisions, marked.
    marked: &'static [&'static str],
    /// How many of the selectable rows start ticked.
    ticked: usize,
    /// The download is in flight.
    running: bool,
    /// Where it is drawn, which decides whether the list is capped or fills.
    framing: Framing,
    body: Body,
    /// The package is behind: what the dry run has said so far, or `None` for
    /// a package that is not.
    incoming: Option<PullCheck>,
    /// Where the incoming rows sit.
    placement: Placement,
}

impl Pane {
    fn new(name: &'static str) -> Self {
        Self {
            name,
            files: package(),
            facet: "All",
            query: "",
            grouped: true,
            whole: false,
            marked: &[],
            ticked: 0,
            running: false,
            framing: Framing::Cell,
            body: Body::Rows,
            incoming: None,
            placement: Placement::OnTop,
        }
    }
}

// ── the newer revision ──────────────────────────────────────

/// Where the files a newer revision brings are drawn in the list.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Placement {
    /// One group at the top of the list, under its own heading, each row
    /// showing its whole path.
    #[default]
    OnTop,
    /// Each row in its own folder, among the installed files.
    InPlace,
}

/// The heading over [`Placement::OnTop`]'s group.
const INCOMING_HEADING: &str = "From the newer revision";

/// `1 file` or `3 files`, with the count in the app's own separators.
fn files_word(n: usize) -> String {
    if n == 1 {
        String::from("1 file")
    } else {
        format!("{} files", thousands(n))
    }
}

/// What the list box's first line says about the dry run, or `None` when it
/// found nothing worth a line: a revision that adds no files and keeps no
/// local changes. `whole` is the sync scope, which decides what *Get latest*
/// does with the files: list them under individual-file sync, download them
/// under the whole package.
///
/// Only the dry run's two facts are worded — the verdict and the added paths.
/// It does not say which files the newer revision changes or removes, so
/// nothing here does either.
fn incoming_words(check: &PullCheck, whole: bool) -> Option<String> {
    let preview = match check {
        PullCheck::Loading => {
            return Some(String::from(
                "Checking which files the newer revision brings\u{2026}",
            ));
        }
        PullCheck::Failed => {
            return Some(String::from(
                "Couldn't check which files the newer revision brings.",
            ));
        }
        PullCheck::Ready(preview) => preview,
    };
    if let PullOutcome::Blocked { conflicts } = &preview.outcome {
        let them = if conflicts.len() == 1 { "it" } else { "them" };
        return Some(format!(
            "{} changed here and in the newer revision. Publish your changes, then resolve {them}.",
            files_word(conflicts.len()),
        ));
    }
    let keeps = matches!(preview.outcome, PullOutcome::KeepsLocalChanges { .. });
    let n = preview.added.len();
    if n == 0 {
        return keeps.then(|| String::from("Get latest keeps your changes."));
    }
    let them = if n == 1 { "it" } else { "them" };
    let fate = if whole {
        format!("Get latest downloads {them}.")
    } else {
        format!("Get latest lists {them} here, to download when you need {them}.")
    };
    let kept = if keeps { " Your changes stay." } else { "" };
    Some(format!(
        "The newer revision brings {}. {fate}{kept}",
        files_word(n)
    ))
}

/// The files the dry run names, as rows: each added path with no row of its
/// own already. A path the copy has as a new file of its own is the both-added
/// case, which the verdict answers; it keeps its one row.
fn with_incoming(mut files: Vec<File>, check: Option<&PullCheck>) -> Vec<File> {
    if let Some(PullCheck::Ready(preview)) = check {
        for path in &preview.added {
            if !files.iter().any(|f| &f.path == path) {
                files.push(File::new(path.clone(), 0, Mark::Incoming));
            }
        }
    }
    files
}

/// The paths a `Blocked` verdict names, which take resolve mode's mark.
fn conflicts(check: Option<&PullCheck>) -> Vec<String> {
    match check {
        Some(PullCheck::Ready(PullPreview {
            outcome: PullOutcome::Blocked { conflicts },
            ..
        })) => conflicts.clone(),
        _ => Vec::new(),
    }
}

/// The list box's first line about the dry run: the sentence, and *Try again*
/// beside it when the check failed. Muted like the cap's notice, which shares
/// the slot, because both describe the list rather than interrupt it.
fn incoming_line(check: Option<&PullCheck>, whole: bool) -> Option<AnyView> {
    let check = check?;
    let words = incoming_words(check, whole)?;
    let failed = check.is_failed();
    // A conflict's sentence is what the marked rows' `Differs` describes, so
    // it carries the id their description points at.
    let id = matches!(
        check,
        PullCheck::Ready(PullPreview {
            outcome: PullOutcome::Blocked { .. },
            ..
        })
    )
    .then(crate::kit::differs_id);
    Some(
        view! {
            <div class="g-fp-incoming">
                <p id=id>{words}</p>
                {failed.then(|| view! { <Button on_click=|_| ()>"Try again"</Button> })}
            </div>
        }
        .into_any(),
    )
}

/// The whole region: the search row, the toolbar, the box, and the footer when
/// there is something to download.
#[allow(clippy::too_many_lines, reason = "one region, drawn once")]
fn pane(p: Pane) -> AnyView {
    let Pane {
        name,
        files,
        facet,
        query,
        grouped,
        whole,
        marked,
        ticked,
        running,
        framing,
        body,
        incoming,
        placement,
    } = p;

    let files = with_incoming(files, incoming.as_ref());
    let conflicting = conflicts(incoming.as_ref());
    let line = incoming_line(incoming.as_ref(), whole);
    let on_top = placement == Placement::OnTop;

    let labels = facet_labels(&files);
    let selected = labels
        .iter()
        .find(|(word, ..)| *word == facet)
        .map_or_else(|| String::from("All"), |(_, label, _)| label.clone());
    let segments: Vec<Segment> = labels
        .iter()
        .map(|(_, label, n)| {
            // Never inert the selected segment: a disabled radio cannot be left,
            // so the control would be stuck on a view that has no rows.
            if *n == 0 && *label != selected {
                Segment::inert(label.clone())
            } else {
                Segment::new(label.clone())
            }
        })
        .collect();

    let all = rows(&files);
    let pickable = all.iter().filter(|r| r.pick.is_some()).count();
    let picks = RwSignal::new(vec![false; pickable]);
    picks.update(|p| {
        for slot in p.iter_mut().take(ticked) {
            *slot = true;
        }
    });

    let all = StoredValue::new(all);
    // Seeded, because a gallery cell can open this from its own trigger as well
    // as from a row's menu.
    let confirm = Confirm::new("raw/plate-03.csv (4.2\u{a0}MB)");

    let query_sig = RwSignal::new(query.to_string());
    let facet_sig = RwSignal::new(selected);
    let group_sig = RwSignal::new(if grouped { "Base folder" } else { "None" }.to_string());

    // Which rows the facet and the search leave, as indices into the sorted
    // package. One derivation feeding the list, select-all's numbers and the
    // empty states, so the three cannot disagree about what is on screen.
    let shown = Signal::derive(move || {
        let keep = admits(&facet_sig.get());
        let needle = query_sig.get().to_lowercase();
        all.with_value(|rs| {
            rs.iter()
                .enumerate()
                .filter(|(_, r)| keep(r.mark) && r.path.to_lowercase().contains(&needle))
                .map(|(i, _)| i)
                .collect::<Vec<usize>>()
        })
    });

    // Only a not-downloaded row can be ticked, so the two numbers part company
    // under every facet but one — and under `Changed` there is nothing to tick
    // at all, which is what takes select-all off the toolbar.
    let offered = Signal::derive(move || {
        all.with_value(|rs| {
            shown
                .get()
                .iter()
                .filter(|&&i| rs[i].pick.is_some())
                .count()
        })
    });
    // Select-all's own number: the ticks among the rows on screen.
    let shown_ticked = Signal::derive(move || {
        all.with_value(|rs| {
            shown
                .get()
                .iter()
                .filter(|&&i| rs[i].pick.is_some_and(|slot| picks.with(|p| p[slot])))
                .count()
        })
    });
    // What the footer's press fetches, its count and its bytes read off one
    // set: every loaded tick, shown or not, as the real footer counts them — a
    // search hides a tick, it does not undo it. Only a missing file takes a
    // tick, so the bytes are exactly what is downloaded, summed over rows the
    // page already holds, with no I/O.
    let chosen = Memo::new(move |_| all.with_value(|rs| picks.with(|p| chosen_among(rs, p))));
    let narrowed =
        Signal::derive(move || !query_sig.get().is_empty() || !facet_sig.get().starts_with("All"));

    // The page's rule: the package, not the view, has nothing left to
    // download, so the slot select-all would take says so under either scope.
    // A deleted file has nothing to fetch, so it counts as downloaded, and
    // the words name it.
    let downloaded = {
        let counted = files
            .iter()
            .filter(|f| !matches!(f.mark, Mark::Ignored | Mark::Incoming))
            .count();
        let deleted = files.iter().filter(|f| f.mark == Mark::Deleted).count();
        let missing = files.iter().any(|f| f.mark == Mark::Missing);
        (!missing && counted > 0).then_some((counted, deleted))
    };
    let fill = framing == Framing::Page;
    let boxes = !whole;
    let shape = Shape {
        boxes,
        capped: matches!(body, Body::Capped(_)),
    };
    let footer_shown = Signal::derive(move || boxes && chosen.get().files > 0);
    let marked: Vec<String> = marked
        .iter()
        .map(|&p| p.to_string())
        .chain(conflicting)
        .collect();
    let marked = StoredValue::new(marked);
    let empty_package = pickable == 0 && all.with_value(Vec::is_empty);

    let list = move || {
        let flat = group_sig.get() == "None";
        let indices = shown.get();

        if indices.is_empty() {
            return if empty_package {
                view! {
                    <Blankslate
                        heading="Nothing in this package"
                        description="The published revision has no files in it yet."
                    />
                }
                .into_any()
            } else {
                // Compact: `Blankslate`'s own `space-10` is taller than the
                // 264px this box gets at the height floor.
                //
                // No action, and that is a deferral rather than a rule — two
                // controls narrowed this view and clearing one of them can
                // land the reader on a second empty list.
                view! {
                    <Blankslate
                        compact=true
                        heading="No files match"
                        description="Nothing in this view matches what you are looking for."
                    />
                }
                .into_any()
            };
        }

        all.with_value(|rs| {
            let mut roots: Vec<AnyView> = Vec::new();
            let mut groups: Vec<(String, Vec<usize>)> = Vec::new();
            let mut arriving: Vec<usize> = Vec::new();

            for i in indices {
                if on_top && rs[i].mark == Mark::Incoming {
                    arriving.push(i);
                    continue;
                }
                match (&rs[i].folder, flat) {
                    (Some(folder), false) => match groups.last_mut() {
                        Some((name, members)) if name == folder => members.push(i),
                        _ => groups.push((folder.clone(), vec![i])),
                    },
                    _ => roots.push(draw(&rs[i], flat, picks, marked, boxes, confirm)),
                }
            }

            view! {
                {(!arriving.is_empty())
                    .then(|| incoming_group(arriving, all, picks, marked, boxes, confirm))}
                {roots}
                {groups
                    .into_iter()
                    .map(|(name, members)| group(name, members, all, picks, marked, shape, confirm))
                    .collect_view()}
            }
            .into_any()
        })
    };

    view! {
        <div
            class=if fill { "g-stack g-ip-filepane" } else { "g-stack" }
            style=if fill { "" } else { PANE }
        >
            // Bare, both rows: verdict 17, they act on the list rather than
            // belonging to it. Search takes the pane's full width; the toolbar's
            // content is inset to the rows' checkbox column.
            // A row of its own, and the `display:flex` is load-bearing rather
            // than decorative: `SearchInput` carries `flex: 1 1 auto` so it
            // takes the free space in a toolbar — and dropped straight into
            // this column that grew it *vertically*, to 162px, because in a
            // column the main axis is the one it was asked to fill. The same
            // mistake `Card` already has written up: a parent's layout living
            // in a child, invisible until the parent is a column with height to
            // give away. A row is what the component assumes, so a row is what
            // it gets.
            <div style="display:flex">
                <SearchInput
                    value=query_sig
                    aria_label="Search files"
                    placeholder="Search files…"
                />
            </div>
            // The toolbar and the box as one child of the column, as on the page:
            // `ListToolbar` carries its own space-2 below it, so as siblings the
            // column's gap doubled it and the toolbar sat further from its list
            // than from the search.
            <div class="g-stack" style=if fill { LIST_FILLING } else { "" }>
                {match body {
                    Body::Loading => {
                        view! {
                            // The toolbar's own labels carry counts, and counts are
                            // data — so this is the one piece of chrome that cannot
                            // outlive its load. §7 has to record the exception.
                            //
                            // It reserves the height the real toolbar takes, which
                            // at this width is two lines and not one: a skeleton
                            // that stands 36px shorter than what replaces it moves
                            // the list down on arrival, which is the jump the
                            // never-move-geometry rule exists to prevent — and the
                            // rule binds a load as much as a hover.
                            <div
                                class="g-stack"
                                style="gap:var(--q-space-2); padding:var(--q-space-1) 0"
                            >
                                <div style="display:flex; gap:var(--q-space-2); \
                                            justify-content:flex-end">
                                    <SkeletonBox width="166px" height="32px" />
                                    <SkeletonBox width="396px" height="32px" />
                                </div>
                                <div style="display:flex; gap:var(--q-space-2)">
                                    <span style="flex:0 0 28px" />
                                    <SkeletonBox width="102px" height="20px" />
                                </div>
                            </div>
                        }
                            .into_any()
                    }
                    _ => {
                        view! {
                            <ListToolbar reverse_when_stacked=true>
                                // Select-all sits in the rows' checkbox column, the
                                // list box's space-3 plus the 16px gutter in from
                                // the pane's edge. With nothing left to download
                                // the slot says so instead, its words in the rows'
                                // name column: the box's 17px stays empty.
                                {downloaded
                                    .map(|(n, deleted)| {
                                        view! {
                                            <span style="flex:0 0 28px" />
                                            // The column's summary, as the page draws
                                            // it: the Success tone's own tick. The
                                            // rows' check is muted bookkeeping; this is
                                            // a statement. A file deleted here is not
                                            // on disk, so the hole stays empty.
                                            <span class="g-fp-done">
                                                {(deleted == 0)
                                                    .then(|| StateTone::Success.glyph())}
                                            </span>
                                            <span style="font-size:var(--q-text-body); \
                                                         color:var(--q-fgColor-muted)">
                                                {downloaded_words(n, deleted)}
                                            </span>
                                        }
                                    })}
                                <Show when=move || {
                                    downloaded.is_none() && boxes && (offered.get() > 0)
                                }>
                                    <span style="flex:0 0 28px" />
                                    <SelectAll
                                        selected=shown_ticked
                                        total=offered
                                        narrowed=narrowed
                                        on_toggle=move |next| {
                                            let indices = shown.get();
                                            all.with_value(|rs| {
                                                picks
                                                    .update(|p| {
                                                        for i in indices {
                                                            if let Some(slot) = rs[i].pick {
                                                                p[slot] = next;
                                                            }
                                                        }
                                                    });
                                            });
                                        }
                                    />
                                </Show>
                                <div style="margin-left:auto; display:flex; gap:var(--q-space-2); \
                                            flex-wrap:wrap-reverse; justify-content:flex-end">
                                    <Select
                                        naming=Naming::Prefix("Group".to_string())
                                        options=vec!["Base folder".to_string(), "None".to_string()]
                                        selected=group_sig
                                    />
                                    <SegmentedControl
                                        aria_label="Filter files"
                                        name=name
                                        options=segments
                                        selected=facet_sig
                                    />
                                </div>
                            </ListToolbar>
                        }
                            .into_any()
                    }
                }}
                // The box. `flush`, because the rows carry their own padding: the
                // headings stick to its top edge and the footer's hairline — which
                // is `Card`'s own rule about two children — reaches both borders.
                <Card flush=true label="Files" fill=fill>
                    {match body {
                        Body::Failed => {
                            view! {
                                <LoadFailure
                                    centred=true
                                    words="Could not read this package's files."
                                    on_retry=Callback::new(|()| ())
                                />
                            }
                                .into_any()
                        }
                        Body::Loading => {
                            view! {
                                // The box reserves the resting list's height too. A
                                // skeleton that stands 117px shorter than the rows
                                // it becomes moves the whole page under the reader
                                // at the moment the data lands.
                                <div style=format!(
                                    "{LIST_RESTING}; height:313px; overflow:hidden; \
                                     padding:var(--q-space-2) var(--q-space-3); \
                                     display:flex; flex-direction:column; \
                                     gap:var(--q-space-3)",
                                )>
                                    {(0..8)
                                        .map(|i| {
                                            view! {
                                                <SkeletonBox width=if i % 3 == 0 {
                                                    "48%"
                                                } else if i % 3 == 1 {
                                                    "62%"
                                                } else {
                                                    "55%"
                                                } />
                                            }
                                        })
                                        .collect_view()}
                                </div>
                            }
                                .into_any()
                        }
                        _ => {
                            view! {
                                {line}
                                {match body {
                                    Body::Capped(total) => Some(total),
                                    _ => None,
                                }
                                    .map(|total| {
                                        view! {
                                            // The first 1,000 by path: the page read
                                            // sorts before it caps and sends the
                                            // total (quilt-rs#992).
                                            <p style="margin:0; padding:var(--q-space-2) \
                                                      var(--q-space-3); \
                                                      color:var(--q-fgColor-muted); \
                                                      font-size:var(--q-text-body)">
                                                {format!(
                                                    "This package has {} files. This list covers the first {} by path.",
                                                    thousands(total),
                                                    thousands(LOADED),
                                                )}
                                            </p>
                                        }
                                    })}
                                <div
                                    style=move || {
                                        format!(
                                            "{}; overflow-y:auto; --q-entry-gutter:{}",
                                            if fill {
                                                LIST_FILLING
                                            } else {
                                                match (footer_shown.get(), body) {
                                                    (true, _) => LIST_WITH_FOOTER,
                                                    (false, Body::Capped(_)) => LIST_UNDER_NOTICE,
                                                    (false, _) => LIST_RESTING,
                                                }
                                            },
                                            if group_sig.get() == "None" { "0" } else { "16px" },
                                        )
                                    }
                                >
                                    {list}
                                </div>
                            }
                                .into_any()
                        }
                    }}
                    <Show when=move || footer_shown.get()>
                        // Spacer and a Button, and `Card` draws the hairline above
                        // them. It slides 4px and fades in, copying the Banner's
                        // carve-out from the never-move-geometry rule, and has no
                        // exit animation: unticking the last row makes the list grow
                        // back, and animating that would move rows under the pointer.
                        <div class="g-fp-footer">
                            <Button
                                variant=ButtonVariant::Primary
                                loading=running
                                on_click=move |_| ()
                            >
                                {move || footer_words(chosen.get())}
                            </Button>
                        </div>
                    </Show>
                </Card>
            </div>
            {stop_keeping(confirm)}
        </div>
    }
    .into_any()
}

/// What the whole cell decides for each heading: whether rows carry boxes,
/// and whether the list is over the cap.
#[derive(Clone, Copy)]
struct Shape {
    /// Per-file choice is on: `Keeping → Files I pick`.
    boxes: bool,
    /// The cap notice is drawn, so the last folder may run past the cut.
    capped: bool,
}

/// One row, with the resolve mark looked up rather than passed down: whether a
/// file differs is a fact about the file, so the list does not have to carry a
/// second parallel list of booleans beside its rows.
fn draw(
    r: &Row,
    flat: bool,
    picks: RwSignal<Vec<bool>>,
    marked: StoredValue<Vec<String>>,
    boxes: bool,
    confirm: Confirm,
) -> AnyView {
    let differs = marked.with_value(|paths| paths.contains(&r.path));
    row(r, flat, picks, differs, boxes, confirm)
}

/// [`Placement::OnTop`]'s group: one heading over every incoming row, each
/// showing its whole path, since the rows come from many folders. No box and
/// no check — nothing under it can be ticked, and none of it is here.
fn incoming_group(
    members: Vec<usize>,
    all: StoredValue<Vec<Row>>,
    picks: RwSignal<Vec<bool>>,
    marked: StoredValue<Vec<String>>,
    boxes: bool,
    confirm: Confirm,
) -> AnyView {
    let count = members.len();
    let members = StoredValue::new(members);
    let children = move || {
        all.with_value(|rs| {
            members.with_value(|ms| {
                ms.iter()
                    .map(|&i| draw(&rs[i], true, picks, marked, boxes, confirm))
                    .collect_view()
            })
        })
    };
    view! {
        <EntryGroup
            name=INCOMING_HEADING.to_string()
            count=Signal::derive(move || count)
            open=RwSignal::new(true)
        >
            {children}
        </EntryGroup>
    }
    .into_any()
}

/// A heading and its rows. The heading's box is derived from the rows under it
/// and toggles exactly those, so `Mixed` is a fact about them rather than an
/// assertion beside them — and a group with nothing selectable carries no box,
/// because it would be a control with nothing to act on. It carries the rows'
/// check instead when every file in its folder is here, judged over the whole
/// package rather than the rows shown, as the page's heading does.
fn group(
    name: String,
    members: Vec<usize>,
    all: StoredValue<Vec<Row>>,
    picks: RwSignal<Vec<bool>>,
    marked: StoredValue<Vec<String>>,
    shape: Shape,
    confirm: Confirm,
) -> AnyView {
    let Shape { boxes, capped } = shape;
    let open = RwSignal::new(true);
    let count = members.len();
    let mine: Vec<usize> =
        all.with_value(|rs| members.iter().filter_map(|&i| rs[i].pick).collect());

    let selection = (boxes && !mine.is_empty()).then(|| {
        let derived = mine.clone();
        let toggled = mine;
        GroupSelection::new(
            Signal::derive(move || {
                let ticked = picks.with(|p| derived.iter().filter(|&&i| p[i]).count());
                if ticked == 0 {
                    CheckState::Off
                } else if ticked == derived.len() {
                    CheckState::On
                } else {
                    CheckState::Mixed
                }
            }),
            Callback::new(move |next: bool| {
                picks.update(|p| {
                    for &i in &toggled {
                        p[i] = next;
                    }
                });
            }),
        )
    });

    // The whole folder, not the rows the view leaves, so a facet or a search
    // that hides a missing file never earns the folder the check. Only the
    // `Ignored` facet shows ignored rows, and it shows nothing else, so a
    // heading over nothing but those is that view's, which draws no check.
    let ignored_view = all.with_value(|rs| members.iter().all(|&i| rs[i].mark == Mark::Ignored));
    // Over the cap the page's rule: the rows are sorted by path, so only a
    // folder holding the last drawn path can run past the cut, and it gets no
    // check.
    let straddles = capped
        && all.with_value(|rs| {
            rs.last()
                .is_some_and(|last| last.path.starts_with(name.as_str()))
        });
    let all_here = !ignored_view
        && !straddles
        && all.with_value(|rs| {
            let mut tracked = rs
                .iter()
                .filter(|r| {
                    r.folder.as_deref() == Some(name.as_str())
                        && !matches!(r.mark, Mark::Ignored | Mark::Incoming)
                })
                .peekable();
            tracked.peek().is_some() && tracked.all(|r| r.mark.local())
        });

    let members = StoredValue::new(members);
    let children = move || {
        all.with_value(|rs| {
            members.with_value(|ms| {
                ms.iter()
                    .map(|&i| draw(&rs[i], false, picks, marked, boxes, confirm))
                    .collect_view()
            })
        })
    };

    match selection {
        Some(selection) => view! {
            <EntryGroup
                name=name
                count=Signal::derive(move || count)
                open=open
                selection=selection
            >
                {children}
            </EntryGroup>
        }
        .into_any(),
        None => view! {
            <EntryGroup
                name=name
                count=Signal::derive(move || count)
                open=open
                have_mark=all_here
            >
                {children}
            </EntryGroup>
        }
        .into_any(),
    }
}

// ── the two confirms ────────────────────────────────────────

/// `Stop keeping`, from a row's `[⋯]`.
///
/// Verdict 8: it names exactly what goes. Everything in this sentence is local —
/// the row's own path and its size — so this is the one confirm on the page
/// whose copy the backend can already produce, even though the command behind it
/// cannot: only whole-package `package_uninstall` is registered, and §3 wants
/// `uninstall_paths`.
///
/// `Cancel` first and the confirm `Primary` last, as `Dialog` has it everywhere
/// else on this platform. The weight sits in the words, because Danger in this
/// system is a *status* colour and a red confirm would read as *this errored*.
fn stop_keeping(confirm: Confirm) -> AnyView {
    let Confirm { open, subject } = confirm;
    view! {
        <Dialog
            open=open
            title="Stop keeping this file?"
            footer=view! {
                <Button on_click=move |_| open.set(false)>"Cancel"</Button>
                <Button variant=ButtonVariant::Primary on_click=move |_| open.set(false)>
                    "Stop keeping"
                </Button>
            }
                .into_any()
        >
            <p style="margin:0">
                {move || {
                    format!(
                        "{} will be deleted from this computer. It stays in the package, and \
                         you can download it again.",
                        subject.get(),
                    )
                }}
            </p>
        </Dialog>
    }
    .into_any()
}

/// The resolve confirm, from the context pane's destructive action.
///
/// Its sentence needs two numbers the backend does not produce yet: the
/// revision count needs `list_revisions`, and the file count needs a row-by-row
/// comparison of two diverged manifests, which nothing computes. The scene uses
/// fixture numbers to show the wording the backend has to support.
fn replace_mine(open: RwSignal<bool>) -> AnyView {
    view! {
        <Dialog
            open=open
            title="Replace your files with the published ones?"
            footer=view! {
                <Button on_click=move |_| open.set(false)>"Cancel"</Button>
                <Button variant=ButtonVariant::Primary on_click=move |_| open.set(false)>
                    "Replace mine"
                </Button>
            }
                .into_any()
        >
            <p style="margin:0">
                "3 revisions you have not published and 2 changed files will be replaced. \
                 This cannot be undone."
            </p>
        </Dialog>
    }
    .into_any()
}

/// This scene's package as the page read's entries: sorted by path, ignored
/// files included for the pane's `Ignored` facet.
fn entries(files: Vec<File>) -> Vec<crate::commands::EntryData> {
    let mut files = files;
    files.sort_by(|a, b| a.path.cmp(&b.path));
    files
        .into_iter()
        .map(|f| crate::commands::EntryData {
            status: match f.mark {
                Mark::Here | Mark::Ignored => "pristine",
                Mark::Changed => "modified",
                Mark::New => "added",
                Mark::Deleted => "deleted",
                // Never reached: no live cell draws an incoming row, and the
                // page read has no such status. After a pull under
                // individual-file sync the file arrives as this.
                Mark::Missing | Mark::Incoming => "remote",
            }
            .to_string(),
            ignored_by: (f.mark == Mark::Ignored).then(|| ".DS_Store".to_string()),
            filename: f.path,
            size: f.bytes,
            junky_pattern: None,
            namespace: "team/dataset".try_into().expect("a namespace"),
        })
        .collect()
}

/// The page read's cap: it sends the first 1,000 entries by path, and only a
/// loaded row takes a tick. The backend's constant is not visible to this
/// crate, so it is restated here once, for every cell that honours it.
const LOADED: usize = 1_000;

/// The rows the page would load from `files`: sorted by path and cut at
/// [`LOADED`], with the package's whole count beside them.
fn loaded(mut files: Vec<File>) -> (Vec<File>, usize) {
    files.sort_by(|a, b| a.path.cmp(&b.path));
    let total = files.len();
    files.truncate(LOADED);
    (files, total)
}

/// The page read's list over `files`, built as the backend builds it: sorted,
/// counted over the whole package, then capped at [`LOADED`].
fn entry_list(files: Vec<File>) -> crate::commands::EntryList {
    let mut entries = entries(files);
    let mut counts = crate::commands::EntryCounts::default();
    for e in &entries {
        if e.ignored_by.is_some() {
            counts.ignored += 1;
            continue;
        }
        counts.all += 1;
        match e.status.as_str() {
            "added" | "modified" => counts.changed += 1,
            "deleted" => {
                counts.changed += 1;
                counts.deleted += 1;
            }
            "remote" => counts.not_downloaded += 1,
            _ => {}
        }
    }
    let total = entries.len();
    entries.truncate(LOADED);
    crate::commands::EntryList {
        truncated: total > entries.len(),
        entries,
        counts,
        total,
    }
}

/// The page's `FilePane` over `list`.
fn live(list: crate::commands::EntryList) -> AnyView {
    view! {
        <div style=format!("display:flex; flex-direction:column; {LIST_RESTING}; height:400px")>
            <crate::pages::FilePane
                listing=Signal::stored(crate::pages::Listing::from(
                    crate::commands::FilesData::Listed(list),
                ))
                grouping=RwSignal::new(crate::pages::Grouping::BaseFolder.label().to_string())
                collapsed=RwSignal::new(std::collections::BTreeSet::new())
                search=RwSignal::new(String::new())
                facet=RwSignal::new(crate::pages::Facet::All.key().to_string())
                on_open=Callback::new(|_: String| ())
                on_retry=Callback::new(|()| ())
            />
        </div>
    }
    .into_any()
}

/// This scene's package with every file here and nothing changed, the state
/// whole-package Keeping settles into: nothing is left to download, so the
/// toolbar's left slot says so instead of offering select-all.
fn downloaded_package() -> Vec<File> {
    package()
        .into_iter()
        .map(|f| match f.mark {
            Mark::Ignored => f,
            _ => File {
                mark: Mark::Here,
                ..f
            },
        })
        .collect()
}

/// The same, but with `notes/superseded-layout.md` still deleted here: nothing
/// left to download, and one change waiting to be published.
fn deleted_here_package() -> Vec<File> {
    package()
        .into_iter()
        .map(|f| match f.mark {
            Mark::Ignored | Mark::Deleted => f,
            _ => File {
                mark: Mark::Here,
                ..f
            },
        })
        .collect()
}

/// This scene's package grown to 1,089 files by a folder of plates this copy
/// has not downloaded, so the page read cuts it.
fn over_the_cap() -> Vec<File> {
    let mut files = package();
    let plates = 1_089 - files.len();
    for i in 1..=plates {
        files.push(File::new(
            format!("plates/plate-{i:04}.csv"),
            18_000,
            Mark::Missing,
        ));
    }
    files
}

/// The footer's `[Download]`, in files and bytes: `Download 3 · 1.2 MB`, in the
/// app's one formatter, the one the Keeping line and the file rows use. A selection of
/// empty files still says `0 B`: the clause is always there, so the button
/// keeps one shape and no missing figure reads as a size that failed — and the
/// files are still fetched, the press makes them.
fn footer_words(chosen: Ticked) -> String {
    format!(
        "Download {} · {}",
        thousands(chosen.files),
        format_size(chosen.bytes)
    )
}

/// The footer's selection: how many files are ticked and what they weigh.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Ticked {
    files: usize,
    bytes: u64,
}

/// Every ticked row among `rows`, which are all the loaded ones, counted and
/// summed in one pass so the count and the bytes cannot describe two sets.
fn chosen_among(rows: &[Row], picks: &[bool]) -> Ticked {
    rows.iter()
        .filter(|r| r.pick.is_some_and(|slot| picks[slot]))
        .fold(Ticked::default(), |sum, r| Ticked {
            files: sum.files + 1,
            bytes: sum.bytes.saturating_add(r.bytes),
        })
}

/// Everything here but two empty files a run leaves to mark it done, so the
/// only ticks there are weigh nothing.
fn empty_selection() -> Vec<File> {
    let mut files = downloaded_package();
    files.push(File::new("raw/plate-07.done", 0, Mark::Missing));
    files.push(File::new("raw/plate-08.done", 0, Mark::Missing));
    files
}

/// Everything here but a folder of 1,000 large plates this copy has not
/// fetched, which takes the package past the cap. Drawn through [`loaded`], so
/// only the plates among the first 1,000 paths can be ticked: 998 of them,
/// after `.DS_Store` and `README.md`.
fn heavy_selection() -> Vec<File> {
    let mut files = downloaded_package();
    for i in 1..=1_000 {
        files.push(File::new(
            format!("imaging/plate-{i:04}.tiff"),
            86_300_000,
            Mark::Missing,
        ));
    }
    files
}

const NOTE: &str = "The page's growing half, at the 700px a 1024 window gives it. Tick a \
    row: the footer arrives and the list goes 313px to 264, measured — the card stays 315 \
    either way, so the pane never changes height. Type in the search or pick a facet: \
    select-all states its own extent, and under `Changed` it goes, having nothing to tick. \
    The marked rows draw ahead of their data. Unresolved and visible: under `Group: None` \
    the ellipsis eats the leaf, kept for now as a deliberate simplification. A file that is \
    here carries a muted check in the box column, in every scope, so the column fills up as \
    files land, and a folder whose files are all here carries it on its heading. Under \
    whole-package Keeping every file is downloaded, so the slot select-all leaves reads \
    `All 53 files downloaded` behind the Success tone's tick; with a file deleted here it reads \
    `All 53 files downloaded · 1 deleted here`, with no tick. The last four cells are the \
    page's own pane: over this fixture, over it grown past the cap, with every file \
    downloaded, and with one deleted here.";

/// The region itself, for the whole-page scene.
///
/// `fill` is the difference between the two places it is drawn: the cells below
/// cap the list at §5's own numbers because those numbers are what they are
/// about, and the page hands it whatever height the window left.
#[component]
pub fn FilePaneRegion(
    /// The radio group's name, unique per cell on the page.
    name: &'static str,
    /// How many rows start ticked, which is what puts the footer on screen.
    #[prop(optional)]
    ticked: usize,
    /// Resolve mode: mark the files that differ between the two revisions.
    #[prop(optional)]
    marked: bool,
    /// `Keeping → The whole package`, which takes the per-file choice away.
    #[prop(optional)]
    whole: bool,
    /// The package is behind, and this is what the dry run has said so far.
    /// The UI's own `PullCheck`, so the page can hand the pane what it reads.
    #[prop(optional)]
    incoming: Option<PullCheck>,
    /// Where the incoming rows sit.
    #[prop(optional)]
    placement: Placement,
    /// No local changes: the package a `CleanUpdate` is about.
    #[prop(optional)]
    clean: bool,
) -> impl IntoView {
    pane(Pane {
        ticked,
        whole,
        incoming,
        placement,
        files: if clean { settled_package() } else { package() },
        marked: if marked { MARKED } else { &[] },
        framing: Framing::Page,
        ..Pane::new(name)
    })
}

#[component]
#[allow(clippy::too_many_lines, reason = "one cell per state, read as a list")]
pub fn FilePaneScene() -> impl IntoView {
    // The standalone trigger has no row behind it, so it names one.
    let keeping = Confirm::new("raw/plate-03.csv (4.2\u{a0}MB)");
    let replacing = RwSignal::new(false);

    view! {
        <Scene title="The file pane" note=NOTE>
            <Cell full=true label="at rest — nothing ticked, so no footer and 313px of list">
                {pane(Pane::new("fp-rest"))}
            </Cell>
            <Cell
                full=true
                label="three ticked — the footer costs a row and a half of the list, and says the \
                       bytes: the page sums EntryData.size over the ticked rows, in the memo that \
                       counts them; no I/O"
            >
                {pane(Pane { ticked: 3, ..Pane::new("fp-selecting") })}
            </Cell>
            <Cell full=true label="downloading — the footer's button carries it, the rows do not">
                {pane(Pane {
                    ticked: 3,
                    running: true,
                    ..Pane::new("fp-running")
                })}
            </Cell>
            <Cell
                full=true
                label="two empty files ticked — `0 B` stays, so the button keeps one shape and no \
                       missing figure reads as a failure"
            >
                {pane(Pane {
                    files: empty_selection(),
                    ticked: 2,
                    ..Pane::new("fp-weightless")
                })}
            </Cell>
            <Cell
                full=true
                label="a huge selection, limited by the 1,000 loaded rows — 1,000 plates are \
                       missing, but only the 998 among the first 1,000 paths can be ticked"
            >
                {
                    let (files, total) = loaded(heavy_selection());
                    pane(Pane {
                        files,
                        ticked: LOADED,
                        body: Body::Capped(total),
                        ..Pane::new("fp-heavy")
                    })
                }
            </Cell>
            <Cell full=true label="resolve mode — the two files that differ, marked in place">
                <Provider value=DiffersId("resolve-differing-file-pane")>
                    {pane(Pane { marked: MARKED, ..Pane::new("fp-marked") })}
                    {differs_caption(MARKED.len())}
                </Provider>
            </Cell>
            <Cell full=true label="Keeping → the whole package, all downloaded: no boxes, no footer, and the slot says so">
                {pane(Pane { whole: true, files: downloaded_package(), ..Pane::new("fp-whole") })}
            </Cell>
            <Cell full=true label="all downloaded, one file deleted here — the line names it, and the tick goes">
                {pane(Pane { files: deleted_here_package(), ..Pane::new("fp-deleted-here") })}
            </Cell>
            <Cell full=true label="the Changed facet — nothing here can be ticked, so select-all goes">
                {pane(Pane { facet: "Changed", ..Pane::new("fp-changed") })}
            </Cell>
            <Cell full=true label="the Ignored facet — three files, two headings, one name between them">
                {pane(Pane { facet: "Ignored", ..Pane::new("fp-ignored") })}
            </Cell>
            <Cell full=true label="a clean copy — `Changed 0` stays in place, inert">
                {pane(Pane { files: settled_package(), ..Pane::new("fp-settled") })}
            </Cell>
            <Cell full=true label="Group: None — the gutter goes to zero, and the ellipsis eats the leaf">
                {pane(Pane { grouped: false, ..Pane::new("fp-flat") })}
            </Cell>
            <Cell full=true label="a search and a facet that leave nothing — compact, and no way out">
                {pane(Pane {
                    facet: "Not downloaded",
                    query: "kickoff",
                    ..Pane::new("fp-narrowed")
                })}
            </Cell>
            <Cell full=true label="a package with no files at all">
                {pane(Pane { files: Vec::new(), ..Pane::new("fp-empty") })}
            </Cell>
            <Cell full=true label="loading — and the toolbar cannot outlive its counts">
                {pane(Pane { body: Body::Loading, ..Pane::new("fp-loading") })}
            </Cell>
            <Cell full=true label="over the cap — and the toolbar's counts openly disagree with it">
                {pane(Pane { body: Body::Capped(4_312), ..Pane::new("fp-capped") })}
            </Cell>
            <Cell full=true label="the read failed — the controls stay, the box carries it">
                {pane(Pane { body: Body::Failed, ..Pane::new("fp-failed") })}
            </Cell>
            <Cell full=true label="both confirms, opened here directly — `Stop keeping` is also on every row's menu">
                <div class="g-inline">
                    <Button on_click=move |_| keeping.open.set(true)>"Stop keeping"</Button>
                    <Button on_click=move |_| replacing.set(true)>"Replace mine"</Button>
                </div>
                {stop_keeping(keeping)}
                {replace_mine(replacing)}
            </Cell>
            <Cell full=true label="live — the page's own pane over this fixture, grouping only so far">
                <div style=PANE>{live(entry_list(package()))}</div>
            </Cell>
            <Cell full=true label="live, over the cap — 1,089 files, the first 1,000 by path loaded">
                <div style=PANE>{live(entry_list(over_the_cap()))}</div>
            </Cell>
            <Cell full=true label="the page's pane, every file downloaded — the caption and its tick, as the page draws them">
                <div style=PANE>{live(entry_list(downloaded_package()))}</div>
            </Cell>
            <Cell full=true label="the page's pane, one file deleted here — the line names it, and no tick">
                <div style=PANE>{live(entry_list(deleted_here_package()))}</div>
            </Cell>
        </Scene>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::*;

    /// The line says what the dry run found, in the scope's own terms, and a
    /// revision that adds nothing and keeps nothing gets no line at all.
    #[test]
    fn the_incoming_line_says_what_the_check_found() {
        let ready = |outcome, added: &[&str]| {
            PullCheck::Ready(PullPreview {
                outcome,
                added: added.iter().map(ToString::to_string).collect(),
            })
        };
        let keeps = || PullOutcome::KeepsLocalChanges {
            added: Vec::new(),
            modified: vec!["a.csv".to_string()],
            removed: Vec::new(),
        };
        let cases: Vec<(PullCheck, bool, Option<&str>)> = vec![
            (
                PullCheck::Loading,
                false,
                Some("Checking which files the newer revision brings\u{2026}"),
            ),
            (
                PullCheck::Failed,
                false,
                Some("Couldn't check which files the newer revision brings."),
            ),
            (
                ready(PullOutcome::CleanUpdate, &["a", "b", "c"]),
                false,
                Some(
                    "The newer revision brings 3 files. Get latest lists them here, to \
                     download when you need them.",
                ),
            ),
            (
                ready(PullOutcome::CleanUpdate, &["a"]),
                true,
                Some("The newer revision brings 1 file. Get latest downloads it."),
            ),
            (
                ready(keeps(), &["a", "b"]),
                true,
                Some(
                    "The newer revision brings 2 files. Get latest downloads them. Your changes stay.",
                ),
            ),
            (
                ready(keeps(), &[]),
                false,
                Some("Get latest keeps your changes."),
            ),
            (ready(PullOutcome::CleanUpdate, &[]), false, None),
            (
                ready(
                    PullOutcome::Blocked {
                        conflicts: vec!["a".to_string(), "b".to_string()],
                    },
                    &["c"],
                ),
                false,
                Some(
                    "2 files changed here and in the newer revision. Publish your changes, \
                     then resolve them.",
                ),
            ),
        ];
        for (check, whole, expected) in cases {
            assert_eq!(
                incoming_words(&check, whole).as_deref(),
                expected,
                "{check:?}, whole: {whole}"
            );
        }
    }

    /// The v2 vocabulary holds on the new line too: none of the words the
    /// package states keep out.
    #[test]
    fn the_incoming_line_uses_no_banned_word() {
        const BANNED: &[&str] = &[
            "commit", "push", "pull", "remote", "behind", "ahead", "diverged", "dirty", "hash",
        ];
        let added = vec!["a".to_string(), "b".to_string()];
        let checks = [
            PullCheck::Loading,
            PullCheck::Failed,
            PullCheck::Ready(PullPreview {
                outcome: PullOutcome::CleanUpdate,
                added: added.clone(),
            }),
            PullCheck::Ready(PullPreview {
                outcome: PullOutcome::KeepsLocalChanges {
                    added: Vec::new(),
                    modified: Vec::new(),
                    removed: Vec::new(),
                },
                added: added.clone(),
            }),
            PullCheck::Ready(PullPreview {
                outcome: PullOutcome::Blocked {
                    conflicts: added.clone(),
                },
                added,
            }),
        ];
        for check in &checks {
            for whole in [false, true] {
                let words = incoming_words(check, whole)
                    .unwrap_or_default()
                    .to_lowercase();
                for bad in BANNED {
                    assert!(
                        !words
                            .split_whitespace()
                            .any(|w| w.trim_matches(|c: char| !c.is_alphanumeric()) == *bad),
                        "{words:?} contains the banned word {bad:?}"
                    );
                }
            }
        }
    }

    /// An added path the copy already has a row for keeps its one row.
    #[test]
    fn an_added_path_with_a_row_is_not_drawn_twice() {
        let check = PullCheck::Ready(PullPreview {
            outcome: PullOutcome::CleanUpdate,
            added: vec![
                "notes/plate-07-rerun.md".to_string(),
                "qc/flags.json".to_string(),
            ],
        });
        let files = with_incoming(package(), Some(&check));
        let incoming: Vec<&str> = files
            .iter()
            .filter(|f| f.mark == Mark::Incoming)
            .map(|f| f.path.as_str())
            .collect();
        assert_eq!(incoming, vec!["qc/flags.json"]);
        // And the facets still count the installed revision.
        assert_eq!(facet_labels(&files)[0].1, facet_labels(&package())[0].1);
    }

    /// The footer says what the press fetches, in files and bytes.
    #[test]
    fn the_footer_says_its_bytes() {
        let plain = |w: String| w.replace('\u{a0}', " ");
        let words = |files, bytes| plain(footer_words(Ticked { files, bytes }));
        assert_eq!(words(3, 1_200_000), "Download 3 · 1.2 MB");
        assert_eq!(words(2, 0), "Download 2 · 0 B");
        assert_eq!(words(1_000, 86_300_000_000), "Download 1,000 · 86.3 GB");
    }

    /// The footer's count and bytes are one set: every loaded tick, the rows a
    /// search or a facet hides included, and nothing that is not ticked.
    #[test]
    fn the_footers_count_and_bytes_are_one_set() {
        let all = rows(&package());
        let slots = all.iter().filter(|r| r.pick.is_some()).count();
        let mut picks = vec![false; slots];
        // A tick on a `raw/` plate and one on `manifest.jsonl`, which a search
        // for "plate" hides.
        let manifest = all.iter().find(|r| r.path == "manifest.jsonl").unwrap();
        let plate = all.iter().find(|r| r.path == "raw/plate-23.csv").unwrap();
        picks[manifest.pick.unwrap()] = true;
        picks[plate.pick.unwrap()] = true;

        let chosen = chosen_among(&all, &picks);
        assert_eq!(chosen.files, 2);
        assert_eq!(chosen.bytes, manifest.bytes + plate.bytes);

        // Whatever is ticked, the bytes are the sum over exactly the rows the
        // count counts.
        for n in 0..=slots {
            let picks: Vec<bool> = (0..slots).map(|i| i < n).collect();
            let chosen = chosen_among(&all, &picks);
            let set: Vec<&Row> = all
                .iter()
                .filter(|r| r.pick.is_some_and(|s| picks[s]))
                .collect();
            assert_eq!(chosen.files, set.len());
            assert_eq!(chosen.bytes, set.iter().map(|r| r.bytes).sum::<u64>());
        }
    }

    /// The huge selection honours the cap: no more than [`LOADED`] rows, so
    /// fewer ticks than the package has missing files.
    #[test]
    fn the_huge_selection_is_limited_by_the_loaded_rows() {
        let (files, total) = loaded(heavy_selection());
        assert_eq!((files.len(), total), (LOADED, 1_056));
        let all = rows(&files);
        let picks = vec![true; all.iter().filter(|r| r.pick.is_some()).count()];
        let chosen = chosen_among(&all, &picks);
        assert_eq!(chosen.files, 998);
        assert_eq!(
            footer_words(chosen).replace('\u{a0}', " "),
            "Download 998 · 86.1 GB"
        );
    }

    /// The live cell over the cap says what the page would: the whole count,
    /// and that the loaded rows are the first 1,000 by path.
    #[wasm_bindgen_test]
    fn the_live_cell_over_the_cap_states_the_cap() {
        let list = entry_list(over_the_cap());
        assert_eq!((list.entries.len(), list.total), (1_000, 1_089));
        assert!(list.truncated);
        assert_eq!(list.counts.all + list.counts.ignored, 1_089);

        let doc = web_sys::window().unwrap().document().unwrap();
        let el: web_sys::HtmlElement = doc.create_element("div").unwrap().dyn_into().unwrap();
        doc.body().unwrap().append_child(&el).unwrap();
        let handle = leptos::mount::mount_to(el.clone(), move || live(list));
        let text = el.text_content().unwrap_or_default();
        drop(handle);
        el.remove();
        assert!(
            text.contains(
                "This package has 1,089 files. This list covers the first 1,000 by path."
            ),
            "text was {text}"
        );
    }
}
