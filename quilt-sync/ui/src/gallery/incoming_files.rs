//! The installed-package page while a newer revision exists, before *Get latest*.
//!
//! The v1 page names the files a newer revision brings and says up front
//! whether getting it would conflict. The v2 page said only *Newer revision
//! available*. This scene draws what it shows instead: the files the newer
//! revision adds, as rows in the list, and a conflict found before the click.
//!
//! Every region is the one the whole-page scene draws, over fixture props.
//! The file pane takes the UI's own `PullCheck`, so the page can hand it what
//! `package_pull_outcome` returns; the header takes the state that check
//! resolves to. Callbacks are dropped.
//!
//! # What the dry run can supply
//!
//! `PullPreview` carries a verdict and the paths the newer revision adds.
//! Nothing else: no sizes for those paths, and nothing about the files the
//! revision changes or removes. So an incoming row has no size, and no row
//! says *changed upstream*. A `Blocked` verdict names its conflicts, and those
//! rows carry resolve mode's `Differs` mark, whose tooltip already says what is
//! true of them.
//!
//! # The header
//!
//! It keeps `Newer revision available` and `Get latest`, with no count: the
//! list box's first line counts the files, beside the rows it counts. A
//! `Blocked` verdict resolves it to `PackageState::PullConflict`, which offers
//! `Publish`, not `Resolve`: the merge page cannot act until the local changes
//! are published. That is the state a failed `Get latest` leaves, so the
//! header reads the same before the click and after it.
//!
//! `Get latest` stays enabled while the check runs and after it fails. The
//! real pull classifies everything again under the lock, so the dry run
//! gates nothing; v1 disabled its button and could leave it stuck.
//!
//! # Measured at 1024x560, in Chromium
//!
//! - **Grouped at the top**, three incoming rows: the line, the heading and
//!   all three rows are on the first screen, with three installed rows under
//!   them. **Each in its folder**: none of the three is on the first screen.
//!   The owner picked the group on 2026-10-08.
//! - **300 incoming**: the heading reads 300 and six rows show; collapsing it
//!   gives the installed list its whole box back.
//! - The line is one row at 700px, except with local changes kept, where
//!   *Your changes stay.* wraps it to two and costs about 20px of list.
//! - The failed check's *Try again* sits on the line, so a failure is no
//!   taller than a success.
//! - In the conflict cell the marked rows are in `notes/`, below the first
//!   screen; the header and the line say it before the reader scrolls.
//!
//! # Left for the port
//!
//! - An incoming row's state label sits left of an empty size column.

use leptos::context::Provider;
use leptos::prelude::*;

use crate::Cell;
use crate::Scene;
use crate::commands::PullCheck;
use crate::commands::PullOutcome;
use crate::commands::PullPreview;
use crate::gallery::context_pane::ContextPaneRegion;
use crate::gallery::file_pane::FilePaneRegion;
use crate::gallery::file_pane::Placement;
use crate::gallery::installed_package::appbar_actions;
use crate::gallery::package_header::PackageHeaderRegion;
use crate::kit::DiffersId;
use crate::kit::PackageState;
use crate::kit::PageLayout;

/// The two local files a conflict names. Both are `Changed` in the fixture,
/// and both are in the first screen of the list.
const CONFLICTS: &[&str] = &["notes/intake-upload.md", "notes/kickoff-thread.md"];

/// Three files a newer revision adds: two in folders the copy has, one in a
/// folder it does not.
fn three_added() -> Vec<String> {
    vec![
        "notes/plate-07-review.md".to_string(),
        "qc/flags.json".to_string(),
        "raw/plate-37.csv".to_string(),
    ]
}

/// A revision that adds a run of 300 plates, after the 36 the copy has.
fn many_added() -> Vec<String> {
    (37..37 + 300)
        .map(|i| format!("raw/plate-{i:03}.csv"))
        .collect()
}

fn ready(outcome: PullOutcome, added: Vec<String>) -> PullCheck {
    PullCheck::Ready(PullPreview { outcome, added })
}

/// What the header resolves to for a check: the conflict state for a
/// `Blocked` verdict, `Behind` for everything else.
fn header_state(check: &PullCheck) -> PackageState {
    match check {
        PullCheck::Ready(PullPreview {
            outcome: PullOutcome::Blocked { conflicts },
            ..
        }) => PackageState::PullConflict {
            files: conflicts.clone(),
        },
        _ => PackageState::Behind,
    }
}

/// How one cell's page stands.
struct Page {
    name: &'static str,
    check: PullCheck,
    placement: Placement,
    /// `Keeping → The whole package`.
    whole: bool,
    /// No local changes, as a `CleanUpdate` has.
    clean: bool,
}

impl Page {
    fn new(name: &'static str, check: PullCheck) -> Self {
        Self {
            name,
            check,
            placement: Placement::OnTop,
            whole: false,
            clean: true,
        }
    }
}

/// The page at the 1024×560 floor.
fn page(p: Page) -> AnyView {
    let Page {
        name,
        check,
        placement,
        whole,
        clean,
    } = p;
    let publish_choice = RwSignal::new(0_usize);
    let scope = RwSignal::new(if whole { "all" } else { "pick" }.to_string());
    let state = header_state(&check);

    view! {
        <div id=name class="g-window" style="width:1024px; --q-frame-height:560px; max-width:100%">
            <PageLayout heading="QuiltSync" banner=().into_any() actions=appbar_actions()>
                <div class="g-ip-page">
                    <PackageHeaderRegion state=state publish_choice=publish_choice />
                    // The conflict cell's rows carry the `Differs` mark, whose
                    // tooltip points at an id of its own per cell.
                    <Provider value=DiffersId("incoming-differing")>
                        <div class="g-ip-shell">
                            <ContextPaneRegion
                                resolving=false
                                scope=scope
                                exit=format!("#{name}")
                            />
                            <FilePaneRegion
                                name=name
                                whole=whole
                                clean=clean
                                incoming=check
                                placement=placement
                            />
                        </div>
                    </Provider>
                </div>
            </PageLayout>
        </div>
    }
    .into_any()
}

const NOTE: &str = "The page at the 1024×560 floor while a newer revision exists, before Get \
    latest. The files it adds are rows labelled Incoming, with no box, size or menu: the dry \
    run sends paths only. The list's first line says what the check found, and the header \
    carries no count. The rows are grouped at the top, as the owner ruled on 2026-10-08; the \
    two cells marked rejected put each in its folder instead. A conflict resolves the header to the existing conflict state, \
    which offers Publish.";

#[component]
#[allow(clippy::too_many_lines, reason = "one cell per state, read as a list")]
pub fn IncomingFilesScene() -> impl IntoView {
    view! {
        <Scene title="The installed package page, a newer revision available" note=NOTE>
            <Cell full=true label="3 incoming, individual-file sync — grouped at the top">
                {page(Page::new("in-top", ready(PullOutcome::CleanUpdate, three_added())))}
            </Cell>
            <Cell full=true label="rejected: each in its folder — none of the three is on the first screen">
                {page(Page {
                    placement: Placement::InPlace,
                    ..Page::new("in-place", ready(PullOutcome::CleanUpdate, three_added()))
                })}
            </Cell>
            <Cell full=true label="3 incoming, whole-package sync — Get latest downloads them">
                {page(Page {
                    whole: true,
                    ..Page::new("in-whole", ready(PullOutcome::CleanUpdate, three_added()))
                })}
            </Cell>
            <Cell full=true label="300 incoming — grouped at the top">
                {page(Page::new("in-many-top", ready(PullOutcome::CleanUpdate, many_added())))}
            </Cell>
            <Cell full=true label="rejected: 300 incoming, each in its folder — they mix into raw/ among the plates you have">
                {page(Page {
                    placement: Placement::InPlace,
                    ..Page::new("in-many-place", ready(PullOutcome::CleanUpdate, many_added()))
                })}
            </Cell>
            <Cell full=true label="still checking — Get latest stays usable">
                {page(Page::new("in-checking", PullCheck::Loading))}
            </Cell>
            <Cell full=true label="the check failed — Try again, and Get latest stays usable">
                {page(Page::new("in-failed", PullCheck::Failed))}
            </Cell>
            <Cell full=true label="local changes, which Get latest keeps">
                {page(Page {
                    clean: false,
                    ..Page::new(
                        "in-keeps",
                        ready(
                            PullOutcome::KeepsLocalChanges {
                                added: vec!["notes/plate-07-rerun.md".to_string()],
                                modified: CONFLICTS.iter().map(ToString::to_string).collect(),
                                removed: vec!["notes/superseded-layout.md".to_string()],
                            },
                            three_added(),
                        ),
                    )
                })}
            </Cell>
            <Cell full=true label="a conflict found before the click — the header offers Publish">
                {page(Page {
                    clean: false,
                    ..Page::new(
                        "in-conflict",
                        ready(
                            PullOutcome::Blocked {
                                conflicts: CONFLICTS.iter().map(ToString::to_string).collect(),
                            },
                            three_added(),
                        ),
                    )
                })}
            </Cell>
            <Cell full=true label="a newer revision that adds no files — no rows and no line">
                {page(Page::new("in-nothing", ready(PullOutcome::CleanUpdate, Vec::new())))}
            </Cell>
        </Scene>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::*;

    /// Incoming rows add no control: the cell with three of them has as many
    /// boxes and `[⋯]` menus as the cell over the same files with none, and
    /// only it shows the group and the rows' label.
    #[wasm_bindgen_test]
    async fn incoming_rows_add_no_box_and_no_menu() {
        let doc = web_sys::window().unwrap().document().unwrap();
        let container: web_sys::HtmlElement =
            doc.create_element("div").unwrap().dyn_into().unwrap();
        doc.body().unwrap().append_child(&container).unwrap();
        let handle = leptos::mount::mount_to(container.clone(), IncomingFilesScene);
        leptos::task::tick().await;

        let count = |cell: &str, selector: &str| {
            container
                .query_selector_all(&format!("#{cell} {selector}"))
                .unwrap()
                .length()
        };
        let text = |cell: &str| {
            container
                .query_selector(&format!("#{cell}"))
                .unwrap()
                .unwrap()
                .text_content()
                .unwrap_or_default()
        };
        let boxes = "input[type=checkbox]";
        let menus = "[aria-label='More actions for this file']";
        let (top, nothing) = (text("in-top"), text("in-nothing"));
        let found = (
            count("in-top", boxes),
            count("in-nothing", boxes),
            count("in-top", menus),
            count("in-nothing", menus),
            top.matches("Incoming").count(),
            top.contains("From the newer revision"),
            nothing.contains("Incoming"),
        );
        drop(handle);
        container.remove();

        let (top_boxes, nothing_boxes, top_menus, nothing_menus, rows, heading, stray) = found;
        assert_eq!(top_boxes, nothing_boxes, "an incoming row drew a box");
        assert_eq!(top_menus, nothing_menus, "an incoming row drew a menu");
        assert!(top_menus > 0, "the cells draw the installed rows' menus");
        assert_eq!(rows, 3, "three rows labelled Incoming");
        assert!(heading, "the group's heading is drawn");
        assert!(!stray, "a revision that adds nothing draws no incoming row");
    }

    /// A conflict found before the click is the state a failed Get latest
    /// leaves, so the header cannot read differently before and after.
    #[test]
    fn a_blocked_check_resolves_to_the_conflict_state() {
        let blocked = ready(
            PullOutcome::Blocked {
                conflicts: vec!["a.csv".to_string()],
            },
            Vec::new(),
        );
        assert_eq!(
            header_state(&blocked),
            PackageState::PullConflict {
                files: vec!["a.csv".to_string()]
            }
        );
        for check in [
            PullCheck::Loading,
            PullCheck::Failed,
            ready(PullOutcome::CleanUpdate, three_added()),
        ] {
            assert_eq!(header_state(&check), PackageState::Behind);
        }
    }
}
