//! The Recent files scene. Its components' stories are in `recent_files_parts.rs`.
//!
//! The scene shows what reviewing one component alone misses: whether a path, a
//! chip, the time and a `[⋯]` fit on one line at 1200px.

use leptos::prelude::*;

use crate::Scene;
use crate::gallery::recent_files_parts::Reach;
use crate::kit::FileRow;
use crate::kit::GroupHeading;
use crate::kit::ListToolbar;
use crate::kit::MenuAction;
use crate::kit::NameTooltip;
use crate::kit::Naming;
use crate::kit::SearchInput;
use crate::kit::SegmentedControl;
use crate::kit::Select;
use crate::pages::row_menu::{RowCommand, menu_actions};

const MINUTE: f64 = 60_000.0;
const HOUR: f64 = 60.0 * MINUTE;
const DAY: f64 = 24.0 * HOUR;

fn ago(ms: f64) -> f64 {
    js_sys::Date::now() - ms
}

/// The page's items for a row, as `pages/main_page/recent_files.rs` picks them.
fn menu(reach: Reach) -> Vec<MenuAction> {
    let path = String::from("file");
    let mut commands = vec![
        RowCommand::Open(path.clone()),
        RowCommand::Reveal(path.clone()),
    ];
    if matches!(reach, Reach::Published) {
        commands.push(RowCommand::OpenInCatalog(path.clone()));
        commands.push(RowCommand::CopyUri(path));
    }
    menu_actions(&commands, Callback::new(|_| ()))
}

/// The fixture set, newest first.
fn fixtures() -> Vec<(&'static str, &'static str, f64)> {
    vec![
        (
            "runs/2026-08-04/plate-07/A01_Specimen_001_A1_A01.fcs",
            "user/package-b",
            2.0 * MINUTE,
        ),
        (
            "runs/2026-08-04/plate-07/A02_Specimen_001_A2_A02.fcs",
            "user/package-b",
            18.0 * MINUTE,
        ),
        (
            "analysis/qc/summary-by-well-normalised.parquet",
            "org/dataset-c",
            41.0 * MINUTE,
        ),
        (
            "notebooks/2026-08-04-qc-review-cohort-b.ipynb",
            "org/dataset-c",
            1.5 * HOUR,
        ),
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

/// The scene's packages: one local only, one with a revision not yet
/// published, the rest published.
fn reach(package: &str) -> Reach {
    match package {
        "user/package-a" => Reach::LocalOnly,
        "user/package-b" => Reach::Ahead,
        _ => Reach::Published,
    }
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
            actions=menu(reach(package))
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
            note="The Group select is live — switch it to Package to see GroupHeading in \
                  context, which is the only place this view uses one. Note what grouping \
                  costs: flat, the newest file is the top row; grouped, it is the top row of \
                  the first group, and a single recent file in the third group sits below \
                  older ones. That is why None is the default. \
                  \
                  Paths truncate from the LEFT so the filename survives. Tab into the list: \
                  the row is one stop, then the package chip, then its [⋯]. user/package-a \
                  is local only and user/package-b has a revision not yet published, so \
                  neither offers Open in catalog."
        >
            <ListToolbar>
                <SegmentedControl
                    aria_label="List view"
                    name="recent-files-view"
                    options=vec!["Packages".into(), "Recent files".into()]
                    selected=view_mode
                />
                <SearchInput value=query aria_label="Search files" placeholder="Search…" />
                <Select
                    naming=Naming::Prefix("Group".to_string())
                    options=vec!["None".to_string(), "Package".to_string()]
                    selected=group
                />
            </ListToolbar>
            <NameTooltip>
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
                                <GroupHeading title=package count=rows.len() />
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
            </NameTooltip>
        </Scene>
    }
}
