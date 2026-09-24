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
//! One cell states a fact the backend cannot currently produce, deliberately, so
//! that the copy is the target rather than a guess made later:
//!
//! - **The marked rows.** Which files differ between two diverged revisions is
//!   computed nowhere — `MergeData` carries a namespace and a URI, and the
//!   working-tree changes are local edits against the installed manifest, not
//!   mine against published (`qhq-mrzt`).
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

use leptos::prelude::*;

use crate::Cell;
use crate::Scene;
use crate::differs_caption;
use crate::kit::Blankslate;
use crate::kit::Button;
use crate::kit::ButtonVariant;
use crate::kit::Card;
use crate::kit::CheckState;
use crate::kit::Dialog;
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
use quilt_sync_ui::util::format_size;

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
        File::new("notes/ernest-thread.md", 12_800, Mark::Changed),
        File::new("notes/caihong-upload.md", 8_200, Mark::Changed),
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
            let n = files.iter().filter(|f| (facet.admits)(f.mark)).count();
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
                size: format_size(f.bytes),
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
    let actions = menu(r.mark, format!("{} ({})", r.path, r.size), confirm);

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
    /// The rows, under the sentence saying they are not all of them.
    Capped,
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
    /// no boxes, no select-all, and the footer can never appear.
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
        }
    }
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
    } = p;

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
    let confirm = Confirm::new("raw/plate-03.csv (4.16 MB)");

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
    let chosen = Signal::derive(move || {
        all.with_value(|rs| {
            shown
                .get()
                .iter()
                .filter(|&&i| rs[i].pick.is_some_and(|slot| picks.with(|p| p[slot])))
                .count()
        })
    });
    let narrowed =
        Signal::derive(move || !query_sig.get().is_empty() || !facet_sig.get().starts_with("All"));

    let fill = framing == Framing::Page;
    let boxes = !whole;
    let footer_shown = Signal::derive(move || boxes && chosen.get() > 0);
    let marked: Vec<String> = marked.iter().map(|&p| p.to_string()).collect();
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

            for i in indices {
                match (&rs[i].folder, flat) {
                    (Some(folder), false) => match groups.last_mut() {
                        Some((name, members)) if name == folder => members.push(i),
                        _ => groups.push((folder.clone(), vec![i])),
                    },
                    _ => roots.push(draw(&rs[i], flat, picks, marked, boxes, confirm)),
                }
            }

            view! {
                {roots}
                {groups
                    .into_iter()
                    .map(|(name, members)| group(name, members, all, picks, marked, boxes, confirm))
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
                            // the pane's edge.
                            <Show when=move || boxes && (offered.get() > 0)>
                                <span style="flex:0 0 28px" />
                                <SelectAll
                                    selected=chosen
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
                            {(body == Body::Capped)
                                .then(|| {
                                    view! {
                                        // The first 1,000 by path: the page read
                                        // sorts before it caps and sends the
                                        // total (quilt-rs#992).
                                        <p style="margin:0; padding:var(--q-space-2) \
                                                  var(--q-space-3); \
                                                  color:var(--q-fgColor-muted); \
                                                  font-size:var(--q-text-body)">
                                            "This package has 4,312 files. Showing the first 1,000."
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
                                                (false, Body::Capped) => LIST_UNDER_NOTICE,
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
                            {move || format!("Download {}", chosen.get())}
                        </Button>
                    </div>
                </Show>
            </Card>
            {stop_keeping(confirm)}
        </div>
    }
    .into_any()
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

/// A heading and its rows. The heading's box is derived from the rows under it
/// and toggles exactly those, so `Mixed` is a fact about them rather than an
/// assertion beside them — and a group with nothing selectable carries no box,
/// because it would be a control with nothing to act on.
fn group(
    name: String,
    members: Vec<usize>,
    all: StoredValue<Vec<Row>>,
    picks: RwSignal<Vec<bool>>,
    marked: StoredValue<Vec<String>>,
    boxes: bool,
    confirm: Confirm,
) -> AnyView {
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
            <EntryGroup name=name count=Signal::derive(move || count) open=open>
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
/// The sentence verdict 8 requires is the one thing here that has no source:
/// the revision count needs `list_revisions` and the file count needs a
/// row-by-row comparison of two diverged manifests, which is computed nowhere
/// (`qhq-mrzt`). Drawn anyway, with the numbers a fixture, because a dialog that
/// cannot count is a dialog that lies and this is what the backend has to grow.
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

/// The page's `FilePane`, fed this scene's package as the page read would send
/// it: sorted by path, ignored files included for the pane to hide.
fn live() -> AnyView {
    let mut files = package();
    files.sort_by(|a, b| a.path.cmp(&b.path));
    let entries: Vec<crate::commands::EntryData> = files
        .into_iter()
        .map(|f| crate::commands::EntryData {
            status: match f.mark {
                Mark::Here | Mark::Ignored => "pristine",
                Mark::Changed => "modified",
                Mark::New => "added",
                Mark::Deleted => "deleted",
                Mark::Missing => "remote",
            }
            .to_string(),
            ignored_by: (f.mark == Mark::Ignored).then(|| ".DS_Store".to_string()),
            filename: f.path,
            size: f.bytes,
            junky_pattern: None,
            namespace: "team/dataset".try_into().expect("a namespace"),
        })
        .collect();
    let total = entries.len();
    view! {
        <div style=format!("display:flex; flex-direction:column; {LIST_RESTING}; height:400px")>
            <crate::pages::FilePane
                listing=Signal::stored(crate::pages::Listing::Ready(crate::pages::FileList {
                    entries,
                    total,
                    truncated: false,
                }))
                grouping=RwSignal::new(crate::pages::Grouping::BaseFolder.label().to_string())
                on_open=Callback::new(|_: String| ())
                on_retry=Callback::new(|()| ())
            />
        </div>
    }
    .into_any()
}

const NOTE: &str = "The page's growing half, at the 700px a 1024 window gives it. Tick a \
    row: the footer arrives and the list goes 313px to 264, measured — the card stays 315 \
    either way, so the pane never changes height. Type in the search or pick a facet: \
    select-all states its own extent, and under `Changed` it goes, having nothing to tick. \
    The marked rows draw ahead of their data. Unresolved and visible: under `Group: None` \
    the ellipsis eats the leaf, kept for now as a deliberate simplification. The last cell \
    is the page's own pane over this fixture.";

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
) -> impl IntoView {
    pane(Pane {
        ticked,
        whole,
        marked: if marked { MARKED } else { &[] },
        framing: Framing::Page,
        ..Pane::new(name)
    })
}

#[component]
pub fn FilePaneScene() -> impl IntoView {
    // The standalone trigger has no row behind it, so it names one.
    let keeping = Confirm::new("raw/plate-03.csv (4.16 MB)");
    let replacing = RwSignal::new(false);

    view! {
        <Scene title="The file pane" note=NOTE>
            <Cell full=true label="at rest — nothing ticked, so no footer and 313px of list">
                {pane(Pane::new("fp-rest"))}
            </Cell>
            <Cell full=true label="three ticked — the footer costs a row and a half of the list">
                {pane(Pane { ticked: 3, ..Pane::new("fp-selecting") })}
            </Cell>
            <Cell full=true label="downloading — the footer's button carries it, the rows do not">
                {pane(Pane {
                    ticked: 3,
                    running: true,
                    ..Pane::new("fp-running")
                })}
            </Cell>
            <Cell full=true label="resolve mode — the two files that differ, marked in place">
                {pane(Pane { marked: MARKED, ..Pane::new("fp-marked") })}
                {differs_caption(MARKED.len())}
            </Cell>
            <Cell full=true label="Keeping → the whole package: no boxes, no select-all, no footer">
                {pane(Pane { whole: true, ..Pane::new("fp-whole") })}
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
                    query: "ernest",
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
                {pane(Pane { body: Body::Capped, ..Pane::new("fp-capped") })}
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
                <div style=PANE>{live()}</div>
            </Cell>
        </Scene>
    }
}
