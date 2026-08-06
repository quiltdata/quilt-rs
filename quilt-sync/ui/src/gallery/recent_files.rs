//! Stories and the scene for the Recent files view.
//!
//! Everything this view needs lives here, because the components exist for it and
//! reviewing one in isolation from the others misses the interesting failures —
//! whether a path, a chip and three icons fit on one line at 1200px, and whether
//! revealing the icons shifts anything.

use leptos::prelude::*;

use crate::Cell;
use crate::Scene;
use crate::Story;
use crate::kit::EmptyState;
use crate::kit::FileRow;
use crate::kit::GroupHeader;
use crate::kit::IconButton;
use crate::kit::IconButtonVariant;
use crate::kit::ListToolbar;
use crate::kit::RelativeTime;
use crate::kit::SearchInput;
use crate::kit::Select;
use crate::kit::ViewToggle;

const MINUTE: f64 = 60_000.0;
const HOUR: f64 = 60.0 * MINUTE;
const DAY: f64 = 24.0 * HOUR;

fn ago(ms: f64) -> f64 {
    js_sys::Date::now() - ms
}

fn refresh_icon() -> AnyView {
    view! {
        <svg viewBox="0 0 16 16" aria-hidden="true" fill="none" stroke="currentColor"
            stroke-width="1.4" stroke-linecap="round">
            <path d="M13.5 8a5.5 5.5 0 1 1-1.9-4.15" />
            <path d="M13.6 1.9v2.4h-2.4" />
        </svg>
    }
    .into_any()
}

fn gear_icon() -> AnyView {
    view! {
        <svg viewBox="0 0 16 16" aria-hidden="true" fill="none" stroke="currentColor"
            stroke-width="1.4">
            <circle cx="8" cy="8" r="2.1" />
            <path d="M8 1.6v1.7M8 12.7v1.7M2.5 8H4.2M11.8 8h1.7M4.1 4.1l1.2 1.2M10.7 10.7l1.2 1.2M11.9 4.1l-1.2 1.2M5.3 10.7l-1.2 1.2" />
        </svg>
    }
    .into_any()
}

#[component]
pub fn RecentFilesStories() -> impl IntoView {
    view! { <Rows /> <Times /> <Icons /> <Groups /> <Empties /> }
}

/// `FileRow` had no story until now — it existed only inside the scene, which meant
/// its edges could not be reviewed and it could not be put beside a `PackageRow` at
/// the same width. Full-width cells, because a row in a 530px cell answers a
/// question the page never asks.
#[component]
fn Rows() -> impl IntoView {
    let row = |path: &'static str, package: &'static str, elapsed: f64| {
        view! {
            <FileRow
                path=path
                package=package
                package_href="#recent-files-parts"
                at=ago(elapsed)
                on_open=|_| ()
                on_reveal=|_| ()
                on_open_catalog=|_| ()
                on_copy_uri=|_| ()
            />
        }
    };

    view! {
        <Story
            title="FileRow"
            note="Compare against PackageRow directly above — same width, opposite \
                  truncation. A path truncates from the LEFT so the filename survives; a \
                  namespace truncates from the right. This row is a div with \
                  role=\"button\", not an anchor, because it has three actions and a second \
                  link inside it and nested anchors are invalid. Both rows share the 80px \
                  time column, which is the thing to check here: if a phrase ellipsises, \
                  the column wants 96px."
        >
            <Cell full=true label="ordinary">
                {row("analysis/qc/summary-by-well.parquet", "org/dataset-c", 41.0 * MINUTE)}
            </Cell>
            <Cell full=true label="deep path — truncates left, filename survives">
                {row(
                    "runs/2026-08-04/plate-07/wells/row-a/A01_Specimen_001_A1_A01.fcs",
                    "user/package-b",
                    2.0 * MINUTE,
                )}
            </Cell>
            <Cell full=true label="long namespace — the chip caps at 30% and truncates too">
                {row(
                    "derived/2026-08/counts_matrix_filtered_log1p.h5ad",
                    "team/rnaseq-batch-2026-07-31-reprocessed-v2-and-then-some",
                    5.0 * HOUR,
                )}
            </Cell>
            <Cell full=true label="short path — nothing truncates, actions still sit right">
                {row("README.md", "user/package-a", 3.0 * HOUR)}
            </Cell>
            <Cell wide=true label="narrow — two columns, roughly a narrow window">
                {row(
                    "runs/2026-08-04/plate-07/wells/row-a/A01_Specimen_001_A1_A01.fcs",
                    "user/package-b",
                    2.0 * MINUTE,
                )}
            </Cell>
            <Cell wide=true label="narrow · long namespace — chip and path compete for the row">
                {row(
                    "derived/2026-08/counts_matrix_filtered_log1p.h5ad",
                    "team/rnaseq-batch-2026-07-31-reprocessed-v2-and-then-some",
                    5.0 * HOUR,
                )}
            </Cell>
            // The expected phrase is spelled in words, not digits. An earlier version
            // used "23 hours ago" as the path and it rendered as "hours ago 23" — the
            // `direction: rtl` that moves the ellipsis to the left also reorders a
            // leading number under the bidi algorithm. The next cell keeps a real
            // case of that hazard on the page instead of hiding it.
            <Cell full=true label="the 80px time column against its worst cases">
                <div class="g-rows">
                    {[
                        ("check/just-now.txt", 10.0 * 1000.0),
                        ("check/twenty-three-hours-ago.txt", 23.0 * HOUR),
                        ("check/three-weeks-ago.txt", 21.0 * DAY),
                        ("check/two-months-ago.txt", 70.0 * DAY),
                    ]
                        .into_iter()
                        .map(|(path, elapsed)| row(path, "user/package-a", elapsed))
                        .collect_view()}
                </div>
            </Cell>
            <Cell full=true label="paths that START with a number — the rtl truncation hazard">
                <div class="g-rows">
                    {["2026/08/04-summary.csv", "2026-08-04-run.log", "0001_plate.fcs"]
                        .into_iter()
                        .map(|path| row(path, "user/package-a", 3.0 * HOUR))
                        .collect_view()}
                </div>
            </Cell>
        </Story>
    }
}

#[component]
fn Times() -> impl IntoView {
    view! {
        <Story
            title="RelativeTime"
            note="Coarse buckets on purpose — nobody reads a file list to learn something \
                  changed 43 minutes ago rather than 44. Hover for the exact local time; that \
                  value is also in a `datetime` attribute, so it travels with the row rather \
                  than only existing in a tooltip. It does not tick: forty rows would want \
                  forty timers, and the page re-renders on every status event anyway."
        >
            <Cell label="seconds">
                <RelativeTime at=ago(20.0 * 1000.0) />
            </Cell>
            <Cell label="minutes">
                <RelativeTime at=ago(18.0 * MINUTE) />
            </Cell>
            <Cell label="hours">
                <RelativeTime at=ago(5.0 * HOUR) />
            </Cell>
            <Cell label="yesterday">
                <RelativeTime at=ago(30.0 * HOUR) />
            </Cell>
            <Cell label="days">
                <RelativeTime at=ago(4.0 * DAY) />
            </Cell>
            <Cell label="weeks">
                <RelativeTime at=ago(23.0 * DAY) />
            </Cell>
            <Cell label="months">
                <RelativeTime at=ago(120.0 * DAY) />
            </Cell>
        </Story>
    }
}

#[component]
fn Icons() -> impl IntoView {
    view! {
        <Story
            title="IconButton"
            note="Framed for chrome, Bare for row actions. Bare does not hide itself at rest — \
                  revealing row actions depends on the row's hover state, which this component \
                  cannot see, so FileRow owns that."
        >
            <Cell label="framed — the appbar's pair">
                <div class="g-inline">
                    <IconButton icon=refresh_icon() aria_label="Refresh" on_click=|_| () />
                    <IconButton icon=gear_icon() aria_label="Settings" on_click=|_| () />
                </div>
            </Cell>
            <Cell label="framed · spinning — a fetch in flight">
                <IconButton icon=refresh_icon() aria_label="Refreshing" on_click=|_| () spinning=true />
            </Cell>
            <Cell label="framed · disabled">
                <IconButton icon=refresh_icon() aria_label="Refresh" on_click=|_| () disabled=true />
            </Cell>
            <Cell label="bare — visible here, because no row is hiding it">
                <IconButton
                    icon=gear_icon()
                    aria_label="Settings"
                    variant=IconButtonVariant::Bare
                    on_click=|_| ()
                />
            </Cell>
        </Story>
    }
}

#[component]
fn Groups() -> impl IntoView {
    view! {
        <Story
            title="GroupHeader"
            note="The count is always shown, including one — hiding it would make a \
                  single-row group look like a header with a bug. The annotation exists so a \
                  shared cause can be stated once per group instead of on thirty rows; only \
                  the bucket axis has one, since a prefix spans buckets."
        >
            <Cell wide=true label="plain — the files view, grouped by package">
                <GroupHeader title="user/package-b" count=3 />
            </Cell>
            <Cell wide=true label="count of one">
                <GroupHeader title="Local only" count=1 />
            </Cell>
            <Cell wide=true label="annotated — the packages view's bucket axis, in the attention tone">
                <GroupHeader title="s3://team-bucket" count=3 annotation="no access as analyst" />
            </Cell>
            <Cell wide=true label="long title truncates, count survives">
                <GroupHeader
                    title="s3://quilt-enterprise-eu-west-1-vir-biotechnology-in-progress"
                    count=14
                />
            </Cell>
        </Story>
    }
}

#[component]
fn Empties() -> impl IntoView {
    view! {
        <Story
            title="EmptyState"
            note="One component for both empties. No results has no action — the user already \
                  knows how to change their search, so a button there would be filler."
        >
            <Cell wide=true label="no results — no action">
                <EmptyState
                    title="No files match “plate-99”"
                    body="Search covers the paths of files you have locally. Files that exist \
                          only in a bucket are not included."
                />
            </Cell>
            <Cell wide=true label="nothing yet — with an action">
                <EmptyState
                    title="No recent changes"
                    body="Files appear here as they arrive from your team or as you edit and \
                          publish them."
                    action=view! {
                        <ViewToggle
                            label="List view"
                            name="empty-view"
                            options=vec!["Packages".to_string(), "Recent files".to_string()]
                            selected=RwSignal::new("Packages".to_string())
                        />
                    }
                        .into_any()
                />
            </Cell>
        </Story>
    }
}

/// The fixture set, newest first.
fn fixtures() -> Vec<(&'static str, &'static str, f64)> {
    vec![
        ("runs/2026-08-04/plate-07/A01_Specimen_001_A1_A01.fcs", "user/package-b", 2.0 * MINUTE),
        ("runs/2026-08-04/plate-07/A02_Specimen_001_A2_A02.fcs", "user/package-b", 18.0 * MINUTE),
        ("analysis/qc/summary-by-well-normalised.parquet", "org/dataset-c", 41.0 * MINUTE),
        ("notebooks/2026-08-04-qc-review-cohort-b.ipynb", "org/dataset-c", 1.5 * HOUR),
        ("README.md", "user/package-b", 3.0 * HOUR),
        (
            "derived/2026-08/counts_matrix_filtered_log1p.h5ad",
            "team/rnaseq-batch-2026-07-31-reprocessed-v2",
            5.0 * HOUR,
        ),
        ("metadata/samples.csv", "user/package-a", 30.0 * HOUR),
        (
            "raw/2026-07-31/L001/Undetermined_S0_L001_R1_001.fastq.gz",
            "team/rnaseq-batch-2026-07-31-reprocessed-v2",
            4.0 * DAY,
        ),
    ]
}

fn row(entry: (&'static str, &'static str, f64)) -> AnyView {
    let (path, package, elapsed) = entry;
    view! {
        <FileRow
            path=path
            package=package
            package_href="#scene-recent-files"
            at=ago(elapsed)
            on_open=|_| ()
            on_reveal=|_| ()
            on_open_catalog=|_| ()
            on_copy_uri=|_| ()
        />
    }
    .into_any()
}

#[component]
pub fn RecentFilesScene() -> impl IntoView {
    let view_mode = RwSignal::new("Recent files".to_string());
    let query = RwSignal::new(String::new());
    let group = RwSignal::new("None".to_string());

    view! {
        <Scene
            title="Scene · recent files"
            note="The Group select is live — switch it to Package to see GroupHeader in \
                  context, which is the only place this view uses one. Note what grouping \
                  costs: flat, the newest file is the top row; grouped, it is the top row of \
                  the first group, and a single recent file in the third group sits below \
                  older ones. That is why None is the default. \
                  \
                  Paths truncate from the LEFT so the filename survives. Tab into the list: \
                  the row is one stop, then the package chip, then the three actions."
        >
            <ListToolbar>
                <ViewToggle
                    label="List view"
                    name="recent-files-view"
                    options=vec!["Packages".to_string(), "Recent files".to_string()]
                    selected=view_mode
                />
                <SearchInput value=query label="Search files" placeholder="Search…" />
                <Select
                    label="Group"
                    options=vec!["None".to_string(), "Package".to_string()]
                    selected=group
                    visible_label=true
                />
            </ListToolbar>
            {move || {
                let files = fixtures();
                if group.get() == "Package" {
                    let mut order: Vec<&'static str> = Vec::new();
                    for (_, package, _) in &files {
                        if !order.contains(package) {
                            order.push(package);
                        }
                    }
                    order
                        .into_iter()
                        .map(|package| {
                            let rows: Vec<_> = files
                                .iter()
                                .copied()
                                .filter(|(_, p, _)| *p == package)
                                .collect();
                            view! {
                                <GroupHeader title=package count=rows.len() />
                                {rows.into_iter().map(row).collect_view()}
                            }
                                .into_any()
                        })
                        .collect_view()
                        .into_any()
                } else {
                    files.into_iter().map(row).collect_view().into_any()
                }
            }}
        </Scene>
    }
}
