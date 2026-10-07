//! The commit page v2, drawn before it is routed.
//!
//! Every region is `pages::commit_v2`'s own, over fixture props, so the routed
//! page and this scene cannot drift. Callbacks are dropped.
//!
//! # Measured at 1024x560, in Chrome
//!
//! | | header ends | message ends | metadata ends | files start | rows on screen |
//! |---|---:|---:|---:|---:|---:|
//! | at rest, 4 files | 144 | 217 | 361 | 377 | 4 of 4 |
//! | 300 files | 144 | 217 | 361 | 377 | **4** of 300 |
//! | no bucket, so no workflow to pick | 144 | 217 | 349 | 365 | 4 of 4 |
//! | both values from your publish settings | 144 | 217 | 410 | 426 | 3 of 4 |
//! | junk banner, signed out | 144 | 283 | 427 | 443 | 2 of 4 |
//! | no access | 144 | 271 | 415 | 431 | 2 of 4 |
//! | the metadata editor open | 144 | 217 | 491 | 507 | 0 |
//! | a failed check | 144 | 217 | 515 | 531 | 0 |
//!
//! - The form stays on the first screen in every cell; a long list scrolls
//!   under it.
//! - A failed check's error ends at 515, inside the window. That sets the
//!   metadata editor's height at 150px; at 220 the error fell to 613.
//! - The editor's context menu is not cropped: its one clipping ancestor is
//!   the page's scroller, and the menu fits in it.
//!
//! # Left for the port
//!
//! - The editor's context menu cuts some of its own labels short (`Edit valu`),
//!   at its own 16px.
//! - The `Metadata` label names the editor's hidden fallback textarea, as on v1.
//! - File rows keep `EntryRow`'s empty leading column, about 60px.
//! - The loading skeleton's header is 4px shorter than the real one.

use leptos::prelude::*;

use crate::Cell;
use crate::Scene;
use crate::kit::Button;
use crate::kit::PageLayout;
use crate::kit::icons;
use crate::pages::commit_v2::{
    Change, CommitColumn, CommitHeader, CommitPageSkeleton, IncludedFile, IncludedList,
    MessageField, MetadataField, Primary, PrimaryWiring, Problem, ProblemBanner, WorkflowChoice,
    WorkflowField,
};

const NAMESPACE: &str = "user/plate-07";
const GENERATED: &str = "Updated 3 files: plate-03.csv, plate-04.csv, wells.csv";
/// More fields than the folded line holds.
const LONG_METADATA: &str = "{\"assay\": \"ELISA\", \"plate\": 7, \"instrument\": \"SpectraMax iD5\", \
    \"wavelength_nm\": 450, \"wells\": [\"A1\", \"A2\", \"A3\"], \"protocol\": {\"version\": 3, \
    \"incubation_min\": 60}, \"notes\": \"Second read after recalibration\"}";
const METADATA: &str = "{\n  \"assay\": \"ELISA\",\n  \"plate\": 7\n}";

fn appbar_actions() -> AnyView {
    view! {
        <Button leading_visual=icons::sync() on_click=|_| ()>
            "Refresh"
        </Button>
        <Button leading_visual=icons::gear() on_click=|_| ()>
            "Settings"
        </Button>
    }
    .into_any()
}

fn few_files() -> Vec<IncludedFile> {
    vec![
        IncludedFile {
            path: "raw/plate-03.csv".to_string(),
            size: 4_200_000,
            change: Change::Changed,
        },
        IncludedFile {
            path: "raw/plate-04.csv".to_string(),
            size: 4_310_000,
            change: Change::New,
        },
        IncludedFile {
            path: "notes/wells.csv".to_string(),
            size: 18_400,
            change: Change::Changed,
        },
        IncludedFile {
            path: "raw/plate-01-old.csv".to_string(),
            size: 3_900_000,
            change: Change::Deleted,
        },
    ]
}

fn many_files() -> Vec<IncludedFile> {
    (1..=300_u64)
        .map(|i| IncludedFile {
            path: format!("raw/run-12/well-{i:03}.fastq.gz"),
            size: 1_000_000 + (i * 7_919) % 900_000,
            change: if i % 17 == 0 {
                Change::Changed
            } else {
                Change::New
            },
        })
        .collect()
}

/// What differs between the cells.
struct Fixture {
    id: &'static str,
    primary: Primary,
    files: Vec<IncludedFile>,
    ignored: usize,
    problem: Option<Problem>,
    blocked: Option<&'static str>,
    editing: bool,
    workflows: bool,
    metadata: &'static str,
    metadata_error: Option<&'static str>,
    /// (workflow, metadata)
    from_settings: (bool, bool),
    no_workflow: &'static str,
}

impl Fixture {
    fn new(id: &'static str) -> Self {
        let files = few_files();
        Self {
            id,
            primary: Primary::Files(files.len()),
            files,
            ignored: 2,
            problem: None,
            blocked: None,
            editing: false,
            workflows: true,
            metadata: METADATA,
            metadata_error: None,
            from_settings: (false, false),
            no_workflow: "None",
        }
    }
}

fn page(f: Fixture) -> AnyView {
    let Fixture {
        id,
        primary,
        files,
        ignored,
        problem,
        blocked,
        editing,
        workflows,
        metadata,
        metadata_error,
        from_settings,
        no_workflow,
    } = f;
    let w = PrimaryWiring {
        choice: RwSignal::new(0),
        on_publish: Callback::new(|()| ()),
        on_save: Callback::new(|()| ()),
        blocked: Signal::stored(blocked.map(str::to_string)),
        running: Signal::stored(false),
    };
    let workflow = workflows.then(|| WorkflowChoice {
        options: vec![
            "Bucket's default (Plate reads)".to_string(),
            "Plate reads".to_string(),
            "Instrument export".to_string(),
            "No workflow".to_string(),
        ],
        selected: RwSignal::new("Bucket's default (Plate reads)".to_string()),
    });
    let problem = problem.map(|problem| {
        let sign_in = matches!(problem, Problem::SignedOut { host: Some(_) })
            .then(|| format!("#{id}"));
        match sign_in {
            Some(href) => view! {
                <ProblemBanner problem=problem on_ignore_junk=Callback::new(|()| ()) sign_in_href=href />
            }
            .into_any(),
            None => view! { <ProblemBanner problem=problem on_ignore_junk=Callback::new(|()| ()) /> }
                .into_any(),
        }
    });
    let workflow_view = view! {
        <WorkflowField
            workflow=workflow
            no_workflow=no_workflow
            from_settings=from_settings.0
            settings_href=format!("#{id}")
        />
    }
    .into_any();
    let metadata_view = view! {
        <MetadataField
            metadata=RwSignal::new(metadata.to_string())
            editing=RwSignal::new(editing)
            error=metadata_error.map(str::to_string)
            from_settings=from_settings.1
            settings_href=format!("#{id}")
        />
    }
    .into_any();

    view! {
        <div id=id class="g-window" style="width:1024px; --q-frame-height:560px; max-width:100%">
            <PageLayout heading="QuiltSync" banner=().into_any() actions=appbar_actions()>
                <CommitColumn
                    header=view! {
                        <CommitHeader
                            namespace=NAMESPACE
                            // The cell itself, so a click does not move the page.
                            package_href=format!("#{id}")
                            home_href=format!("#{id}")
                            primary=primary
                            w=w
                            on_open_folder=Callback::new(|()| ())
                        />
                    }
                        .into_any()
                    problem=problem.unwrap_or_else(|| ().into_any())
                    message=view! {
                        <MessageField value=RwSignal::new(String::new()) placeholder=GENERATED />
                    }
                        .into_any()
                    workflow=workflow_view
                    metadata=metadata_view
                    included=view! {
                        <IncludedList
                            files=files
                            ignored=ignored
                            on_ignore=Callback::new(|_: String| ())
                        />
                    }
                        .into_any()
                />
            </PageLayout>
        </div>
    }
    .into_any()
}

fn loading() -> AnyView {
    view! {
        <div class="g-window" style="width:1024px; --q-frame-height:560px; max-width:100%">
            <PageLayout heading="QuiltSync" banner=().into_any() actions=appbar_actions()>
                <CommitPageSkeleton />
            </PageLayout>
        </div>
    }
    .into_any()
}

const NOTE: &str = "The commit page v2 at the app's 1024×560 floor, one cell per state: header, \
    problem, message, workflow, metadata, then the changed files, which scroll under the form. \
    Not routed yet.";

const FAILED_CHECK: &str = "The workflow requires an \"operator\" field in the metadata";

#[component]
pub fn CommitPageScene() -> impl IntoView {
    view! {
        <Scene title="The commit page v2" note=NOTE>
            <Cell full=true label="files changed, at rest">
                {page(Fixture::new("commit-rest"))}
            </Cell>
            <Cell
                full=true
                label="workflow and metadata from your publish settings — each field says so, and \
                       only then"
            >
                {page(Fixture { from_settings: (true, true), ..Fixture::new("commit-from-settings") })}
            </Cell>
            <Cell full=true label="metadata only — no file changes, so the primary publishes the revision">
                {page(Fixture {
                    primary: Primary::MetadataOnly,
                    files: Vec::new(),
                    ignored: 0,
                    metadata: "{\"assay\": \"ELISA\", \"plate\": 7, \"reviewed\": true}",
                    from_settings: (false, true),
                    ..Fixture::new("commit-metadata-only")
                })}
            </Cell>
            <Cell full=true label="a local-only package — Save revision, no caret, no workflows to choose">
                {page(Fixture {
                    primary: Primary::LocalOnly,
                    workflows: false,
                    no_workflow: "None — this package has no bucket yet",
                    ..Fixture::new("commit-local-only")
                })}
            </Cell>
            <Cell full=true label="no metadata — the absence muted, and the button reads Add">
                {page(Fixture { metadata: "", ..Fixture::new("commit-no-metadata") })}
            </Cell>
            <Cell full=true label="the metadata editor open">
                {page(Fixture { editing: true, ..Fixture::new("commit-expanded") })}
            </Cell>
            <Cell
                full=true
                label="the metadata editor's context menu — right-click a key: the menu opens upward \
                       over the form and nothing crops it"
            >
                {page(Fixture { editing: true, ..Fixture::new("commit-editor-menu") })}
            </Cell>
            <Cell
                full=true
                label="a failed metadata check — the editor opened itself, the caption says why, \
                       and Publish is disabled with a tooltip saying the same"
            >
                {page(Fixture {
                    metadata_error: Some(FAILED_CHECK),
                    blocked: Some(FAILED_CHECK),
                    ..Fixture::new("commit-failed-check")
                })}
            </Cell>
            <Cell full=true label="junk files — the banner, with Ignore them">
                {page(Fixture {
                    problem: Some(Problem::Junk {
                        count: 2,
                        names: vec![".DS_Store".to_string()],
                    }),
                    ..Fixture::new("commit-junk")
                })}
            </Cell>
            <Cell full=true label="no access for the role — both halves of the primary disabled: saving checks the workflow, which reads the bucket">
                {page(Fixture {
                    problem: Some(Problem::NoAccess {
                        reason: "Your role analyst cannot write to s3://quilt-example".to_string(),
                    }),
                    blocked: Some("Your role cannot write to this bucket"),
                    ..Fixture::new("commit-no-access")
                })}
            </Cell>
            <Cell full=true label="signed out — the primary disabled, Sign in in the banner">
                {page(Fixture {
                    problem: Some(Problem::SignedOut {
                        host: Some("demo.quiltdata.com".to_string()),
                    }),
                    blocked: Some("Sign in to publish"),
                    ..Fixture::new("commit-signed-out")
                })}
            </Cell>
            <Cell full=true label="a long list, 300 files — the form stays on the first screen">
                {page(Fixture {
                    primary: Primary::Files(300),
                    files: many_files(),
                    ignored: 14,
                    metadata: LONG_METADATA,
                    ..Fixture::new("commit-long")
                })}
            </Cell>
            <Cell full=true label="loading">
                {loading()}
            </Cell>
        </Scene>
    }
}
