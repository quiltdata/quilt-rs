//! The commit page v2, drawn before it is routed.
//!
//! Every region is `pages::commit_v2`'s own, over fixture props, so the port
//! that routes the page fills these same views from `get_commit_data` and the
//! page cannot drift from this scene. The callbacks are dropped: there is no
//! Tauri runtime here, and the arrangement is what is under review.
//!
//! # What it measured, at 1024x560
//!
//! **The form stays on the first screen in every cell.** Measured from the
//! window's top edge, in Chrome:
//!
//! | | header ends | message ends | workflow section ends | files start | rows on screen |
//! |---|---:|---:|---:|---:|---:|
//! | at rest, 4 files | 144 | 217 | 321 | 337 | 4 of 4 |
//! | 300 files | 144 | 217 | 321 | 337 | **5** of 300 |
//! | junk banner, signed out | 144 | 283 | 387 | 403 | 3 of 4 |
//! | no access | 144 | 271 | 375 | 391 | 4 of 4 |
//! | workflow & metadata open | 144 | 217 | 519 | 535 | 0 |
//! | a failed check | 144 | 217 | 543 | 559 | 0 |
//!
//! - **The long list costs the form nothing.** The 300-file cell lays out the
//!   form exactly where the four-file cell does, and the page scrolls under it.
//!   This is the bet the layout makes by putting the list last, and it is won.
//! - **The folded section is three lines, 104px**: its heading with `Edit`,
//!   the exact workflow the revision will carry — the select's own option, so
//!   it follows a change — and the metadata as the catalog draws it: a
//!   `kit::JsonDisplay` folded to one line that fits the room it has, and
//!   opens in place to read the whole document without the editor. Two
//!   lines more than a bare summary, and worth them: the reader sees what will
//!   be published without opening anything. It costs the 300-file cell two
//!   rows, seven down to five. Opened, the metadata pushes the files down, as
//!   the reader asked it to.
//! - **The header is 60px**, the same as the installed-package page's: it is
//!   the same `kit::PageHeader`, with a `Trail` where that page has a
//!   `BackLink`.
//! - **A banner costs 54px on one line**, 66 with an action — `Ignore them`,
//!   `Sign in` — which takes a control's height. The glyph and the sentence
//!   are centred on that height, through `Banner`'s `action` slot; drawn
//!   inside the sentence, the button made the line taller and left the glyph
//!   at its top.
//! - **Opened, the section takes the rest of the window.** The workflow
//!   `Select` and the metadata editor end at 519, and the list starts below
//!   the fold. That is the right trade while editing: the reader asked for the
//!   editor. With a failed check the caption adds 24px and ends at 543, inside
//!   the window — which is what sets the editor's height. At 220px the caption
//!   landed at 613, below the fold; 150 is the most the floor allows.
//! - **The editor's context menu is not cropped.** It is v1's
//!   `vanilla-jsoneditor`, and it draws its menu inside its own box, so any
//!   clipping ancestor would crop it — v1's scrolling column was one. Right-
//!   clicking a key in the `editor's context menu` cell opens a 318px menu
//!   upward over the form, 257px above the editor's top; the one ancestor
//!   that clips is the page's own scroller, and the menu fits inside it.
//! - **The menu's labels are cut short** — `Edit valu`, `Cu`, `Cop` — inside
//!   the menu itself, at the editor's own 16px. That is the editor's sizing,
//!   not this page's: its button rules out-rank the normalize sheet. Not
//!   checked against v1's page.
//! - **The `Metadata` label names the fallback textarea**, which the editor
//!   hides once it mounts, so the editor itself has no accessible name from
//!   it. As on v1; the port can point the label at the editor.
//! - **The rows keep `EntryRow`'s empty leading column**, the disclosure
//!   gutter and the box's hole, about 60px. On this page no row has a box or
//!   a group, so it is width spent on nothing; the port can decide whether
//!   `EntryRow` grows a way to drop it.
//! - **The loading skeleton's header is 4px short** of the real one (140
//!   against 144): its first bar is 16px and the `Trail`'s line is 20. The
//!   installed-package page's skeleton has the same bar under a `BackLink`.
//!
//! # A blocked primary disables both halves
//!
//! No access and signed out block saving as well as publishing, because a
//! save checks the workflow and that reads the bucket's config first. So the
//! split button is disabled whole, which is what `SplitButton` does anyway,
//! and the banner does not offer saving as a way round.
//!
//! # Where the problems go
//!
//! In the column, under the header, and not in `PageLayout`'s banner slot.
//! That slot reports what a command just did and is dismissed; a problem here
//! stands while its cause does and is about the revision being written, so it
//! sits with the form it qualifies, above the field the reader starts on.
//!
//! # A blocked primary
//!
//! Disabled, with a `Tooltip` on the pointer's way to it — checked: hovering
//! the disabled split button opens it in Chrome — and the same reason
//! on the page itself — under the field that failed, or in the banner —
//! because a disabled button takes no keyboard focus and a touch screen shows
//! no tooltip. The tooltip is for the reader who reaches for the button and
//! wonders why it will not go.

use leptos::prelude::*;

use crate::Cell;
use crate::Scene;
use crate::kit::Button;
use crate::kit::PageLayout;
use crate::kit::icons;
use crate::pages::commit_v2::{
    Change, CommitColumn, CommitHeader, CommitPageSkeleton, IncludedFile, IncludedList,
    MessageField, Primary, PrimaryWiring, Problem, ProblemBanner, WorkflowChoice, WorkflowSection,
};

const NAMESPACE: &str = "user/plate-07";
const GENERATED: &str = "Updated 3 files: plate-03.csv, plate-04.csv, wells.csv";
/// Long enough to be cut: more fields than the line holds, and a nested value
/// the preview counts rather than spells out.
const LONG_METADATA: &str = "{\"assay\": \"ELISA\", \"plate\": 7, \"instrument\": \"SpectraMax iD5\", \
    \"wavelength_nm\": 450, \"wells\": [\"A1\", \"A2\", \"A3\"], \"protocol\": {\"version\": 3, \
    \"incubation_min\": 60}, \"notes\": \"Second read after recalibration\"}";
const METADATA: &str = "{\n  \"assay\": \"ELISA\",\n  \"plate\": 7\n}";

/// The appbar is the app's own, untouched.
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

/// A few files of each kind, which is what most revisions look like.
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

/// About three hundred, for the long-list cell: a plate run's worth of reads.
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

/// One cell's fixture: what differs between the states.
struct Fixture {
    id: &'static str,
    primary: Primary,
    files: Vec<IncludedFile>,
    ignored: usize,
    problem: Option<Problem>,
    /// Why the primary cannot run.
    blocked: Option<&'static str>,
    expanded: bool,
    /// The bucket offers workflows to choose between.
    workflows: bool,
    metadata: &'static str,
    metadata_error: Option<&'static str>,
    metadata_source: &'static str,
    /// What the workflow line says when the bucket offers no choice.
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
            expanded: false,
            workflows: true,
            metadata: METADATA,
            metadata_error: None,
            metadata_source: "from the current revision",
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
        expanded,
        workflows,
        metadata,
        metadata_error,
        metadata_source,
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
    let workflow_view = match workflow {
        Some(workflow) => view! {
            <WorkflowSection
                metadata_source=metadata_source
                expanded=RwSignal::new(expanded)
                workflow=workflow
                metadata=RwSignal::new(metadata.to_string())
                metadata_error=metadata_error.map(str::to_string)
            />
        }
        .into_any(),
        None => view! {
            <WorkflowSection
                no_workflow=no_workflow
                metadata_source=metadata_source
                expanded=RwSignal::new(expanded)
                metadata=RwSignal::new(metadata.to_string())
                metadata_error=metadata_error.map(str::to_string)
            />
        }
        .into_any(),
    };

    view! {
        <div id=id class="g-window" style="width:1024px; --q-frame-height:560px; max-width:100%">
            <PageLayout heading="QuiltSync" banner=().into_any() actions=appbar_actions()>
                <CommitColumn
                    header=view! {
                        <CommitHeader
                            namespace=NAMESPACE
                            // The cell's own window, as the other page scenes do: a link
                            // that goes nowhere must not move the page.
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

const NOTE: &str = "The commit page v2 at the app's 1024×560 floor, one cell per state. One \
    column: the header with the one primary, a problem when there is one, the message whose \
    placeholder is the generated one, workflow and metadata folded to a line, and the changed \
    files last — so the long-list cell scrolls the page under the form rather than pushing the \
    form off the first screen. Not routed yet: the regions are the page's own views, and the \
    port fills them from get_commit_data.";

const FAILED_CHECK: &str = "The workflow requires an \"operator\" field in the metadata";

#[component]
pub fn CommitPageScene() -> impl IntoView {
    view! {
        <Scene title="The commit page v2" note=NOTE>
            <Cell full=true label="files changed, at rest">
                {page(Fixture::new("commit-rest"))}
            </Cell>
            <Cell full=true label="metadata only — no file changes, so the primary publishes the revision">
                {page(Fixture {
                    primary: Primary::MetadataOnly,
                    files: Vec::new(),
                    ignored: 0,
                    metadata: "{\"assay\": \"ELISA\", \"plate\": 7, \"reviewed\": true}",
                    metadata_source: "edited",
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
            <Cell full=true label="workflow & metadata expanded">
                {page(Fixture { expanded: true, ..Fixture::new("commit-expanded") })}
            </Cell>
            <Cell
                full=true
                label="the metadata editor's context menu — right-click a key: the menu opens upward \
                       over the form and nothing crops it"
            >
                {page(Fixture { expanded: true, ..Fixture::new("commit-editor-menu") })}
            </Cell>
            <Cell
                full=true
                label="a failed workflow check — the section opened itself, the caption says why, \
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
