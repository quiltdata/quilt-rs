//! The package file pane: the list of the package's files.
//!
//! The region beside the context pane, and the page's growing half. Built from
//! the kit the gallery's file-pane scene composes — `EntryRow`, `EntryGroup`,
//! `ListToolbar`, `Select`, `Card` — so the scene stays the design record and
//! this is the same arrangement with the page's data in it.
//!
//! # The frame has slots the later slices fill
//!
//! Search takes a row of its own above the toolbar; select-all sits on the
//! toolbar's left, in the rows' checkbox column; grouping and the facets are one
//! right-hand group. Search, grouping, the facets and select-all are drawn. The other
//! slots are left absent rather than drawn inert — a control that does nothing is
//! worse than one that is not there yet.
//!
//! # What this slice draws
//!
//! - **Rows, silent at rest.** `Downloaded` carries no label; `Changed`, `New`,
//!   `Deleted` and `Not downloaded` do (design §3's table).
//! - **The facets filter by state.** `All · Changed · Not downloaded ·
//!   Ignored`, each with the package's count (the payload's `counts`, not the
//!   loaded rows). A facet at zero stays in place and cannot be chosen, except
//!   `All`, the home a facet that empties falls back to. `All` leaves ignored
//!   files out; only `Ignored` shows them, with no label and no box, since the
//!   view already says it and an ignored file is never selectable. A tracked
//!   file that is also ignored is two rows, as in v1, and counts under both.
//! - **A click does the thing the row can do.** A file that is here opens, a
//!   missing one is `EntryRow`'s `Select` shape, and a deleted one is inert.
//! - **Selection feeds one action.** Select-all, the headings' boxes and the
//!   footer's `[Download]` read one ticked set the page owns ([`selection`]).
//!   Under whole-package Keeping none of them is drawn.
//! - **The cap is stated.** Over the cap the backend says so, and the pane
//!   says how many files the package has. The flag decides, never a length.
//! - **Search narrows what is shown.** A case-insensitive substring of the
//!   whole path; headings are drawn from the rows it leaves, and a search that
//!   leaves none says so.

use std::collections::BTreeSet;

use leptos::prelude::*;

use crate::commands::{EntryCounts, EntryData, EntryList, FilesData};
use crate::kit::state_label::StateTone;
use crate::kit::{
    Blankslate, Button, ButtonVariant, Card, EntryAction, EntryGroup, EntryRow, EntrySelection,
    GroupSelection, ListToolbar, LoadFailure, Naming, SearchInput, Segment, SegmentedControl,
    Select, SelectAll, SkeletonBox,
};
use crate::util::format_size;

pub(crate) mod selection;

pub use selection::Picking;

stylance::import_crate_style!(
    style,
    "src/pages/installed_package_v2/file_pane.module.scss"
);

/// How the list is grouped. Not remembered: every visit starts at
/// [`Grouping::BaseFolder`], because which axis is useful depends on the shape
/// of the package in front of the reader.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grouping {
    /// The folder the file is in. The default: it always reflects real
    /// structure, and its failure mode — many small groups — is visible.
    BaseFolder,
    /// The first segment of the path.
    TopFolder,
    /// Flat, full paths.
    None,
}

impl Grouping {
    const ALL: [Self; 3] = [Self::BaseFolder, Self::TopFolder, Self::None];

    /// The words on the `Group:` select.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::BaseFolder => "Base folder",
            Self::TopFolder => "Top folder",
            Self::None => "None",
        }
    }

    fn from_label(label: &str) -> Self {
        Self::ALL
            .into_iter()
            .find(|g| g.label() == label)
            .unwrap_or(Self::BaseFolder)
    }
}

/// Which files the list shows, by where they stand. One choice of four, and
/// the pane's only filter.
///
/// Held by the page as its [`Facet::key`], the words without the count: the
/// count moves when the package does, and a choice held as the words would
/// then match no segment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Facet {
    /// Every file that is not ignored.
    All,
    /// Added, modified or deleted.
    Changed,
    /// In the revision and not on disk.
    NotDownloaded,
    /// Matched by `.quiltignore`. The only view that shows them.
    Ignored,
}

impl Facet {
    const ALL: [Self; 4] = [Self::All, Self::Changed, Self::NotDownloaded, Self::Ignored];

    /// The segment's words without its count, and the value the page holds.
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Changed => "Changed",
            Self::NotDownloaded => "Not downloaded",
            Self::Ignored => "Ignored",
        }
    }

    fn from_key(key: &str) -> Self {
        Self::ALL
            .into_iter()
            .find(|f| f.key() == key)
            .unwrap_or(Self::All)
    }

    /// The package's count for this facet: the whole package, not the rows
    /// that were loaded.
    const fn count(self, counts: &EntryCounts) -> usize {
        match self {
            Self::All => counts.all,
            Self::Changed => counts.changed,
            Self::NotDownloaded => counts.not_downloaded,
            Self::Ignored => counts.ignored,
        }
    }

    /// Whether a row belongs in this view. The backend's rule for the counts
    /// (`EntryCounts::of`), so a facet shows the rows its count counted.
    const fn admits(self, place: Place, ignored: bool) -> bool {
        match self {
            Self::Ignored => ignored,
            _ if ignored => false,
            Self::All => true,
            Self::Changed => matches!(place, Place::Changed | Place::New | Place::Deleted),
            Self::NotDownloaded => matches!(place, Place::Missing),
        }
    }

    /// Nothing in the package for this view, so it cannot be chosen. Never
    /// `All`, which stays choosable at zero.
    fn is_empty(self, counts: &EntryCounts) -> bool {
        self != Self::All && self.count(counts) == 0
    }

    /// `Not downloaded 17`.
    fn words(self, counts: &EntryCounts) -> String {
        format!("{} {}", self.key(), thousands(self.count(counts)))
    }
}

/// One thing the list draws: a row on its own, or a heading over its rows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Item {
    Row {
        index: usize,
        name: String,
    },
    Group {
        heading: String,
        rows: Vec<(usize, String)>,
    },
}

/// Group paths, given in path order, for drawing.
///
/// Root files come first and carry no heading: `(root)` would name a folder
/// that does not exist. A group of one carries no heading either — it would be
/// mostly heading — so its row stands alone and shows its whole path. Groups
/// keep the order of their first file, so the list still reads in path order
/// even where a subfolder's paths sort between a folder's own files.
#[must_use]
pub fn group(paths: &[&str], grouping: Grouping) -> Vec<Item> {
    let row = |index: usize| Item::Row {
        index,
        name: paths[index].to_string(),
    };
    // Where the heading ends and the row's own name starts.
    let cut = |path: &str| match grouping {
        Grouping::BaseFolder => path.rfind('/'),
        Grouping::TopFolder => path.find('/'),
        Grouping::None => None,
    };

    let mut roots = Vec::new();
    let mut groups: Vec<(&str, Vec<usize>)> = Vec::new();
    for (index, path) in paths.iter().enumerate() {
        match cut(path) {
            None => roots.push(row(index)),
            Some(at) => {
                let heading = &path[..=at];
                match groups.iter_mut().find(|(h, _)| *h == heading) {
                    Some((_, members)) => members.push(index),
                    None => groups.push((heading, vec![index])),
                }
            }
        }
    }

    roots
        .into_iter()
        .chain(groups.into_iter().map(|(heading, members)| {
            if let [only] = members[..] {
                return row(only);
            }
            Item::Group {
                heading: heading.to_string(),
                rows: members
                    .into_iter()
                    .map(|i| (i, paths[i][heading.len()..].to_string()))
                    .collect(),
            }
        }))
        .collect()
}

/// Whether the search keeps a file: a case-insensitive substring of its whole
/// logical path, so a folder's name finds the files under it. An empty search
/// keeps everything.
#[must_use]
pub fn path_matches(path: &str, query: &str) -> bool {
    path.to_lowercase().contains(&query.to_lowercase())
}

/// What the pane draws from: the page read's rows, and whether it cut them.
#[derive(Clone, Debug)]
pub struct FileList {
    /// Sorted by path, then capped.
    pub entries: Vec<EntryData>,
    /// The facets' counts, over the whole package.
    pub counts: EntryCounts,
    /// Every file the package has, before the cap.
    pub total: usize,
    /// The backend cut the list. Never inferred from `entries.len()`.
    pub truncated: bool,
}

impl From<EntryList> for FileList {
    fn from(list: EntryList) -> Self {
        Self {
            entries: list.entries,
            counts: list.counts,
            total: list.total,
            truncated: list.truncated,
        }
    }
}

/// Where the pane's list stands.
#[derive(Clone, Debug)]
pub enum Listing {
    Loading,
    /// The page read sent no list, and why: not even a local status could be
    /// computed, so there is nothing honest to classify the rows by.
    Unlisted(String),
    Ready(FileList),
}

impl From<FilesData> for Listing {
    fn from(files: FilesData) -> Self {
        match files {
            FilesData::Listed(list) => Self::Ready(list.into()),
            FilesData::Unlisted { reason } => Self::Unlisted(reason),
        }
    }
}

/// Where a file is, which decides its words and its click.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Place {
    /// Here and unchanged. Silent.
    Here,
    Changed,
    New,
    /// In the revision, gone from disk: nothing to open.
    Deleted,
    /// Not downloaded.
    Missing,
    /// A status this page does not know. Drawn, silent and inert, rather than
    /// guessed at.
    Unknown,
}

impl Place {
    fn of(status: &str) -> Self {
        match status {
            "pristine" => Self::Here,
            "modified" => Self::Changed,
            "added" => Self::New,
            "deleted" => Self::Deleted,
            "remote" => Self::Missing,
            _ => Self::Unknown,
        }
    }

    const fn state(self) -> Option<(&'static str, StateTone)> {
        match self {
            Self::Here | Self::Unknown => None,
            Self::Changed => Some(("Changed", StateTone::Attention)),
            Self::New => Some(("New", StateTone::Attention)),
            Self::Deleted => Some(("Deleted", StateTone::Danger)),
            Self::Missing => Some(("Not downloaded", StateTone::Neutral)),
        }
    }
}

/// `4312` as `4,312`.
fn thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// The one line saying the list is not all of the package.
///
/// It describes the loaded list, not the view: a facet or a search narrows the
/// rows further, and the notice still speaks of the list they are drawn from.
///
/// Page-local and not a kit piece: not a `Banner`, which reports an outcome
/// for the whole page, and not a `ZeroLine`, which is the healthy queue's
/// one-liner. It sits inside the list's box, above the rows it qualifies.
#[component]
fn CapNotice(total: usize, shown: usize) -> impl IntoView {
    view! {
        <p class=style::cap>
            {format!(
                "This package has {} files. This list covers the first {} by path.",
                thousands(total),
                thousands(shown),
            )}
        </p>
    }
}

/// One file, reduced to what drawing it needs.
#[derive(Clone, PartialEq, Eq)]
struct Row {
    path: String,
    size: String,
    place: Place,
    /// Matched by `.quiltignore`. Drawn only under [`Facet::Ignored`], where
    /// the view already says it, so the row carries no label and no click.
    ignored: bool,
}

impl Row {
    /// Whether a box can tick it: not downloaded, and not ignored — an ignored
    /// file is stopped being ignored before it is fetched.
    fn selectable(&self) -> bool {
        self.place == Place::Missing && !self.ignored
    }
}

/// The paths a box could tick among `rows`, in path order.
fn offered(rows: &[Row]) -> Vec<String> {
    rows.iter()
        .filter(|r| r.selectable())
        .map(|r| r.path.clone())
        .collect()
}

/// One row. `name` is what the list shows — the whole path, or the part under
/// a heading — and the whole path is always the `title`.
fn row(r: &Row, name: String, on_open: Callback<String>, picking: Picking) -> AnyView {
    let state = if r.ignored { None } else { r.place.state() };
    let words = state.map(|(words, _)| words.to_string());
    let tone = state.map_or(StateTone::Neutral, |(_, tone)| tone);
    let action = match r.place {
        _ if r.ignored => None,
        Place::Here | Place::Changed | Place::New => {
            let path = r.path.clone();
            Some(EntryAction::Open(Callback::new(move |()| {
                on_open.run(path.clone());
            })))
        }
        Place::Missing if r.selectable() && !picking.whole_package => {
            let ticked = picking.ticked;
            let (is, set) = (r.path.clone(), r.path.clone());
            Some(EntryAction::Select(EntrySelection::new(
                Signal::derive(move || ticked.with(|t| t.contains(&is))),
                Callback::new(move |next| {
                    ticked.update(|t| selection::tick_all(t, std::slice::from_ref(&set), next));
                }),
            )))
        }
        // Nothing to open, and nothing to tick: ignored, or the whole package
        // is kept and the scope fetches it.
        Place::Missing | Place::Deleted | Place::Unknown => None,
    };
    match action {
        Some(action) => view! {
            <EntryRow
                name=name
                path=r.path.clone()
                state=words
                tone=tone
                size=r.size.clone()
                action=action
            />
        }
        .into_any(),
        None => view! {
            <EntryRow name=name path=r.path.clone() state=words tone=tone size=r.size.clone() />
        }
        .into_any(),
    }
}

/// The rows, grouped as the select says. `collapsed` names the headings the
/// reader closed; it is the page's, so a re-read that rebuilds these views
/// finds each folder as the reader left it.
fn rows_view(
    rows: &[Row],
    grouping: Grouping,
    collapsed: RwSignal<BTreeSet<String>>,
    on_open: Callback<String>,
    picking: Picking,
) -> AnyView {
    let paths: Vec<&str> = rows.iter().map(|r| r.path.as_str()).collect();
    let items = group(&paths, grouping);
    // A list with no heading at all has no disclosure to align to, so the
    // gutter the rows keep for one is zero (`EntryRow`'s module doc).
    let headed = items.iter().any(|i| matches!(i, Item::Group { .. }));
    let drawn = items
        .into_iter()
        .map(|item| match item {
            Item::Row { index, name } => row(&rows[index], name, on_open, picking),
            Item::Group {
                heading,
                rows: members,
            } => {
                let count = members.len();
                // A collapsed group renders nothing, so the heading rebuilds its
                // rows each time it opens: it keeps the rows, not their views.
                let members: Vec<(Row, String)> = members
                    .into_iter()
                    .map(|(index, name)| (rows[index].clone(), name))
                    .collect();
                let pickable: Vec<Row> = members.iter().map(|(r, _)| r.clone()).collect();
                let pickable = offered(&pickable);
                let members = StoredValue::new(members);
                let open = RwSignal::new(!collapsed.with_untracked(|c| c.contains(&heading)));
                let folder = heading.clone();
                Effect::new(move |_| {
                    let open = open.get();
                    collapsed.update(|c| {
                        if open {
                            c.remove(&folder);
                        } else {
                            c.insert(folder.clone());
                        }
                    });
                });
                let children = move || {
                    members.with_value(|ms| {
                        ms.iter()
                            .map(|(r, name)| row(r, name.clone(), on_open, picking))
                            .collect_view()
                    })
                };
                match group_selection(pickable, picking) {
                    Some(selection) => view! {
                        <EntryGroup
                            name=heading
                            count=Signal::stored(count)
                            open=open
                            selection=selection
                        >
                            {children}
                        </EntryGroup>
                    }
                    .into_any(),
                    None => view! {
                        <EntryGroup name=heading count=Signal::stored(count) open=open>
                            {children}
                        </EntryGroup>
                    }
                    .into_any(),
                }
            }
        })
        .collect_view();
    view! {
        <div
            class=style::list
            style=format!("--q-entry-gutter:{}", if headed { "16px" } else { "0" })
        >
            {drawn}
        </div>
    }
    .into_any()
}

/// The rows the facet and the search leave, in path order. The one place the
/// pane narrows its rows, so what the list draws and what select-all ticks
/// cannot disagree.
fn shown_rows(rows: &[Row], query: &str, facet: Facet) -> Vec<Row> {
    rows.iter()
        .filter(|r| facet.admits(r.place, r.ignored) && path_matches(&r.path, query))
        .cloned()
        .collect()
}

/// The search, alone on the row above the toolbar and the pane's full width.
///
/// The `display:flex` row is load-bearing: `SearchInput` grows along its
/// parent's main axis, and dropped straight into the pane's column it would
/// grow tall rather than wide (the gallery's file-pane scene has the story).
fn search_row(search: RwSignal<String>) -> impl IntoView {
    view! {
        <div class=style::search>
            <SearchInput value=search aria_label="Search files" placeholder="Search files…" />
        </div>
    }
}

/// The four facets, each carrying the package's count.
///
/// A facet at zero stays in its place and cannot be chosen, except `All`: it
/// is the pane's home, and the view a facet that empties falls back to. The
/// fallback happens here, before the control is built, because an inert
/// segment must not hold the choice (`Segment::inert`).
fn facets(counts: &EntryCounts, facet: RwSignal<String>) -> impl IntoView {
    if Facet::from_key(&facet.get_untracked()).is_empty(counts) {
        facet.set(Facet::All.key().to_string());
    }
    let options = Facet::ALL
        .into_iter()
        .map(|f| {
            let words = f.words(counts);
            let segment = if f.is_empty(counts) {
                Segment::inert(words)
            } else {
                Segment::new(words)
            };
            segment.valued(f.key())
        })
        .collect();
    view! {
        <SegmentedControl
            aria_label="Show files"
            name="file-facets"
            options=options
            selected=facet
        />
    }
}

/// A heading's box: present when a row under it can be ticked, and never
/// under whole-package scope. It ticks exactly those rows, so `Mixed` is a fact
/// about them. `EntryGroup` names it `Select all in {heading}`.
fn group_selection(pickable: Vec<String>, picking: Picking) -> Option<GroupSelection> {
    if picking.whole_package || pickable.is_empty() {
        return None;
    }
    let ticked = picking.ticked;
    let members = StoredValue::new(pickable);
    Some(GroupSelection::new(
        Signal::derive(move || {
            ticked.with(|t| members.with_value(|ms| selection::group_state(t, ms)))
        }),
        Callback::new(move |next| {
            members.with_value(|ms| ticked.update(|t| selection::tick_all(t, ms, next)));
        }),
    ))
}

/// Select-all, in the rows' checkbox column: the list box's `space-3` plus
/// the gutter the rows keep for a disclosure. Absent under whole-package scope,
/// and while nothing on screen can be ticked.
fn select_all(
    picking: Picking,
    shown: Signal<Vec<String>>,
    narrowed: Signal<bool>,
    headed: Signal<bool>,
) -> AnyView {
    if picking.whole_package {
        return ().into_any();
    }
    let ticked = picking.ticked;
    let selected = Signal::derive(move || {
        shown.with(|s| ticked.with(|t| selection::ticked_among(t, s).len()))
    });
    let total = Signal::derive(move || shown.with(Vec::len));
    let any = Signal::derive(move || total.get() > 0);
    view! {
        <Show when=move || any.get()>
            <div
                class=style::selectall
                style=move || format!("--q-entry-gutter:{}", if headed.get() { "16px" } else { "0" })
            >
                <SelectAll
                    selected=selected
                    total=total
                    narrowed=narrowed
                    on_toggle=move |next| {
                        shown.with_untracked(|s| ticked.update(|t| selection::tick_all(t, s, next)));
                    }
                />
            </div>
        </Show>
    }
    .into_any()
}

/// The list box's last child while something is ticked: a right-aligned
/// primary that counts what it will fetch, `Download 3`. The count is every
/// loaded tick, hidden ones included, so it can exceed select-all's `1 of 1
/// selected` under a search (owner, 2026-09-25).
///
/// Slides 4px and fades in over 160ms and has no exit (`.footer`, which copies
/// the Banner's carve-out): unticking the last row grows the list back, and
/// animating that would move rows under the pointer. Not the gallery's
/// `g-fp-footer`: that is gallery chrome, which the app does not load.
fn footer(picking: Picking, loaded: StoredValue<Vec<String>>) -> AnyView {
    if picking.whole_package {
        return ().into_any();
    }
    let ticked = picking.ticked;
    // Every loaded row, not only the shown ones: a search hides a tick, it
    // does not undo it.
    let chosen =
        Memo::new(move |_| ticked.with(|t| loaded.with_value(|l| selection::ticked_among(t, l))));
    view! {
        <Show when=move || chosen.with(|c| !c.is_empty())>
            <div class=style::footer>
                <Button
                    variant=ButtonVariant::Primary
                    loading=picking.downloading
                    disabled=picking.busy
                    on_click=move |_| picking.on_download.run(chosen.get_untracked())
                >
                    {move || format!("Download {}", thousands(chosen.with(Vec::len)))}
                </Button>
            </div>
        </Show>
    }
    .into_any()
}

/// The toolbar under the search row: select-all's slot on the left, the view
/// controls on the right. The facets need the package's counts, so a pane
/// with no answer draws grouping alone.
fn toolbar(
    grouping: RwSignal<String>,
    select_all: AnyView,
    facets: Option<AnyView>,
) -> impl IntoView {
    view! {
        // Stacks upwards, so the line nearest the rows is the one acting on
        // them. Select-all goes first, on the left.
        <ListToolbar reverse_when_stacked=true>
            {select_all}
            <div class=style::views>
                <Select
                    naming=Naming::Prefix("Group".to_string())
                    options=Grouping::ALL.iter().map(|g| g.label().to_string()).collect()
                    selected=grouping
                />
                {facets}
            </div>
        </ListToolbar>
    }
}

/// The file pane.
#[component]
pub fn FilePane(
    listing: Signal<Listing>,
    /// The `Group:` select's value. The page owns it, so a re-read does not
    /// reset it; a visit to another package does.
    grouping: RwSignal<String>,
    /// The folder headings the reader collapsed. The page's, like `grouping`.
    collapsed: RwSignal<BTreeSet<String>>,
    /// The search field's text. The page's, like `grouping`.
    search: RwSignal<String>,
    /// The chosen [`Facet`], as its [`Facet::key`]. The page owns it, for the
    /// grouping's reason.
    facet: RwSignal<String>,
    /// Open a downloaded file, by its logical path.
    on_open: Callback<String>,
    /// Read the list again after a failure.
    on_retry: Callback<()>,
    /// What is ticked and what `[Download]` does. The page's, so it outlives
    /// a re-read.
    #[prop(optional)]
    picking: Picking,
) -> impl IntoView {
    move || match listing.get() {
        Listing::Loading => view! { <FilePaneSkeleton /> }.into_any(),
        Listing::Unlisted(reason) => view! {
            <section class=style::root aria-label="Files">
                {search_row(search)}
                {toolbar(grouping, ().into_any(), None)}
                <Card flush=true label="Files" fill=true>
                    <LoadFailure
                        centred=true
                        words="Could not list this package's files."
                        detail=reason
                        on_retry=on_retry
                    />
                </Card>
            </section>
        }
        .into_any(),
        Listing::Ready(list) => ready(list, grouping, collapsed, search, facet, on_open, picking),
    }
}

fn ready(
    list: FileList,
    grouping: RwSignal<String>,
    collapsed: RwSignal<BTreeSet<String>>,
    search: RwSignal<String>,
    facet: RwSignal<String>,
    on_open: Callback<String>,
    picking: Picking,
) -> AnyView {
    let FileList {
        entries,
        counts,
        total,
        truncated,
    } = list;
    let loaded_count = entries.len();
    let rows: Vec<Row> = entries
        .into_iter()
        .map(|e| Row {
            place: Place::of(&e.status),
            size: format_size(e.size),
            ignored: e.ignored_by.is_some(),
            path: e.filename,
        })
        .collect();
    let loaded = StoredValue::new(offered(&rows));
    let rows = StoredValue::new(rows);

    // What the list draws and select-all counts and ticks. A memo, so a
    // keystroke that leaves the same rows standing — the first letters typed
    // into a big package — rebuilds nothing.
    let shown = Memo::new(move |_| {
        let f = Facet::from_key(&facet.get());
        search.with(|q| rows.with_value(|rs| shown_rows(rs, q, f)))
    });
    // A view control narrowed the list. Known here, not from the count: a
    // facet can narrow to exactly as many rows as there are (`SelectAll`).
    let narrowed = Signal::derive(move || {
        !search.with(String::is_empty) || Facet::from_key(&facet.get()) != Facet::All
    });
    let shown_offered = Signal::derive(move || shown.with(|rs| offered(rs)));
    let headed = Signal::derive(move || {
        let g = Grouping::from_label(&grouping.get());
        shown.with(|rs| {
            let paths: Vec<&str> = rs.iter().map(|r| r.path.as_str()).collect();
            group(&paths, g)
                .iter()
                .any(|i| matches!(i, Item::Group { .. }))
        })
    });

    let body = move || {
        if total == 0 {
            return view! {
                <Blankslate
                    heading="Nothing in this package"
                    description="This package has no files yet."
                />
            }
            .into_any();
        }
        let g = Grouping::from_label(&grouping.get());
        shown.with(|rs| {
            if !rs.is_empty() {
                return rows_view(rs, g, collapsed, on_open, picking);
            }
            let f = Facet::from_key(&facet.get());
            // Compact: `Blankslate`'s own padding is taller than this box at
            // the height floor. No action yet — see `Blankslate`.
            if rows.with_value(|rs| !shown_rows(rs, "", f).is_empty()) {
                // The facet has rows here; the search left none of them.
                view! {
                    <Blankslate
                        compact=true
                        heading="No files match"
                        description="Nothing in this view matches what you are looking for."
                    />
                }
                .into_any()
            } else if f == Facet::All && counts.all == 0 {
                // Never zero-count unless it is `All`: an inert facet cannot
                // be chosen. So `All` at zero is a package all ignored, and
                // any other empty facet counts files the cap left out.
                view! {
                    <Blankslate
                        compact=true
                        heading="No files to show"
                        description="Every file in this package is ignored."
                    />
                }
                .into_any()
            } else {
                view! {
                    <Blankslate
                        compact=true
                        heading="None of the loaded files are in this view"
                        description="Its files sort after the ones this list covers."
                    />
                }
                .into_any()
            }
        })
    };

    view! {
        <section class=style::root aria-label="Files">
            {search_row(search)}
            {toolbar(
                grouping,
                select_all(picking, shown_offered, narrowed, headed),
                Some(facets(&counts, facet).into_any()),
            )}
            <Card flush=true label="Files" fill=true>
                {truncated.then(|| view! { <CapNotice total=total shown=loaded_count /> })}
                {body}
                {footer(picking, loaded)}
            </Card>
        </section>
    }
    .into_any()
}

/// The pane before its read answers. Reserves the toolbar's height and the
/// list's, so nothing moves when the rows land.
#[component]
pub fn FilePaneSkeleton() -> impl IntoView {
    view! {
        <section class=style::root aria-label="Files">
            // The search field: full width, one control high.
            <SkeletonBox width="100%" height="32px" />
            <div class=style::skeletonbar>
                // The `Group:` select's width, then the facets'.
                <SkeletonBox width="166px" height="32px" />
                <SkeletonBox width="396px" height="32px" />
            </div>
            <Card flush=true label="Files" fill=true busy=Signal::stored(true)>
                <div class=style::skeleton>
                    {["48%", "62%", "55%", "48%", "62%", "55%", "48%", "62%"]
                        .into_iter()
                        .map(|width| view! { <SkeletonBox width=width /> })
                        .collect_view()}
                </div>
            </Card>
        </section>
    }
}

#[cfg(test)]
mod grouping_tests {
    use super::*;

    fn row(index: usize, name: &str) -> Item {
        Item::Row {
            index,
            name: name.to_string(),
        }
    }

    fn heading(heading: &str, rows: &[(usize, &str)]) -> Item {
        Item::Group {
            heading: heading.to_string(),
            rows: rows.iter().map(|&(i, n)| (i, n.to_string())).collect(),
        }
    }

    /// Path order, as the page read sends it.
    const PACKAGE: &[&str] = &[
        "README.md",
        "notes/a.md",
        "notes/b.md",
        "quilt_summarize.json",
        "raw/2026/plate-01.csv",
        "raw/2026/plate-02.csv",
        "raw/readme.txt",
    ];

    #[test]
    fn base_folder_puts_root_files_first_with_no_heading() {
        assert_eq!(
            group(PACKAGE, Grouping::BaseFolder),
            vec![
                row(0, "README.md"),
                row(3, "quilt_summarize.json"),
                heading("notes/", &[(1, "a.md"), (2, "b.md")]),
                heading("raw/2026/", &[(4, "plate-01.csv"), (5, "plate-02.csv")]),
                // A group of one gets no heading: the row stands alone and
                // carries its folder in its name.
                row(6, "raw/readme.txt"),
            ]
        );
    }

    #[test]
    fn top_folder_groups_by_the_first_segment_and_names_the_rest() {
        assert_eq!(
            group(PACKAGE, Grouping::TopFolder),
            vec![
                row(0, "README.md"),
                row(3, "quilt_summarize.json"),
                heading("notes/", &[(1, "a.md"), (2, "b.md")]),
                heading(
                    "raw/",
                    &[
                        (4, "2026/plate-01.csv"),
                        (5, "2026/plate-02.csv"),
                        (6, "readme.txt")
                    ]
                ),
            ]
        );
    }

    #[test]
    fn none_is_flat_full_paths_in_path_order() {
        let flat: Vec<Item> = PACKAGE.iter().enumerate().map(|(i, p)| row(i, p)).collect();
        assert_eq!(group(PACKAGE, Grouping::None), flat);
    }

    #[test]
    fn a_singleton_top_folder_is_ungrouped_and_shows_its_path() {
        assert_eq!(
            group(&["a.txt", "docs/deep/one.md"], Grouping::TopFolder),
            vec![row(0, "a.txt"), row(1, "docs/deep/one.md")]
        );
    }

    /// Path order does not keep a folder's files together: a subfolder's paths
    /// can sort between them. One heading per folder all the same.
    #[test]
    fn a_folder_split_by_its_subfolder_is_still_one_group() {
        assert_eq!(
            group(
                &["a/m.txt", "a/n/x.txt", "a/n/y.txt", "a/z.txt"],
                Grouping::BaseFolder
            ),
            vec![
                heading("a/", &[(0, "m.txt"), (3, "z.txt")]),
                heading("a/n/", &[(1, "x.txt"), (2, "y.txt")]),
            ]
        );
    }
}

#[cfg(test)]
mod search_tests {
    use super::*;

    #[test]
    fn an_empty_search_matches_every_file() {
        assert!(path_matches("raw/2026/plate-01.csv", ""));
    }

    #[test]
    fn search_ignores_case_on_both_sides() {
        assert!(path_matches("raw/Plate-01.CSV", "plate-01.csv"));
        assert!(path_matches("raw/plate-01.csv", "PLATE"));
    }

    /// The whole logical path, not only the name: a folder finds its files.
    #[test]
    fn search_matches_a_substring_anywhere_in_the_path() {
        assert!(path_matches("raw/2026/plate-01.csv", "2026/pla"));
        assert!(path_matches("raw/2026/plate-01.csv", "w/20"));
        assert!(!path_matches("raw/2026/plate-01.csv", "2027"));
    }
}

#[cfg(test)]
mod facet_tests {
    use super::*;

    /// The facet and the search narrow together, in the one hook both the list
    /// and select-all read.
    #[test]
    fn shown_rows_takes_the_facet_and_the_search_together() {
        let row = |path: &str, status: &str| Row {
            path: path.to_string(),
            size: String::new(),
            place: Place::of(status),
            ignored: false,
        };
        let rows = [
            row("raw/a.csv", "modified"),
            row("raw/b.csv", "pristine"),
            row("notes/a.md", "modified"),
        ];
        let paths = |query: &str, facet: Facet| {
            shown_rows(&rows, query, facet)
                .into_iter()
                .map(|r| r.path)
                .collect::<Vec<_>>()
        };
        assert_eq!(paths("raw/", Facet::Changed), ["raw/a.csv"]);
        assert_eq!(paths("", Facet::Changed), ["raw/a.csv", "notes/a.md"]);
        assert_eq!(paths("raw/", Facet::All), ["raw/a.csv", "raw/b.csv"]);
        assert!(paths("raw/", Facet::NotDownloaded).is_empty());
    }

    /// The backend's own rule (`EntryCounts::of`), so a facet shows the rows
    /// its count counted.
    #[test]
    fn each_facet_admits_the_rows_its_count_counts() {
        let admitted = |facet: Facet| {
            [
                ("pristine", false),
                ("modified", false),
                ("added", false),
                ("deleted", false),
                ("remote", false),
                ("pristine", true),
                ("remote", true),
            ]
            .into_iter()
            .filter(|&(status, ignored)| facet.admits(Place::of(status), ignored))
            .collect::<Vec<_>>()
        };
        assert_eq!(
            admitted(Facet::All),
            [
                ("pristine", false),
                ("modified", false),
                ("added", false),
                ("deleted", false),
                ("remote", false),
            ]
        );
        assert_eq!(
            admitted(Facet::Changed),
            [("modified", false), ("added", false), ("deleted", false)]
        );
        assert_eq!(admitted(Facet::NotDownloaded), [("remote", false)]);
        assert_eq!(
            admitted(Facet::Ignored),
            [("pristine", true), ("remote", true)]
        );
    }
}

#[cfg(test)]
mod pane_tests {
    use super::*;
    use crate::commands::{EntryCounts, EntryData};
    use crate::test_support::{button_saying, element_saying, mount};
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::*;

    fn entry(path: &str, status: &str) -> EntryData {
        EntryData {
            filename: path.to_string(),
            size: 4_100,
            status: status.to_string(),
            junky_pattern: None,
            ignored_by: None,
            namespace: "team/dataset".try_into().unwrap(),
        }
    }

    fn ignored(path: &str) -> EntryData {
        EntryData {
            ignored_by: Some(".DS_Store".to_string()),
            ..entry(path, "pristine")
        }
    }

    /// A whole list: its counts are its own rows', as the backend counts them.
    fn list(entries: Vec<EntryData>, total: usize, truncated: bool) -> FileList {
        let mut counts = EntryCounts::default();
        for e in &entries {
            if e.ignored_by.is_some() {
                counts.ignored += 1;
                continue;
            }
            counts.all += 1;
            match e.status.as_str() {
                "added" | "modified" | "deleted" => counts.changed += 1,
                "remote" => counts.not_downloaded += 1,
                _ => {}
            }
        }
        FileList {
            entries,
            counts,
            total,
            truncated,
        }
    }

    fn facet(el: &web_sys::Element, words: &str) -> web_sys::HtmlInputElement {
        element_saying(el, words)
            .closest("label")
            .unwrap()
            .expect("a segment")
            .query_selector("input[type=radio]")
            .unwrap()
            .expect("its radio")
            .unchecked_into()
    }

    fn facet_pane(listing: Listing, facet: RwSignal<String>) -> web_sys::Element {
        mount(move || {
            view! {
                <FilePane
                    listing=Signal::stored(listing)
                    grouping=RwSignal::new(Grouping::None.label().to_string())
                    collapsed=RwSignal::new(BTreeSet::new())
                    search=RwSignal::new(String::new())
                    facet=facet
                    on_open=Callback::new(|_: String| ())
                    on_retry=Callback::new(|()| ())
                />
            }
        })
    }

    fn drawn(el: &web_sys::Element) -> Vec<String> {
        let found = el.query_selector_all("[title]").unwrap();
        (0..found.length())
            .filter_map(|i| found.get(i)?.dyn_into::<web_sys::Element>().ok())
            .filter_map(|e| e.get_attribute("title"))
            .collect()
    }

    fn mixed() -> FileList {
        list(
            vec![
                entry("added.txt", "added"),
                entry("deleted.txt", "deleted"),
                entry("here.txt", "pristine"),
                entry("modified.txt", "modified"),
                entry("remote.txt", "remote"),
                ignored(".DS_Store"),
            ],
            6,
            false,
        )
    }

    /// Choosing a facet narrows the rows to it, and `All` brings them back.
    #[wasm_bindgen_test]
    async fn choosing_a_facet_narrows_the_rows_to_it() {
        let chosen = RwSignal::new(Facet::All.key().to_string());
        let el = facet_pane(Listing::Ready(mixed()), chosen);
        assert_eq!(
            drawn(&el),
            [
                "added.txt",
                "deleted.txt",
                "here.txt",
                "modified.txt",
                "remote.txt"
            ]
        );

        facet(&el, "Changed 3").click();
        leptos::task::tick().await;
        assert_eq!(chosen.get_untracked(), "Changed", "the page holds the key");
        assert_eq!(drawn(&el), ["added.txt", "deleted.txt", "modified.txt"]);

        facet(&el, "Not downloaded 1").click();
        leptos::task::tick().await;
        assert_eq!(drawn(&el), ["remote.txt"]);

        facet(&el, "All 5").click();
        leptos::task::tick().await;
        assert_eq!(drawn(&el).len(), 5);
    }

    /// Every row under `Ignored` is ignored, so none says so; and an ignored
    /// file is never selectable, even one that is not downloaded — stop
    /// ignoring it first.
    #[wasm_bindgen_test]
    fn ignored_rows_carry_no_label_and_no_box() {
        let chosen = RwSignal::new(Facet::Ignored.key().to_string());
        let el = facet_pane(
            Listing::Ready(list(
                vec![
                    ignored(".DS_Store"),
                    EntryData {
                        ignored_by: Some("*.tmp".to_string()),
                        ..entry("scratch.tmp", "remote")
                    },
                    entry("here.txt", "pristine"),
                ],
                3,
                false,
            )),
            chosen,
        );
        assert_eq!(drawn(&el), [".DS_Store", "scratch.tmp"]);
        let list = el
            .query_selector(&format!(".{}", style::list))
            .unwrap()
            .expect("the list");
        assert!(
            list.query_selector("input[type=checkbox]")
                .unwrap()
                .is_none(),
            "no box; markup was {}",
            list.inner_html()
        );
        assert!(
            list.query_selector("button").unwrap().is_none(),
            "and nothing to click"
        );
        assert!(
            !text(&list).contains("Not downloaded"),
            "no label; markup was {}",
            list.inner_html()
        );
    }

    /// A tracked file that `.quiltignore` also matches is two rows, as in v1:
    /// the ignored one under `Ignored`, and the tracked one — here deleted,
    /// since the walk skips it — under `All` and `Changed`. Counts follow rows.
    #[wasm_bindgen_test]
    async fn an_ignored_tracked_file_keeps_both_its_rows() {
        let chosen = RwSignal::new(Facet::All.key().to_string());
        let el = facet_pane(
            Listing::Ready(list(
                vec![ignored("data.csv"), entry("data.csv", "deleted")],
                2,
                false,
            )),
            chosen,
        );
        assert_eq!(drawn(&el), ["data.csv"]);
        assert!(text(&el).contains("Deleted"));

        facet(&el, "Ignored 1").click();
        leptos::task::tick().await;
        assert_eq!(drawn(&el), ["data.csv"]);
        assert!(!text(&el).contains("Deleted"), "the ignored row is silent");

        facet(&el, "Changed 1").click();
        leptos::task::tick().await;
        assert_eq!(drawn(&el), ["data.csv"]);
    }

    /// A facet with nothing in it stays on screen and cannot be chosen. `All`
    /// always can: it is the pane's home and where a view falls back to.
    #[wasm_bindgen_test]
    fn a_zero_count_facet_is_present_and_inert() {
        let el = pane(Listing::Ready(list(vec![ignored(".DS_Store")], 1, false)));
        assert!(facet(&el, "Changed 0").disabled());
        assert!(facet(&el, "Not downloaded 0").disabled());
        assert!(!facet(&el, "Ignored 1").disabled());
        assert!(!facet(&el, "All 0").disabled(), "home is never shut");
    }

    /// A re-read can empty the facet the reader is on — the download finished,
    /// so `Not downloaded` is zero. The view falls back to `All`, since an
    /// inert segment cannot hold the choice.
    #[wasm_bindgen_test]
    async fn a_chosen_facet_that_empties_falls_back_to_all() {
        let chosen = RwSignal::new(Facet::NotDownloaded.key().to_string());
        let el = facet_pane(
            Listing::Ready(list(vec![entry("here.txt", "pristine")], 1, false)),
            chosen,
        );
        leptos::task::tick().await;
        assert_eq!(chosen.get_untracked(), "All");
        assert!(facet(&el, "All 1").checked());
        assert_eq!(drawn(&el), ["here.txt"]);
    }

    /// A package whose every file is ignored has nothing under `All`, and all
    /// of it under `Ignored`.
    #[wasm_bindgen_test]
    async fn a_wholly_ignored_package_lists_its_files_under_ignored() {
        let chosen = RwSignal::new(Facet::All.key().to_string());
        let el = facet_pane(
            Listing::Ready(list(vec![ignored(".DS_Store")], 1, false)),
            chosen,
        );
        element_saying(&el, "Every file in this package is ignored.");

        facet(&el, "Ignored 1").click();
        leptos::task::tick().await;
        assert_eq!(drawn(&el), [".DS_Store"]);
    }

    /// Over the cap a facet can count files none of whose rows were loaded.
    /// The view says so rather than drawing an empty box.
    #[wasm_bindgen_test]
    fn a_facet_whose_files_are_past_the_cap_says_so() {
        let el = facet_pane(
            Listing::Ready(FileList {
                counts: EntryCounts {
                    all: 1_500,
                    changed: 4,
                    not_downloaded: 0,
                    ignored: 0,
                },
                ..list(vec![entry("a.csv", "pristine")], 1_500, true)
            }),
            RwSignal::new(Facet::Changed.key().to_string()),
        );
        assert!(drawn(&el).is_empty());
        element_saying(&el, "None of the loaded files are in this view");
    }

    /// The facets describe the package, not the loaded rows: over the cap a
    /// facet still says how many files the package has in it.
    #[wasm_bindgen_test]
    fn the_facets_count_the_whole_package() {
        let el = pane(Listing::Ready(FileList {
            counts: EntryCounts {
                all: 4_309,
                changed: 2,
                not_downloaded: 17,
                ignored: 3,
            },
            ..list(vec![entry("a.csv", "pristine")], 4_312, true)
        }));
        for words in ["All 4,309", "Changed 2", "Not downloaded 17", "Ignored 3"] {
            assert!(
                facet(&el, words).name() == "file-facets",
                "one group of four; markup was {}",
                el.inner_html()
            );
        }
        assert!(facet(&el, "All 4,309").checked(), "All is the start");
    }

    fn pane(listing: Listing) -> web_sys::Element {
        mount(move || {
            view! {
                <FilePane
                    listing=Signal::stored(listing)
                    grouping=RwSignal::new(Grouping::BaseFolder.label().to_string())
                    collapsed=RwSignal::new(BTreeSet::new())
                    search=RwSignal::new(String::new())
                    facet=RwSignal::new(Facet::All.key().to_string())
                    on_open=Callback::new(|_: String| ())
                    on_retry=Callback::new(|()| ())
                />
            }
        })
    }

    fn text(el: &web_sys::Element) -> String {
        el.text_content().unwrap_or_default()
    }

    /// Ignored rows are hidden until the `Ignored` facet shows them, and even
    /// then one that is not downloaded carries no box: stop ignoring it first.
    #[test]
    fn only_a_missing_row_that_is_not_ignored_can_be_ticked() {
        let r = |place, ignored| Row {
            path: "a.csv".to_string(),
            size: String::new(),
            place,
            ignored,
        };
        assert!(r(Place::Missing, false).selectable());
        assert!(!r(Place::Missing, true).selectable());
        for place in [
            Place::Here,
            Place::Changed,
            Place::New,
            Place::Deleted,
            Place::Unknown,
        ] {
            assert!(!r(place, false).selectable());
        }
    }

    #[test]
    fn counts_carry_thousands_separators() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1_000), "1,000");
        assert_eq!(thousands(4_312), "4,312");
        assert_eq!(thousands(1_234_567), "1,234,567");
    }

    /// The flag decides, never the length: a list of two that the backend says
    /// was cut gets the notice, and a list it says is whole does not.
    #[wasm_bindgen_test]
    fn the_cap_notice_appears_only_when_the_backend_says_the_list_was_cut() {
        let rows = || vec![entry("a.csv", "pristine"), entry("b.csv", "pristine")];

        let cut = pane(Listing::Ready(list(rows(), 4_312, true)));
        assert!(
            text(&cut)
                .contains("This package has 4,312 files. This list covers the first 2 by path."),
            "markup was {}",
            cut.inner_html()
        );

        let whole = pane(Listing::Ready(list(rows(), 4_312, false)));
        assert!(
            !text(&whole).contains("This package has"),
            "markup was {}",
            whole.inner_html()
        );
    }

    #[wasm_bindgen_test]
    fn ignored_files_are_hidden() {
        let el = pane(Listing::Ready(list(
            vec![entry("a.csv", "pristine"), ignored(".DS_Store")],
            2,
            false,
        )));
        assert!(
            text(&el).contains("a.csv"),
            "markup was {}",
            el.inner_html()
        );
        assert!(
            !text(&el).contains("DS_Store"),
            "markup was {}",
            el.inner_html()
        );
    }

    /// Design §3's table: the resting states are silent, the rest are named.
    #[wasm_bindgen_test]
    fn rows_are_labelled_by_state_and_the_resting_state_is_silent() {
        let el = pane(Listing::Ready(list(
            vec![
                entry("added.txt", "added"),
                entry("deleted.txt", "deleted"),
                entry("here.txt", "pristine"),
                entry("modified.txt", "modified"),
                entry("remote.txt", "remote"),
            ],
            5,
            false,
        )));
        let row_text = |name: &str| {
            element_saying(&el, name)
                .closest(&format!(".{}", style::list))
                .unwrap()
                .expect("inside the list");
            let title = el
                .query_selector(&format!("[title='{name}']"))
                .unwrap()
                .expect("titled row");
            text(&title.parent_element().unwrap())
        };
        assert!(row_text("added.txt").contains("New"));
        assert!(row_text("deleted.txt").contains("Deleted"));
        assert!(row_text("modified.txt").contains("Changed"));
        assert!(row_text("remote.txt").contains("Not downloaded"));
        let here = row_text("here.txt");
        for word in ["New", "Deleted", "Changed", "Not downloaded", "Downloaded"] {
            assert!(
                !here.contains(word),
                "a downloaded row says nothing: {here}"
            );
        }
    }

    /// A click does what the row can do: a file here opens, a missing one is a
    /// box, and a deleted one is inert.
    #[wasm_bindgen_test]
    fn a_downloaded_row_opens_a_missing_one_ticks_and_a_deleted_one_is_inert() {
        let opened = RwSignal::new(Vec::<String>::new());
        let el = mount(move || {
            view! {
                <FilePane
                    listing=Signal::stored(Listing::Ready(list(
                        vec![
                            entry("deleted.txt", "deleted"),
                            entry("here.txt", "pristine"),
                            entry("remote.txt", "remote"),
                        ],
                        3,
                        false,
                    )))
                    grouping=RwSignal::new(Grouping::BaseFolder.label().to_string())
                    collapsed=RwSignal::new(BTreeSet::new())
                    search=RwSignal::new(String::new())
                    facet=RwSignal::new(Facet::All.key().to_string())
                    on_open=Callback::new(move |path: String| opened.update(|o| o.push(path)))
                    on_retry=Callback::new(|()| ())
                />
            }
        });
        let titled = |name: &str| {
            el.query_selector(&format!("[title='{name}']"))
                .unwrap()
                .expect("titled row")
        };

        let here: web_sys::HtmlElement = titled("here.txt").dyn_into().unwrap();
        assert_eq!(
            here.tag_name(),
            "BUTTON",
            "a downloaded row's name is a button"
        );
        here.click();
        assert_eq!(opened.get_untracked(), vec!["here.txt".to_string()]);

        let remote = titled("remote.txt").closest("label").unwrap();
        assert!(
            remote.is_some_and(|l| l.query_selector("input[type=checkbox]").unwrap().is_some()),
            "a missing row is the Select shape"
        );

        let deleted = titled("deleted.txt");
        assert_eq!(deleted.tag_name(), "SPAN");
        assert!(
            deleted.closest("label").unwrap().is_none(),
            "and a deleted one is inert"
        );
    }

    /// A pane over these rows, with its selection handed in so a test can
    /// read what was ticked and what `[Download]` asked for.
    fn picking_pane(
        entries: Vec<EntryData>,
        picking: Picking,
        grouping: Grouping,
    ) -> web_sys::Element {
        let total = entries.len();
        mount(move || {
            view! {
                <FilePane
                    listing=Signal::stored(Listing::Ready(list(entries, total, false)))
                    grouping=RwSignal::new(grouping.label().to_string())
                    collapsed=RwSignal::new(BTreeSet::new())
                    search=RwSignal::new(String::new())
                    facet=RwSignal::new(Facet::All.key().to_string())
                    on_open=Callback::new(|_: String| ())
                    on_retry=Callback::new(|()| ())
                    picking=picking
                />
            }
        })
    }

    fn boxes(el: &web_sys::Element) -> u32 {
        el.query_selector_all("input[type=checkbox]")
            .unwrap()
            .length()
    }

    fn tick(el: &web_sys::Element, name: &str) {
        let row = el
            .query_selector(&format!("[title='{name}']"))
            .unwrap()
            .expect("titled row")
            .closest("label")
            .unwrap()
            .expect("a selectable row is a label");
        let input: web_sys::HtmlElement = row
            .query_selector("input[type=checkbox]")
            .unwrap()
            .unwrap()
            .unchecked_into();
        input.click();
    }

    fn mixed_package() -> Vec<EntryData> {
        vec![
            entry("added.txt", "added"),
            entry("deleted.txt", "deleted"),
            entry("here.txt", "pristine"),
            EntryData {
                status: "remote".to_string(),
                ..ignored("junk.tmp")
            },
            entry("remote-a.csv", "remote"),
            entry("remote-b.csv", "remote"),
        ]
    }

    /// A facet or a search narrows what select-all reaches, and its label
    /// says so; under `Ignored` nothing can be ticked, so it is gone.
    #[wasm_bindgen_test]
    async fn the_view_controls_narrow_select_all_and_say_so() {
        let (search, facet) = (RwSignal::new(String::new()), RwSignal::new(String::new()));
        facet.set(Facet::NotDownloaded.key().to_string());
        let entries = mixed_package();
        let total = entries.len();
        let picking = Picking::default();
        let el = mount(move || {
            view! {
                <FilePane
                    listing=Signal::stored(Listing::Ready(list(entries, total, false)))
                    grouping=RwSignal::new(Grouping::None.label().to_string())
                    collapsed=RwSignal::new(BTreeSet::new())
                    search=search
                    facet=facet
                    on_open=Callback::new(|_: String| ())
                    on_retry=Callback::new(|()| ())
                    picking=picking
                />
            }
        });
        element_saying(&el, "Select all 2 shown");
        search.set("remote-a".to_string());
        leptos::task::tick().await;
        element_saying(&el, "Select all 1 shown");
        // A search hides a tick without undoing it: `[Download]` counts both.
        picking
            .ticked
            .set(["remote-a.csv".to_string(), "remote-b.csv".to_string()].into());
        leptos::task::tick().await;
        element_saying(&el, "1 of 1 selected");
        button_saying(&el, "Download 2");
        search.set(String::new());
        facet.set(Facet::Ignored.key().to_string());
        leptos::task::tick().await;
        assert_eq!(boxes(&el), 0, "markup was {}", el.inner_html());
        assert!(!text(&el).contains("Select all"));
        button_saying(&el, "Download 2");
    }

    /// Only a not-downloaded, non-ignored row carries a box — here two of
    /// them — and select-all counts exactly those.
    #[wasm_bindgen_test]
    fn only_downloadable_rows_carry_a_box_and_select_all_counts_them() {
        let el = picking_pane(mixed_package(), Picking::default(), Grouping::BaseFolder);
        assert_eq!(
            boxes(&el),
            3,
            "two rows and select-all; markup was {}",
            el.inner_html()
        );
        element_saying(&el, "Select all 2");
    }

    #[wasm_bindgen_test]
    async fn select_all_reads_what_is_ticked_and_ticks_what_is_shown() {
        let picking = Picking::default();
        let el = picking_pane(mixed_package(), picking, Grouping::BaseFolder);

        tick(&el, "remote-a.csv");
        leptos::task::tick().await;
        element_saying(&el, "1 of 2 selected");

        // Mixed, so a click ticks the rest.
        element_saying(&el, "1 of 2 selected").click();
        leptos::task::tick().await;
        element_saying(&el, "2 of 2 selected");
        assert_eq!(
            picking.ticked.get_untracked(),
            ["remote-a.csv", "remote-b.csv"]
                .map(String::from)
                .into_iter()
                .collect()
        );

        element_saying(&el, "2 of 2 selected").click();
        leptos::task::tick().await;
        element_saying(&el, "Select all 2");
    }

    /// A heading's box names its group, and ticks the rows under it.
    #[wasm_bindgen_test]
    async fn a_heading_box_names_its_group_and_ticks_its_rows() {
        let picking = Picking::default();
        let el = picking_pane(
            vec![
                entry("raw/a.csv", "remote"),
                entry("raw/b.csv", "remote"),
                entry("raw/c.csv", "pristine"),
            ],
            picking,
            Grouping::BaseFolder,
        );
        let heading: web_sys::HtmlElement = el
            .query_selector("input[aria-label='Select all in raw/']")
            .unwrap()
            .expect("the heading's box names its group")
            .unchecked_into();
        heading.click();
        leptos::task::tick().await;
        assert_eq!(picking.ticked.get_untracked().len(), 2);
        element_saying(&el, "2 of 2 selected");
    }

    /// The footer exists only while something is ticked, and `[Download]`
    /// sends the ticked paths in path order.
    #[wasm_bindgen_test]
    async fn the_footer_appears_on_a_tick_and_downloads_the_ticked_paths() {
        let asked = RwSignal::new(Vec::<Vec<String>>::new());
        let picking = Picking {
            on_download: Callback::new(move |paths| asked.update(|a| a.push(paths))),
            ..Picking::default()
        };
        let el = picking_pane(mixed_package(), picking, Grouping::BaseFolder);
        let download = |el: &web_sys::Element| {
            let all = el.query_selector_all("button").unwrap();
            (0..all.length())
                .map(|i| {
                    all.item(i)
                        .unwrap()
                        .unchecked_into::<web_sys::HtmlElement>()
                })
                .find(|b| {
                    b.text_content()
                        .unwrap_or_default()
                        .trim()
                        .starts_with("Download")
                })
        };
        assert!(download(&el).is_none(), "no footer with nothing ticked");

        tick(&el, "remote-b.csv");
        tick(&el, "remote-a.csv");
        leptos::task::tick().await;
        let button = download(&el).expect("the footer's Download");
        assert_eq!(
            button.text_content().unwrap_or_default().trim(),
            "Download 2"
        );
        button.click();
        assert_eq!(
            asked.get_untracked(),
            vec![vec!["remote-a.csv".to_string(), "remote-b.csv".to_string()]]
        );
    }

    #[wasm_bindgen_test]
    fn the_download_shows_its_progress_on_the_button() {
        let picking = Picking {
            downloading: Signal::stored(true),
            ..Picking::default()
        };
        picking.ticked.set(["remote-a.csv".to_string()].into());
        let el = picking_pane(mixed_package(), picking, Grouping::BaseFolder);
        let button = button_saying(&el, "Download 1");
        assert_eq!(button.get_attribute("aria-busy").as_deref(), Some("true"));
        assert!(button.disabled());
    }

    /// Under whole-package Keeping the scope has taken the per-file choice
    /// away: no boxes, no select-all, and no footer even with a tick left over.
    #[wasm_bindgen_test]
    fn whole_package_scope_draws_no_boxes_no_select_all_and_no_footer() {
        let picking = Picking {
            whole_package: true,
            ..Picking::default()
        };
        picking.ticked.set(["remote-a.csv".to_string()].into());
        let el = picking_pane(
            vec![
                entry("raw/a.csv", "remote"),
                entry("raw/b.csv", "remote"),
                entry("remote-a.csv", "remote"),
            ],
            picking,
            Grouping::BaseFolder,
        );
        assert_eq!(boxes(&el), 0, "markup was {}", el.inner_html());
        assert!(!text(&el).contains("Select all"));
        assert!(!text(&el).contains("Download"));
    }

    #[wasm_bindgen_test]
    async fn grouping_defaults_to_base_folder_and_none_flattens_the_list() {
        let grouping = RwSignal::new(Grouping::BaseFolder.label().to_string());
        let el = mount(move || {
            view! {
                <FilePane
                    listing=Signal::stored(Listing::Ready(list(
                        vec![
                            entry("README.md", "pristine"),
                            entry("notes/a.md", "pristine"),
                            entry("notes/b.md", "pristine"),
                        ],
                        3,
                        false,
                    )))
                    grouping=grouping
                    collapsed=RwSignal::new(BTreeSet::new())
                    search=RwSignal::new(String::new())
                    facet=RwSignal::new(Facet::All.key().to_string())
                    on_open=Callback::new(|_: String| ())
                    on_retry=Callback::new(|()| ())
                />
            }
        });
        let select: web_sys::HtmlSelectElement = el
            .query_selector("select")
            .unwrap()
            .expect("the grouping select")
            .unchecked_into();
        assert_eq!(select.value(), "Base folder");
        assert_eq!(select.length(), 3, "Base folder, Top folder, None");
        element_saying(&el, "notes/");
        assert!(
            el.query_selector("[aria-expanded]").unwrap().is_some(),
            "a heading with its disclosure"
        );

        grouping.set(Grouping::None.label().to_string());
        leptos::task::tick().await;

        assert!(
            el.query_selector("[aria-expanded]").unwrap().is_none(),
            "no headings under None; markup was {}",
            el.inner_html()
        );
        element_saying(&el, "notes/a.md");
    }

    #[wasm_bindgen_test]
    fn loading_draws_skeletons_and_a_failure_offers_a_retry() {
        let loading = pane(Listing::Loading);
        assert!(
            loading
                .query_selector("[aria-busy=true]")
                .unwrap()
                .is_some(),
            "markup was {}",
            loading.inner_html()
        );

        let unlisted = pane(Listing::Unlisted("Access denied".to_string()));
        element_saying(&unlisted, "Could not list this package's files.");
        assert!(
            text(&unlisted).contains("Access denied"),
            "the reason is stated; markup was {}",
            unlisted.inner_html()
        );
        assert!(
            unlisted.query_selector("[title]").unwrap().is_none(),
            "and no row is drawn without a status"
        );
    }

    #[wasm_bindgen_test]
    fn an_empty_package_says_so() {
        let el = pane(Listing::Ready(list(Vec::new(), 0, false)));
        element_saying(&el, "Nothing in this package");
        assert!(
            text(&el).contains("This package has no files yet."),
            "worded for a local-only package too; markup was {}",
            el.inner_html()
        );
    }

    /// Over the cap, only the loaded rows are known to be ignored: the counts,
    /// being the whole package's, say the rest are not.
    #[wasm_bindgen_test]
    fn a_cut_list_all_ignored_speaks_of_the_loaded_files_only() {
        let cut = pane(Listing::Ready(FileList {
            counts: EntryCounts {
                all: 4_311,
                ignored: 1,
                ..EntryCounts::default()
            },
            ..list(vec![ignored(".DS_Store")], 4_312, true)
        }));
        element_saying(&cut, "None of the loaded files are in this view");
        assert!(
            !text(&cut).contains("Every file in this package is ignored."),
            "markup was {}",
            cut.inner_html()
        );

        let whole = pane(Listing::Ready(list(vec![ignored(".DS_Store")], 1, false)));
        assert!(
            text(&whole).contains("Every file in this package is ignored."),
            "markup was {}",
            whole.inner_html()
        );
    }

    /// A re-read rebuilds the rows; a folder the reader closed stays closed.
    #[wasm_bindgen_test]
    async fn a_collapsed_folder_stays_collapsed_across_a_re_read() {
        let files = || {
            Listing::Ready(list(
                vec![
                    entry("notes/a.md", "pristine"),
                    entry("notes/b.md", "pristine"),
                ],
                2,
                false,
            ))
        };
        let listing = RwSignal::new(files());
        let collapsed = RwSignal::new(BTreeSet::new());
        let el = mount(move || {
            view! {
                <FilePane
                    listing=Signal::from(listing)
                    grouping=RwSignal::new(Grouping::BaseFolder.label().to_string())
                    collapsed=collapsed
                    search=RwSignal::new(String::new())
                    facet=RwSignal::new(Facet::All.key().to_string())
                    on_open=Callback::new(|_: String| ())
                    on_retry=Callback::new(|()| ())
                />
            }
        });
        let disclosure = || {
            el.query_selector("[aria-expanded]")
                .unwrap()
                .expect("the folder's disclosure")
        };
        disclosure()
            .unchecked_into::<web_sys::HtmlElement>()
            .click();
        crate::test_support::sleep_ms(10).await;
        assert_eq!(
            disclosure().get_attribute("aria-expanded").as_deref(),
            Some("false")
        );
        assert!(
            collapsed.get_untracked().contains("notes/"),
            "recorded on the page"
        );

        listing.set(files());
        leptos::task::tick().await;

        assert_eq!(
            disclosure().get_attribute("aria-expanded").as_deref(),
            Some("false"),
            "still collapsed; markup was {}",
            el.inner_html()
        );
        assert!(
            el.query_selector("[title='notes/a.md']").unwrap().is_none(),
            "its rows stay hidden"
        );
        assert!(collapsed.get_untracked().contains("notes/"));
    }

    /// Type into the search field as a reader would.
    async fn type_search(el: &web_sys::Element, query: &str) {
        let field: web_sys::HtmlInputElement = el
            .query_selector("input[type=search]")
            .unwrap()
            .expect("the search field")
            .unchecked_into();
        field.set_value(query);
        field
            .dispatch_event(&web_sys::Event::new("input").unwrap())
            .unwrap();
        leptos::task::tick().await;
    }

    fn searchable(search: RwSignal<String>, entries: Vec<EntryData>) -> web_sys::Element {
        let total = entries.len();
        mount(move || {
            view! {
                <FilePane
                    listing=Signal::stored(Listing::Ready(list(entries, total, false)))
                    grouping=RwSignal::new(Grouping::BaseFolder.label().to_string())
                    collapsed=RwSignal::new(BTreeSet::new())
                    search=search
                    facet=RwSignal::new(Facet::All.key().to_string())
                    on_open=Callback::new(|_: String| ())
                    on_retry=Callback::new(|()| ())
                />
            }
        })
    }

    fn titled(el: &web_sys::Element, path: &str) -> bool {
        el.query_selector(&format!("[title='{path}']"))
            .unwrap()
            .is_some()
    }

    #[wasm_bindgen_test]
    async fn the_search_field_is_named_and_writes_the_pages_query() {
        let search = RwSignal::new(String::new());
        let el = searchable(search, vec![entry("a.csv", "pristine")]);
        let field = el
            .query_selector("input[type=search]")
            .unwrap()
            .expect("the search field");
        assert_eq!(
            field.get_attribute("aria-label").as_deref(),
            Some("Search files")
        );
        type_search(&el, "a.c").await;
        assert_eq!(search.get_untracked(), "a.c");
    }

    /// Case-insensitive, over the whole path: `RAW` finds everything under
    /// `raw/`, including a file whose own name does not say it.
    #[wasm_bindgen_test]
    async fn the_search_narrows_the_rows_to_paths_containing_it() {
        let search = RwSignal::new(String::new());
        let el = searchable(
            search,
            vec![
                entry("README.md", "pristine"),
                entry("raw/2026/plate-01.csv", "remote"),
                entry("raw/2026/plate-02.csv", "pristine"),
                entry("notes/raw-data.md", "pristine"),
            ],
        );
        type_search(&el, "RAW/").await;
        assert!(titled(&el, "raw/2026/plate-01.csv"), "{}", el.inner_html());
        assert!(titled(&el, "raw/2026/plate-02.csv"));
        assert!(!titled(&el, "README.md"), "{}", el.inner_html());
        assert!(!titled(&el, "notes/raw-data.md"));

        type_search(&el, "").await;
        for path in ["README.md", "raw/2026/plate-01.csv", "notes/raw-data.md"] {
            assert!(titled(&el, path), "cleared, {path} is back");
        }
    }

    /// Headings are drawn from the rows the search leaves: a folder with no
    /// match has no heading, and one left with a single file loses its heading
    /// and shows the row's whole path, as any group of one does.
    #[wasm_bindgen_test]
    async fn group_headings_follow_the_rows_the_search_leaves() {
        let search = RwSignal::new(String::new());
        let el = searchable(
            search,
            vec![
                entry("notes/a.md", "pristine"),
                entry("notes/b.md", "pristine"),
                entry("raw/plate-01.csv", "pristine"),
                entry("raw/plate-02.csv", "pristine"),
            ],
        );
        element_saying(&el, "notes/");
        element_saying(&el, "raw/");

        type_search(&el, "plate").await;
        element_saying(&el, "raw/");
        assert!(
            !text(&el).contains("notes/"),
            "no match, no heading: {}",
            el.inner_html()
        );

        type_search(&el, "plate-01").await;
        assert!(
            el.query_selector("[aria-expanded]").unwrap().is_none(),
            "a group of one has no heading: {}",
            el.inner_html()
        );
        element_saying(&el, "raw/plate-01.csv");
    }

    #[wasm_bindgen_test]
    async fn a_search_matching_nothing_says_so() {
        let search = RwSignal::new(String::new());
        let el = searchable(search, vec![entry("a.csv", "pristine")]);
        type_search(&el, "zzz").await;
        element_saying(&el, "No files match");
        assert!(!titled(&el, "a.csv"));
        assert!(
            el.query_selector("input[type=search]").unwrap().is_some(),
            "the field stays, so the reader can change the search"
        );
    }

    /// Ignored files stay hidden whatever the search: only their facet shows
    /// them.
    #[wasm_bindgen_test]
    async fn the_search_does_not_find_ignored_files() {
        let search = RwSignal::new(String::new());
        let el = searchable(
            search,
            vec![entry("a.csv", "pristine"), ignored(".DS_Store")],
        );
        type_search(&el, "DS_Store").await;
        assert!(!text(&el).contains(".DS_Store"), "{}", el.inner_html());
        element_saying(&el, "No files match");
    }

    /// A folder the reader closed is still closed when a search hides it and
    /// then brings it back: the headings are rebuilt, the page's record is not.
    #[wasm_bindgen_test]
    async fn a_collapsed_folder_stays_collapsed_through_a_search() {
        let search = RwSignal::new(String::new());
        let collapsed = RwSignal::new(BTreeSet::new());
        let el = mount(move || {
            view! {
                <FilePane
                    listing=Signal::stored(Listing::Ready(list(
                        vec![
                            entry("notes/a.md", "pristine"),
                            entry("notes/b.md", "pristine"),
                            entry("raw/plate-01.csv", "pristine"),
                            entry("raw/plate-02.csv", "pristine"),
                        ],
                        4,
                        false,
                    )))
                    grouping=RwSignal::new(Grouping::BaseFolder.label().to_string())
                    collapsed=collapsed
                    search=search
                    facet=RwSignal::new(Facet::All.key().to_string())
                    on_open=Callback::new(|_: String| ())
                    on_retry=Callback::new(|()| ())
                />
            }
        });
        // `notes/` sorts first, so its disclosure is the first one drawn.
        let notes = || {
            el.query_selector("[aria-expanded]")
                .unwrap()
                .expect("the notes/ disclosure")
        };
        notes().unchecked_into::<web_sys::HtmlElement>().click();
        crate::test_support::sleep_ms(10).await;
        assert!(collapsed.get_untracked().contains("notes/"));

        type_search(&el, "plate").await;
        assert!(!text(&el).contains("notes/"), "{}", el.inner_html());
        assert!(
            collapsed.get_untracked().contains("notes/"),
            "hiding a folder does not open it"
        );

        type_search(&el, "").await;
        crate::test_support::sleep_ms(10).await;
        assert_eq!(
            notes().get_attribute("aria-expanded").as_deref(),
            Some("false"),
            "back, and still collapsed: {}",
            el.inner_html()
        );
        assert!(el.query_selector("[title='notes/a.md']").unwrap().is_none());
        assert!(titled(&el, "raw/plate-01.csv"));
    }

    /// A keystroke that leaves the same rows standing rebuilds nothing: the
    /// rows on screen are the very nodes that were there before it.
    #[wasm_bindgen_test]
    async fn a_search_that_keeps_every_row_rebuilds_none() {
        let search = RwSignal::new(String::new());
        let el = searchable(
            search,
            vec![
                entry("raw/plate-01.csv", "pristine"),
                entry("raw/plate-02.csv", "pristine"),
            ],
        );
        let before = el
            .query_selector("[title='raw/plate-01.csv']")
            .unwrap()
            .expect("the row");
        type_search(&el, "pla").await;
        type_search(&el, "plat").await;
        let after = el
            .query_selector("[title='raw/plate-01.csv']")
            .unwrap()
            .expect("the row");
        assert!(before.is_same_node(Some(&after)), "the row was rebuilt");
    }
}
