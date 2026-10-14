//! The commit page v2, one cell per state.
//!
//! Every region is `pages::commit_v2`'s own, over fixture props, so the routed
//! page (`pages::CommitV2`) and this scene cannot drift. Callbacks are dropped.
//!
//! # Measured in Chrome
//!
//! "Rows" counts those wholly in sight, in the list's box and the window.
//!
//! | | layout | box | rows |
//! |---|---|---:|---:|
//! | 1280×900, 300 files | two columns | 191–867 | 20 of 300 |
//! | 1024×560, 300 files | two columns | 191–527 | 10 of 300 |
//! | 1024×560, a banner | two columns | 257–388 | 4 of 4 |
//! | 1000×560, at rest | one column | 458–541 | 2 of 4 |
//! | 1000×560, 300 files, seven metadata fields | one column | 559–643 | 0; the page scrolls 84px |
//! | 640×560, a failed check | one column | 563–646 | 0; the page scrolls 87px |
//!
//! - Wide, the header and any banner span the page, and both columns take
//!   the height under them. The open editor with a failed check fits in the
//!   left column at 1024×560, so the page does not scroll.
//! - Narrow, the box takes what the form leaves, down to its floor; with the
//!   editor open the page scrolls instead.
//! - Paths start 13px right of the heading: the rows have no leading column.
//! - The message field's border and the list's border start on one line.
//! - The metadata preview opens its top level, a line per field, so long
//!   metadata pushes a narrow page's list down; the page then scrolls.
//! - The editor's context menu is not cropped by the column: its one
//!   clipping ancestor is the page's scroller.
//! - Nothing scrolls sideways at 640.
//!
//! # Left for later
//!
//! - The editor's context menu cuts some of its own labels short (`Edit valu`),
//!   at its own 16px.
//! - The `Metadata` label names the editor's hidden fallback textarea, as on v1.
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
    /// The window's width; the app's narrowest is 640.
    width: u32,
    height: u32,
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
            width: 1024,
            height: 560,
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
        width,
        height,
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

    let frame = format!("width:{width}px; --q-frame-height:{height}px; max-width:100%");
    view! {
        <div id=id class="g-window" style=frame>
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
                            on_command=Callback::new(|_| ())
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

const NOTE: &str = "The commit page v2. The header and any banner span the page; under them \
    the form is on the left and the changed files on the right, scrolling in their own box. \
    Narrower than about a 1024px window it is one column, the files last. The state cells are \
    at 1024×560; then the default 1280×900, one column at 1000, and the window's narrowest, 640.";

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
            <Cell full=true label="no metadata — the absence muted, and the toggle reads Add">
                {page(Fixture { metadata: "", ..Fixture::new("commit-no-metadata") })}
            </Cell>
            <Cell full=true label="the metadata editor open — drag the bar under it, or focus it and use the arrow keys, to make it taller">
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
            <Cell full=true label="a long list, 300 files — the rows scroll in their own box, and the header and form stay">
                {page(Fixture {
                    primary: Primary::Files(300),
                    files: many_files(),
                    ignored: 14,
                    metadata: LONG_METADATA,
                    ..Fixture::new("commit-long")
                })}
            </Cell>
            {size_cells()}
            {narrow_cells()}
            <Cell full=true label="loading">
                {loading()}
            </Cell>
        </Scene>
    }
}

/// The default window, and one just under the switch.
fn size_cells() -> AnyView {
    view! {
        <Cell full=true label="1280×900, the default window — at rest">
            {page(Fixture { width: 1280, height: 900, ..Fixture::new("commit-1280-rest") })}
        </Cell>
        <Cell full=true label="1280×900 — 300 files: the list scrolls in its own box beside the form">
            {page(Fixture {
                primary: Primary::Files(300),
                files: many_files(),
                ignored: 14,
                metadata: LONG_METADATA,
                width: 1280,
                height: 900,
                ..Fixture::new("commit-1280-long")
            })}
        </Cell>
        <Cell full=true label="1000 wide — one column, the files last">
            {page(Fixture { width: 1000, ..Fixture::new("commit-1000-rest") })}
        </Cell>
        <Cell full=true label="1000 wide — 300 files">
            {page(Fixture {
                primary: Primary::Files(300),
                files: many_files(),
                ignored: 14,
                metadata: LONG_METADATA,
                width: 1000,
                ..Fixture::new("commit-1000-long")
            })}
        </Cell>
    }
    .into_any()
}

/// The same page at 640, the window's narrowest: the states that add a line
/// or a long one.
fn narrow_cells() -> AnyView {
    view! {
        <Cell full=true label="640 wide, the window's narrowest — at rest">
            {page(Fixture { width: 640, ..Fixture::new("commit-640-rest") })}
        </Cell>
        <Cell full=true label="640 wide — signed out, the banner's button in reach">
            {page(Fixture {
                problem: Some(Problem::SignedOut {
                    host: Some("demo.quiltdata.com".to_string()),
                }),
                blocked: Some("Sign in to publish"),
                width: 640,
                ..Fixture::new("commit-640-signed-out")
            })}
        </Cell>
        <Cell full=true label="640 wide — the editor open, with a failed check">
            {page(Fixture {
                metadata_error: Some(FAILED_CHECK),
                blocked: Some(FAILED_CHECK),
                width: 640,
                ..Fixture::new("commit-640-failed-check")
            })}
        </Cell>
        <Cell full=true label="640 wide — 300 files, long names">
            {page(Fixture {
                primary: Primary::Files(300),
                files: many_files(),
                ignored: 14,
                metadata: LONG_METADATA,
                width: 640,
                ..Fixture::new("commit-640-long")
            })}
        </Cell>
    }
    .into_any()
}
