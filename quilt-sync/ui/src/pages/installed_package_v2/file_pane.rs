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
//! right-hand group. This slice ships grouping. The other slots are left
//! absent rather than drawn inert — a control that does nothing is worse than
//! one that is not there yet.
//!
//! # What this slice draws
//!
//! - **Rows, silent at rest.** `Downloaded` carries no label; `Changed`, `New`,
//!   `Deleted` and `Not downloaded` do (design §3's table).
//! - **Ignored files are hidden.** The `Ignored` facet is the only view that
//!   shows them, and it has not landed.
//! - **A click does the thing the row can do.** A file that is here opens, a
//!   missing one is `EntryRow`'s `Select` shape, and a deleted one is inert.
//!   The box is the row's own until the selection slice gives the pane one.
//! - **The cap is stated.** Over the cap the backend says so, and the pane
//!   says how many files the package has. The flag decides, never a length.

use leptos::prelude::*;

use crate::commands::{EntryData, InstalledPackageData};
use crate::kit::state_label::StateTone;
use crate::kit::{
    Blankslate, Card, EntryAction, EntryGroup, EntryRow, EntrySelection, ListToolbar, LoadFailure,
    Naming, Select, SkeletonBox,
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
    /// Every file the package has, before the cap.
    pub total: usize,
    /// The backend cut the list. Never inferred from `entries.len()`.
    pub truncated: bool,
}

impl From<InstalledPackageData> for FileList {
    fn from(data: InstalledPackageData) -> Self {
        Self {
            entries: data.entries,
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
#[derive(Clone, Copy, PartialEq, Eq)]
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
/// Page-local and not a kit piece: not a `Banner`, which reports an outcome
/// for the whole page, and not a `ZeroLine`, which is the healthy queue's
/// one-liner. It sits inside the list's box, above the rows it qualifies.
#[component]
fn CapNotice(total: usize, shown: usize) -> impl IntoView {
    view! {
        <p class=style::cap>
            {format!(
                "This package has {} files. Showing the first {}.",
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
}

/// One row. `name` is what the list shows — the whole path, or the part under
/// a heading — and the whole path is always the `title`.
fn row(r: &Row, name: String, on_open: Callback<String>) -> AnyView {
    let state = r.place.state();
    let words = state.map(|(words, _)| words.to_string());
    let tone = state.map_or(StateTone::Neutral, |(_, tone)| tone);
    let action = match r.place {
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

/// The toolbar under the search row: select-all's slot on the left, the view
/// controls on the right.
fn toolbar(grouping: RwSignal<String>) -> impl IntoView {
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
                // The facets (their own slice) follow here, flush right.
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
    /// Open a downloaded file, by its logical path.
    on_open: Callback<String>,
    /// Read the list again after a failure.
    on_retry: Callback<()>,
) -> impl IntoView {
    move || match listing.get() {
        Listing::Loading => view! { <FilePaneSkeleton /> }.into_any(),
        Listing::Failed => view! {
            <section class=style::root aria-label="Files">
                {toolbar(grouping)}
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
        Listing::Ready(list) => ready(list, grouping, on_open),
    }
}

fn ready(list: FileList, grouping: RwSignal<String>, on_open: Callback<String>) -> AnyView {
    let FileList {
        entries,
        total,
        truncated,
    } = list;
    let shown = entries.len();
    let rows: Vec<Row> = entries
        .into_iter()
        .filter(|e| e.ignored_by.is_none())
        .map(|e| Row {
            place: Place::of(&e.status),
            size: format_size(e.size),
            path: e.filename,
        })
        .collect();

    let body = if rows.is_empty() {
        if total == 0 {
            view! {
                <Blankslate
                    heading="Nothing in this package"
                    description="The published revision has no files in it yet."
                />
            }
            .into_any()
        } else {
            view! {
                <Blankslate
                    compact=true
                    heading="No files to show"
                    description="Every file in this package is ignored."
                />
            }
            .into_any()
        }
    } else {
        let rows = StoredValue::new(rows);
        (move || {
            let g = Grouping::from_label(&grouping.get());
            rows.with_value(|rs| rows_view(rs, g, on_open))
        })
        .into_any()
    };

    view! {
        <section class=style::root aria-label="Files">
            {toolbar(grouping)}
            <Card flush=true label="Files" fill=true>
                {truncated.then(|| view! { <CapNotice total=total shown=shown /> })}
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
                // The `Group:` select's width.
                <SkeletonBox width="166px" height="32px" />
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
mod pane_tests {
    use super::*;
    use crate::commands::EntryData;
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

    fn list(entries: Vec<EntryData>, total: usize, truncated: bool) -> FileList {
        FileList {
            entries,
            total,
            truncated,
        }
    }

    fn pane(listing: Listing) -> web_sys::Element {
        mount(move || {
            view! {
                <FilePane
                    listing=Signal::stored(listing)
                    grouping=RwSignal::new(Grouping::BaseFolder.label().to_string())
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
            text(&cut).contains("This package has 4,312 files. Showing the first 2."),
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
