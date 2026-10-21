//! `PackageRow` stories and the packages-view scene.
//!
//! The scene is the point. A row in isolation says nothing about whether forty
//! `Latest` labels read as a list or as a wall of green, and that is the open
//! question the design has about this view.

use leptos::prelude::*;

use crate::Cell;
use crate::Scene;
use crate::Story;
use crate::kit::Blankslate;
use crate::kit::Button;
use crate::kit::ButtonVariant;
use crate::kit::Card;
use crate::kit::GroupHeading;
use crate::kit::ListToolbar;
use crate::kit::Naming;
use crate::kit::PackageRow;
use crate::kit::RowDownloads;
use crate::kit::SearchInput;
use crate::kit::SegmentedControl;
use crate::kit::Select;
use crate::kit::StateTone;
use quilt_sync_ui::pages::list_region_class;
use quilt_sync_ui::pages::rows_class;

const MINUTE: f64 = 60_000.0;
const HOUR: f64 = 60.0 * MINUTE;
const DAY: f64 = 24.0 * HOUR;

fn ago(ms: f64) -> f64 {
    js_sys::Date::now() - ms
}

/// Bucket, namespace, state words, tone, time since the last change.
type Fixture = (&'static str, &'static str, &'static str, StateTone, f64);

#[component]
pub fn PackageRowStories() -> impl IntoView {
    view! { <StatesStory /><DownloadsStory /><EdgesStory /> }
}

/// The nine states again, but on a row — which is where they are actually read, and
/// where the ragged left edge of the state column becomes visible. The queue's own
/// wordings are clauses and never render in a chip, so every label here is the
/// list's.
#[component]
fn StatesStory() -> impl IntoView {
    let states: Vec<(&str, StateTone, f64)> = vec![
        ("Latest", StateTone::Success, 2.0 * HOUR),
        ("Not the latest", StateTone::Attention, 3.0 * DAY),
        ("2 files changed", StateTone::Neutral, 8.0 * MINUTE),
        ("conflicts in 2 files", StateTone::Danger, 40.0 * MINUTE),
        ("Changed in both places", StateTone::Danger, 5.0 * HOUR),
        ("No access", StateTone::Danger, 30.0 * DAY),
        ("No S3 bucket yet", StateTone::Attention, 26.0 * HOUR),
        ("Not published yet", StateTone::Attention, 45.0 * MINUTE),
        ("Revision not published", StateTone::Attention, 9.0 * DAY),
    ];

    view! {
        <Story
            title="PackageRow"
            note="The whole row is one anchor — hover anywhere, and middle-click or \
                  Ctrl+click behave as links because it is a real href, not a click \
                  handler. No underline on hover: the background tint is the affordance \
                  and two signals for one target is noise. Tab through: one stop per row, \
                  and the ring is around the row rather than around anything inside it. \
                  \
                  The state column is right-aligned and ragged on the left, on purpose — \
                  see the last cell of the scene below for why."
        >
            {states
                .into_iter()
                .map(|(state, tone, elapsed)| {
                    view! {
                        <Cell full=true label=state>
                            <PackageRow
                                namespace="user/package-a"
                                href="#packagerow"
                                changed_at=ago(elapsed)
                                state=state
                                tone=tone
                            />
                        </Cell>
                    }
                })
                .collect_view()}
        </Story>
    }
}

/// The download markers. A v2 row only: the whole-package scope exists only there.
#[component]
fn DownloadsStory() -> impl IntoView {
    let cells: Vec<(&str, RowDownloads, &str, StateTone)> = vec![
        (
            "whole package, every file downloaded: nothing extra",
            RowDownloads::EntirePackage { to_download: 0 },
            "Latest",
            StateTone::Success,
        ),
        (
            "whole package, one file outstanding",
            RowDownloads::EntirePackage { to_download: 1 },
            "Latest",
            StateTone::Success,
        ),
        (
            "whole package, files outstanding beside local changes",
            RowDownloads::EntirePackage { to_download: 3 },
            "2 files changed",
            StateTone::Neutral,
        ),
        (
            "picked files",
            RowDownloads::IndividualFiles {
                downloaded: 12,
                total: 340,
            },
            "Latest",
            StateTone::Success,
        ),
        (
            "picked files, with a newer remote revision",
            RowDownloads::IndividualFiles {
                downloaded: 1,
                total: 58,
            },
            "Not the latest",
            StateTone::Attention,
        ),
    ];

    view! {
        <Story
            title="PackageRow — downloads"
            note="What is on disk, as an outlined badge between the time and the state label. A whole \
                  package with every file downloaded shows nothing extra; one with files \
                  outstanding shows how many; a package kept to picked files shows how \
                  many of the package's files are on disk. Hover a marker for its \
                  sentence. The state labels are unchanged."
        >
            {cells
                .into_iter()
                .map(|(label, downloads, state, tone)| {
                    view! {
                        <Cell full=true label=label>
                            <PackageRow
                                namespace="user/package-a"
                                href="#packagerow"
                                changed_at=ago(3.0 * HOUR)
                                state=state
                                tone=tone
                                downloads=downloads
                            />
                        </Cell>
                    }
                })
                .collect_view()}
            <Cell wide=true label="narrow — the namespace gives way, the marker does not">
                <PackageRow
                    namespace="team/rnaseq-batch-2026-07-31-reprocessed-v2"
                    href="#packagerow"
                    changed_at=ago(4.0 * DAY)
                    state="Changed in both places"
                    tone=StateTone::Danger
                    downloads=RowDownloads::EntirePackage { to_download: 128 }
                />
            </Cell>
        </Story>
    }
}

#[component]
fn EdgesStory() -> impl IntoView {
    view! {
        <Story
            title="PackageRow — edges"
            note="Namespaces truncate from the right, the opposite of FileRow's paths: a \
                  namespace is distinguished by its start. The time column is a 104px floor, \
                  sized to the widest phrase RelativeTime can produce in the widest system \
                  face, and never a cap. The last cell is the worst-case phrase set, and \
                  every one of them has to sit inside the column. `Changed in both places`, \
                  at 22 characters, is the widest label a list row ever carries."
        >
            <Cell full=true label="long namespace truncates right, state keeps its place">
                <PackageRow
                    namespace="team/rnaseq-batch-2026-07-31-reprocessed-v2-with-a-very-long-suffix"
                    href="#packagerow"
                    changed_at=ago(4.0 * DAY)
                    state="Changed in both places"
                    tone=StateTone::Danger
                />
            </Cell>
            <Cell full=true label="no recorded time — a word, never a blank cell">
                <PackageRow
                    namespace="user/package-a"
                    href="#packagerow"
                    state="Not published yet"
                    tone=StateTone::Attention
                />
            </Cell>
            <Cell wide=true label="narrow — two columns, roughly a narrow window">
                <PackageRow
                    namespace="team/rnaseq-batch-2026-07-31-reprocessed-v2"
                    href="#packagerow"
                    changed_at=ago(4.0 * DAY)
                    state="Changed in both places"
                    tone=StateTone::Danger
                />
            </Cell>
            <Cell wide=true label="narrow · short state — the namespace gets the slack back">
                <PackageRow
                    namespace="team/rnaseq-batch-2026-07-31-reprocessed-v2"
                    href="#packagerow"
                    changed_at=ago(2.0 * HOUR)
                    state="Latest"
                    tone=StateTone::Success
                />
            </Cell>
            <Cell full=true label="provisional — the guess dims and the label dashes; the timestamp does not, because it is not part of the guess">
                <div class="g-rows">
                    <PackageRow
                        namespace="user/package-a"
                        href="#packagerow"
                        changed_at=ago(2.0 * HOUR)
                        state="Latest"
                        tone=StateTone::Success
                        provisional=true
                    />
                    <PackageRow
                        namespace="user/package-b"
                        href="#packagerow"
                        changed_at=ago(20.0 * MINUTE)
                        state="Not the latest"
                        tone=StateTone::Attention
                        provisional=true
                    />
                    <PackageRow
                        namespace="user/package-c"
                        href="#packagerow"
                        changed_at=ago(5.0 * HOUR)
                        state="Latest"
                        tone=StateTone::Success
                    />
                </div>
            </Cell>
            <Cell full=true label="the time column against its widest phrases">
                <div class="g-rows">
                    {[
                        ("just now", 10.0 * 1000.0),
                        ("18 min ago", 18.0 * MINUTE),
                        ("23 hours ago", 23.0 * HOUR),
                        ("yesterday", 30.0 * HOUR),
                        ("3 weeks ago", 21.0 * DAY),
                        ("11 months ago", 340.0 * DAY),
                        ("3 years ago", 1100.0 * DAY),
                    ]
                        .into_iter()
                        .map(|(phrase, elapsed)| {
                            view! {
                                <PackageRow
                                    namespace=format!("expected “{phrase}”")
                                    href="#packagerow"
                                    changed_at=ago(elapsed)
                                    state="Latest"
                                    tone=StateTone::Success
                                />
                            }
                        })
                        .collect_view()}
                </div>
            </Cell>
        </Story>
    }
}

/// Forty-three packages, three of which need something — the real ratio, because
/// the ratio is what the open question is about.
fn fixtures() -> Vec<Fixture> {
    let latest = ("Latest", StateTone::Success);
    let mut rows: Vec<Fixture> = vec![
        (
            "s3://my-bucket",
            "user/package-a",
            "2 files changed",
            StateTone::Neutral,
            8.0 * MINUTE,
        ),
        (
            "s3://my-bucket",
            "user/package-b",
            "Not the latest",
            StateTone::Attention,
            20.0 * MINUTE,
        ),
        (
            "s3://my-bucket",
            "user/qc-plates-2026-08",
            latest.0,
            latest.1,
            2.0 * HOUR,
        ),
        (
            "s3://my-bucket",
            "user/scratch",
            latest.0,
            latest.1,
            5.0 * HOUR,
        ),
        (
            "s3://my-bucket",
            "user/reference-genomes",
            latest.0,
            latest.1,
            26.0 * HOUR,
        ),
        (
            "s3://team-bucket",
            "team/rnaseq-batch-2026-07-31-reprocessed-v2",
            "No access",
            StateTone::Danger,
            3.0 * DAY,
        ),
        (
            "s3://team-bucket",
            "team/imaging-cohort-b",
            "No access",
            StateTone::Danger,
            9.0 * DAY,
        ),
        (
            "s3://org-archive",
            "org/dataset-c",
            latest.0,
            latest.1,
            21.0 * DAY,
        ),
        (
            "s3://org-archive",
            "org/dataset-d",
            latest.0,
            latest.1,
            70.0 * DAY,
        ),
        // Deliberately a `user/` package in someone else's bucket. Without one, every
        // prefix here would sit in exactly one bucket and the two axes would produce
        // the same groups with different names — which would hide the reason the
        // annotation exists on one axis and not the other.
        (
            "s3://org-archive",
            "user/shared-archive",
            latest.0,
            latest.1,
            11.0 * DAY,
        ),
    ];
    // Pad the healthy bucket, because five green rows and forty green rows are
    // different design questions and only one of them is ours.
    for i in 0..8 {
        rows.push((
            "s3://my-bucket",
            match i {
                0 => "user/assay-controls",
                1 => "user/instrument-logs",
                2 => "user/pilot-2026-06",
                3 => "user/pilot-2026-07",
                4 => "user/annotations",
                5 => "user/manifests",
                6 => "user/thumbnails",
                _ => "user/exports",
            },
            latest.0,
            latest.1,
            f64::from(i + 2) * DAY,
        ));
    }
    rows
}

/// Two spellings of the same header, because `GroupHeading`'s `annotation` is
/// `#[prop(optional, into)]` and Leptos makes that setter take `impl Into<String>`
/// — so an `Option<String>` cannot be forwarded to it. Harmless here, where the
/// condition is a fixture, but the real page reads the cause off a DTO as an
/// `Option` and will hit this. Noted rather than fixed: changing the prop would
/// cost every caller that passes a literal.
fn header(title: String, count: usize, bucket_axis: bool) -> AnyView {
    // Only the bucket axis carries a cause. A prefix spans buckets — `user/` has
    // packages in two of them here — so no cause can be a property of a prefix group,
    // and the slot stays empty rather than repeating a per-row problem.
    if bucket_axis && title == "s3://team-bucket" {
        view! { <GroupHeading title=title count=count annotation="no access as analyst" /> }
            .into_any()
    } else {
        view! { <GroupHeading title=title count=count /> }.into_any()
    }
}

/// The namespace's owner segment, with its slash — `user/package-a` groups under
/// `user/`. The split is `quilt_uri::Namespace`'s; the slash is this heading's.
fn prefix(namespace: &'static str) -> String {
    let namespace = quilt_uri::Namespace::try_from(namespace).expect("a scene namespace");
    format!("{}/", namespace.prefix())
}

/// Each scene package's download scope. Most are kept to picked files: that is
/// the stored default, so every package that never chose shows `N of M files`.
fn downloads(namespace: &str) -> RowDownloads {
    let picked = |downloaded, total| RowDownloads::IndividualFiles { downloaded, total };
    match namespace {
        "user/package-a" => RowDownloads::EntirePackage { to_download: 3 },
        "user/reference-genomes" => RowDownloads::EntirePackage { to_download: 12 },
        "user/pilot-2026-07" => RowDownloads::EntirePackage { to_download: 1 },
        "user/qc-plates-2026-08" | "user/annotations" | "org/dataset-c" => {
            RowDownloads::EntirePackage { to_download: 0 }
        }
        "user/package-b" => picked(4, 4),
        "user/scratch" => picked(17, 17),
        "team/rnaseq-batch-2026-07-31-reprocessed-v2" => picked(96, 2_304),
        "team/imaging-cohort-b" => picked(12, 340),
        "org/dataset-d" => picked(1, 1),
        "user/shared-archive" => picked(0, 85),
        "user/assay-controls" => picked(31, 120),
        "user/instrument-logs" => picked(2, 1_870),
        "user/pilot-2026-06" => picked(64, 64),
        "user/manifests" => picked(9, 210),
        "user/thumbnails" => picked(150, 4_096),
        _ => picked(5, 48),
    }
}

fn row(entry: Fixture, provisional: bool, with_downloads: bool) -> AnyView {
    let (_, namespace, state, tone, elapsed) = entry;
    let marker = with_downloads.then(|| downloads(namespace));
    view! {
        <PackageRow
            namespace=namespace
            href="#scene-packages"
            changed_at=ago(elapsed)
            state=state
            tone=tone
            provisional=provisional
            downloads=marker
        />
    }
    .into_any()
}

/// The region itself, so the whole-page scene composes this code rather than a copy.
#[component]
pub fn PackagesRegion(
    /// Radio-group name for the view toggle. Unique per mounted instance: the whole-page
    /// scenes mount this region beside this file's own, and two instances sharing a
    /// name become one group with one selection between them.
    view_name: &'static str,
    /// Every row still waiting on its check, dimmed.
    #[prop(optional)]
    provisional: bool,
    /// Draw the download markers. Off in the whole-page scenes, which show the
    /// page as it ships.
    #[prop(optional)]
    downloads: bool,
) -> impl IntoView {
    let view_mode = RwSignal::new("Packages".to_string());
    let query = RwSignal::new(String::new());
    let group = RwSignal::new("Bucket".to_string());
    let sort = RwSignal::new("Changed".to_string());

    view! {
        // The page's region 4: the toolbar, then the card that scrolls under it,
        // laid out by the page's own class. The toolbar sits on the card, not in
        // it. No title on the card: the SegmentedControl names the view, and a
        // card headed `Packages` above a Packages / Recent files switch says it
        // twice.
        <div class=list_region_class()>
            <ListToolbar>
                <SegmentedControl
                    aria_label="List view"
                    name=view_name
                    options=vec!["Packages".into(), "Recent files".into()]
                    selected=view_mode
                />
                <SearchInput value=query aria_label="Search packages" placeholder="Search…" />
                <Select
                    naming=Naming::Prefix("Group".to_string())
                    options=vec![
                        "Bucket".to_string(),
                        "Prefix".to_string(),
                        "None".to_string(),
                    ]
                    selected=group
                />
                <Select
                    naming=Naming::Prefix("Sort".to_string())
                    options=vec!["Changed".to_string(), "Name".to_string()]
                    selected=sort
                />
                // Default, as the page draws it: the accent is the fresh
                // install's Blankslate action, not a toolbar's.
                <Button on_click=|_| ()>"Create package"</Button>
            </ListToolbar>
            // Headings and lists are the card's direct children, as on the page:
            // `Card` rules between them, and the list rules between its rows.
            <Card label="Packages">
                {move || {
                    let rows = fixtures();
                    let axis = group.get();
                    if axis == "None" {
                        return list(rows, provisional, downloads);
                    }
                    let bucket_axis = axis == "Bucket";
                    // One path for both axes, differing only in the key. Grouping by
                    // prefix used to fall through to the flat list, which read as
                    // "sorted by prefix" — the same rows in the same order with no
                    // headers at all.
                    let key = move |entry: &Fixture| {
                        if bucket_axis { entry.0.to_string() } else { prefix(entry.1) }
                    };
                    // First-appearance order, not sorted: the fixture order is the
                    // page's sort order, and re-sorting the groups here would hide
                    // whatever the Sort control did.
                    let mut order: Vec<String> = Vec::new();
                    for entry in &rows {
                        let k = key(entry);
                        if !order.contains(&k) {
                            order.push(k);
                        }
                    }
                    order
                        .into_iter()
                        .map(|group_key| {
                            let group_rows: Vec<_> = rows
                                .iter()
                                .copied()
                                .filter(|entry| key(entry) == group_key)
                                .collect();
                            view! {
                                {header(group_key, group_rows.len(), bucket_axis)}
                                {list(group_rows, provisional, downloads)}
                            }
                                .into_any()
                        })
                        .collect_view()
                        .into_any()
                }}
            </Card>
        </div>
    }
}

/// One group's rows as the page lists them: a `ul` with the page's rows class,
/// which rules between its items.
fn list(rows: Vec<Fixture>, provisional: bool, with_downloads: bool) -> AnyView {
    view! {
        <ul class=rows_class() role="list">
            {rows
                .into_iter()
                .map(|entry| view! { <li>{row(entry, provisional, with_downloads)}</li> })
                .collect_view()}
        </ul>
    }
    .into_any()
}

#[component]
pub fn PackagesScene() -> impl IntoView {
    view! {
        <Scene
            title="Scene · packages"
            note="Seventeen rows at the real ratio: three need something, the rest are \
                  Latest. The open question is here — does a column of Latest read as a calm \
                  list, or as a wall of green that spends the signal the tone was bought \
                  for? Switch the theme; light and dark disagree about this more than about \
                  anything else in the kit. All three Group options are live: switch between \
                  Bucket and Prefix and watch `user/shared-archive` move."
        >
            <PackagesRegion view_name="packages-view" />
        </Scene>
        <Scene
            title="Scene · packages, with download markers"
            note="The same rows with what is on disk beside each state. Most packages \
                  are kept to picked files and read `N of M files`: that is the stored \
                  default, so every package that never chose a scope shows it, and the \
                  marker is how people learn the whole-package scope exists. Some of them \
                  have every file on disk and still read as a count, which is how a \
                  picked-files package differs from a complete whole one. A few whole packages have files \
                  outstanding (`user/package-a` beside its local changes); the rest are \
                  complete and show nothing extra."
        >
            <PackagesRegion view_name="packages-downloads-view" downloads=true />
        </Scene>
        <Scene
            title="Scene · a fresh install"
            note="Zero packages, which is the FIRST thing a new user sees and the one empty \
                  state that was missing — the other two are 'no results' for a search, \
                  which is a different situation with a different answer. \
                  \
                  This one carries an action, because the user genuinely cannot proceed \
                  without one and has no way to guess it. 'No results' carries none, \
                  because they already know how to change their own search."
        >
            <Card>
                <div>
                    <Blankslate
                        heading="No packages yet"
                        description="A package is a folder of files that QuiltSync keeps in step with \
                              an S3 bucket. Create one from a folder you already have, or \
                              install an existing package from the catalog."
                        primary_action=view! {
                            <Button variant=ButtonVariant::Primary on_click=|_| ()>
                                "Create package"
                            </Button>
                        }
                            .into_any()
                    />
                </div>
            </Card>
        </Scene>
        <Scene
            title="Scene · packages, no results"
            note="One Blankslate component for both empties. No results carries no action \
                  yet — an empty list is a frustration and the way out of it is still to \
                  design, which is a deferral rather than the rule this note used to state."
        >
            <Blankslate
                heading="No packages match “plate-99”"
                description="Search covers the names of packages installed on this machine."
            />
        </Scene>
    }
}
