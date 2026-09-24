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
//! right-hand group. Grouping and the facets are drawn. The other slots are left
//! absent rather than drawn inert — a control that does nothing is worse than
//! one that is not there yet.
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
//!   The box is the row's own until the selection slice gives the pane one.
//! - **The cap is stated.** Over the cap the backend says so, and the pane
//!   says how many files the package has. The flag decides, never a length.

use leptos::prelude::*;

use crate::commands::{EntryCounts, EntryData, InstalledPackageData};
use crate::kit::state_label::StateTone;
use crate::kit::{
    Blankslate, Card, EntryAction, EntryGroup, EntryRow, EntrySelection, ListToolbar, LoadFailure,
    Naming, Segment, SegmentedControl, Select, SkeletonBox,
};
use crate::util::format_size;

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

impl From<InstalledPackageData> for FileList {
    fn from(data: InstalledPackageData) -> Self {
        Self {
            entries: data.entries,
            counts: data.counts,
            total: data.total,
            truncated: data.truncated,
        }
    }
}

/// Where the pane's read stands.
#[derive(Clone, Debug)]
pub enum Listing {
    Loading,
    Failed,
    Ready(FileList),
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
#[derive(Clone)]
struct Row {
    path: String,
    size: String,
    place: Place,
    /// Matched by `.quiltignore`. Drawn only under [`Facet::Ignored`], where
    /// the view already says it, so the row carries no label and no click.
    ignored: bool,
}

/// One row. `name` is what the list shows — the whole path, or the part under
/// a heading — and the whole path is always the `title`.
fn row(r: &Row, name: String, on_open: Callback<String>) -> AnyView {
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
        // The row's own tick. The selection slice replaces it with the pane's.
        Place::Missing => {
            let ticked = RwSignal::new(false);
            Some(EntryAction::Select(EntrySelection::new(
                ticked,
                Callback::new(move |next| ticked.set(next)),
            )))
        }
        Place::Deleted | Place::Unknown => None,
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

/// The rows, grouped as the select says.
fn rows_view(rows: &[Row], grouping: Grouping, on_open: Callback<String>) -> AnyView {
    let paths: Vec<&str> = rows.iter().map(|r| r.path.as_str()).collect();
    let items = group(&paths, grouping);
    // A list with no heading at all has no disclosure to align to, so the
    // gutter the rows keep for one is zero (`EntryRow`'s module doc).
    let headed = items.iter().any(|i| matches!(i, Item::Group { .. }));
    let drawn = items
        .into_iter()
        .map(|item| match item {
            Item::Row { index, name } => row(&rows[index], name, on_open),
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
                let members = StoredValue::new(members);
                view! {
                    <EntryGroup
                        name=heading
                        count=Signal::stored(count)
                        open=RwSignal::new(true)
                    >
                        {move || {
                            members
                                .with_value(|ms| {
                                    ms.iter()
                                        .map(|(r, name)| row(r, name.clone(), on_open))
                                        .collect_view()
                                })
                        }}
                    </EntryGroup>
                }
                .into_any()
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

/// The rows the facet leaves, in path order. The one place the pane narrows
/// its rows, so what the list draws and what select-all ticks cannot disagree:
/// the search adds its test here.
fn shown_rows(rows: &[Row], facet: Facet) -> Vec<Row> {
    rows.iter()
        .filter(|r| facet.admits(r.place, r.ignored))
        .cloned()
        .collect()
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

/// The toolbar under the search row: select-all's slot on the left, the view
/// controls on the right. The facets need the package's counts, so a pane
/// with no answer draws grouping alone.
fn toolbar(grouping: RwSignal<String>, facets: Option<AnyView>) -> impl IntoView {
    view! {
        // Stacks upwards, so the line nearest the rows is the one acting on
        // them. Select-all (the selection slice) goes first, on the left.
        <ListToolbar reverse_when_stacked=true>
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
    /// The chosen [`Facet`], as its [`Facet::key`]. The page owns it, for the
    /// grouping's reason.
    facet: RwSignal<String>,
    /// Open a downloaded file, by its logical path.
    on_open: Callback<String>,
    /// Read the list again after a failure.
    on_retry: Callback<()>,
) -> impl IntoView {
    move || match listing.get() {
        Listing::Loading => view! { <FilePaneSkeleton /> }.into_any(),
        Listing::Failed => view! {
            <section class=style::root aria-label="Files">
                {toolbar(grouping, None)}
                <Card flush=true label="Files" fill=true>
                    <LoadFailure
                        centred=true
                        words="Could not read this package's files."
                        on_retry=on_retry
                    />
                </Card>
            </section>
        }
        .into_any(),
        Listing::Ready(list) => ready(list, grouping, facet, on_open),
    }
}

fn ready(
    list: FileList,
    grouping: RwSignal<String>,
    facet: RwSignal<String>,
    on_open: Callback<String>,
) -> AnyView {
    let FileList {
        entries,
        counts,
        total,
        truncated,
    } = list;
    let loaded = entries.len();
    let rows: Vec<Row> = entries
        .into_iter()
        .map(|e| Row {
            place: Place::of(&e.status),
            size: format_size(e.size),
            ignored: e.ignored_by.is_some(),
            path: e.filename,
        })
        .collect();
    let rows = StoredValue::new(rows);
    let shown = Signal::derive(move || {
        let f = Facet::from_key(&facet.get());
        rows.with_value(|rs| shown_rows(rs, f))
    });

    let body = move || {
        if total == 0 {
            return view! {
                <Blankslate
                    heading="Nothing in this package"
                    description="The published revision has no files in it yet."
                />
            }
            .into_any();
        }
        let g = Grouping::from_label(&grouping.get());
        shown.with(|rs| {
            if !rs.is_empty() {
                return rows_view(rs, g, on_open);
            }
            // Empty, and never zero-count unless it is `All`: an inert facet
            // cannot be chosen. So `All` at zero is a package all ignored,
            // and any other empty view counts files the cap left out.
            if Facet::from_key(&facet.get()) == Facet::All && counts.all == 0 {
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
            {toolbar(grouping, Some(facets(&counts, facet).into_any()))}
            <Card flush=true label="Files" fill=true>
                {truncated.then(|| view! { <CapNotice total=total shown=loaded /> })}
                {body}
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
mod facet_tests {
    use super::*;

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
    use crate::test_support::{element_saying, mount};
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

        let failed = pane(Listing::Failed);
        element_saying(&failed, "Could not read this package's files.");
    }

    #[wasm_bindgen_test]
    fn an_empty_package_says_so() {
        let el = pane(Listing::Ready(list(Vec::new(), 0, false)));
        element_saying(&el, "Nothing in this package");
    }
}
