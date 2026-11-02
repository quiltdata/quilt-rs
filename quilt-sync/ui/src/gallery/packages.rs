//! The packages-view scene. `PackageRow`'s own stories are in `package_row.rs`.
//!
//! The scene is the point. A row in isolation says nothing about whether forty
//! `Latest` labels read as a list or as a wall of green, and that is the open
//! question the design has about this view.

use leptos::prelude::*;

use crate::Scene;
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
use quilt_sync_ui::pages::ToolbarBand;
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

/// The catalog host each scene bucket's packages point at, as the Catalog axis
/// heads them. `s3://org-archive` has none, so its packages group under
/// `No catalog`, before every host.
fn catalog(bucket: &str) -> &'static str {
    match bucket {
        "s3://my-bucket" => "catalog.example.com",
        "s3://team-bucket" => "team.example.com",
        _ => "No catalog",
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
        "user/package-b" => RowDownloads::EntirePackage { to_download: 5 },
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
        // The page's region 4: the toolbar in its band, then the list that
        // scrolls the page under it, laid out by the page's own classes. The
        // list is a bare card, as on the page. No title on it: the
        // SegmentedControl names the view, and a card headed `Packages` above a
        // Packages / Recent files switch says it twice.
        <div class=list_region_class()>
            <ToolbarBand>
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
                            "Catalog".to_string(),
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
            </ToolbarBand>
            // Headings and lists are the card's direct children, as on the page:
            // `Card` rules between them, and the list rules between its rows.
            <Card label="Packages" bare=true>
                {move || {
                    let rows = fixtures();
                    let axis = group.get();
                    if axis == "None" {
                        return list(rows, provisional, downloads);
                    }
                    let bucket_axis = axis == "Bucket";
                    // One path for every axis, differing only in the key. Grouping by
                    // prefix used to fall through to the flat list, which read as
                    // "sorted by prefix" — the same rows in the same order with no
                    // headers at all.
                    let catalog_axis = axis == "Catalog";
                    let key = move |entry: &Fixture| match axis.as_str() {
                        "Bucket" => entry.0.to_string(),
                        "Catalog" => catalog(entry.0).to_string(),
                        _ => prefix(entry.1),
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
                    // As on the page, the packages without a catalog come before every host.
                    if catalog_axis
                        && let Some(at) = order.iter().position(|k| k == "No catalog")
                    {
                        let first = order.remove(at);
                        order.insert(0, first);
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
                  anything else in the kit. Every Group option is live: switch between Bucket \
                  and Prefix and watch `user/shared-archive` move, or to Catalog to see \
                  the packages without a catalog grouped first."
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
                  picked-files package differs from a complete whole one. A few whole \
                  packages have files to download, filled in the accent so they stand out \
                  (`user/package-a` beside its local changes, `user/package-b` beside a \
                  newer remote revision); the rest are complete and show nothing extra."
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
