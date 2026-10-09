//! The installed-package page while a newer revision exists, before *Get latest*.
//!
//! The page names the files a newer revision brings and says up front
//! whether getting it would conflict: after the header's state label,
//! `· 6 file changes`, dash-underlined, opens a popover listing them. Its
//! pinned first line counts them by kind and says what Get latest does with
//! them, or, on a conflict, how many conflict and what to do; each file links
//! to its newer version in the catalog.
//!
//! The summary is the page's own `IncomingSummary`
//! (`pages/installed_package_v2/incoming.rs`, whose docs hold the design),
//! over fixture checks. The header and the panes are the whole-page scene's
//! regions, over fixture props; the header takes the state the check resolves
//! to. Callbacks are dropped.
//!
//! # What the dry run supplies
//!
//! `PullPreview` carries a verdict, the paths the newer revision adds, changes
//! and removes, and the newer revision's hash, which the catalog links name.
//! Nothing else: no sizes. A `Deleted` file has no link: the newer revision
//! does not hold it. The cells' catalog host and hash are fixtures.

use std::path::Path;

use leptos::context::Provider;
use leptos::prelude::*;

use crate::Cell;
use crate::Scene;
use crate::commands::PullCheck;
use crate::commands::PullOutcome;
use crate::commands::PullPreview;
use crate::gallery::context_pane::ContextPaneRegion;
use crate::gallery::file_pane::FilePaneRegion;
use crate::gallery::installed_package::appbar_actions;
use crate::gallery::package_header::PackageHeaderRegion;
use crate::kit::DiffersId;
use crate::kit::PackageState;
use crate::kit::PageLayout;
use crate::pages::Catalog;
use crate::pages::IncomingSummary;
use crate::pages::header_state;

/// The id of the conflict sentence the marked rows' `Differs` points at.
const DIFFERS: &str = "incoming-differing";

/// The two local files a conflict names. Both are `Changed` in the fixture.
const CONFLICTS: &[&str] = &["notes/intake-upload.md", "notes/kickoff-thread.md"];

/// The newer revision's hash.
const NEWER: &str = "9f3c1a2b7d4e";

/// Where the fixture package is read.
fn catalog() -> Option<Catalog> {
    let package = quilt_sync_ui::util::package_uri(
        "quilt-lab-plates",
        &"user/plate-07".try_into().expect("a namespace"),
        Some("quilt-lab.example"),
    );
    Catalog::new(Some(&package), Callback::new(|_url: String| ()))
}

/// Three files a newer revision adds.
fn three_added() -> Vec<String> {
    vec![
        "notes/plate-07-review.md".to_string(),
        "qc/flags.json".to_string(),
        "raw/plate-37.csv".to_string(),
    ]
}

/// A revision that adds a run of `n` plates, after the 36 the copy has.
fn plates_added(n: usize) -> Vec<String> {
    (37..37 + n)
        .map(|i| format!("raw/plate-{i:04}.csv"))
        .collect()
}

/// A check that found `added`, and no changed or removed files.
fn ready(outcome: PullOutcome, added: Vec<String>) -> PullCheck {
    with(outcome, added, &[], &[])
}

/// A check that found files of every kind, each list sorted by path, as the
/// engine sends them.
fn with(
    outcome: PullOutcome,
    mut added: Vec<String>,
    changed: &[&str],
    removed: &[&str],
) -> PullCheck {
    let owned = |paths: &[&str]| {
        let mut paths: Vec<String> = paths.iter().map(ToString::to_string).collect();
        paths.sort_by(|a, b| Path::new(a).cmp(Path::new(b)));
        paths
    };
    added.sort_by(|a, b| Path::new(a).cmp(Path::new(b)));
    PullCheck::Ready(PullPreview {
        outcome,
        added,
        changed: owned(changed),
        removed: owned(removed),
        latest_hash: Some(NEWER.to_string()),
    })
}

/// How one cell's page stands.
struct Page {
    name: &'static str,
    check: PullCheck,
    /// `Keeping → The whole package`.
    whole: bool,
    /// No local changes, as a clean update has.
    clean: bool,
    /// The popover starts open, as a reader's click leaves it.
    opened: bool,
}

impl Page {
    fn new(name: &'static str, check: PullCheck) -> Self {
        Self {
            name,
            check,
            whole: false,
            clean: true,
            opened: false,
        }
    }
}

/// The page at the 1024×560 floor.
fn page(p: Page) -> AnyView {
    let Page {
        name,
        check,
        whole,
        clean,
        opened,
    } = p;
    let publish_choice = RwSignal::new(0_usize);
    let scope = RwSignal::new(if whole { "all" } else { "pick" }.to_string());
    let state = header_state(&PackageState::Behind, Some(&check), true);
    let conflicts = if matches!(state, PackageState::PullConflict { .. }) {
        CONFLICTS
    } else {
        &[]
    };

    view! {
        <div id=name class="g-window" style="width:1024px; --q-frame-height:560px; max-width:100%">
            <PageLayout heading="QuiltSync" banner=().into_any() actions=appbar_actions()>
                // The conflict cell's rows carry the `Differs` mark, which
                // the popover's sentence describes.
                <Provider value=DiffersId(DIFFERS)>
                    <div class="g-ip-page">
                        <PackageHeaderRegion
                            state=state
                            publish_choice=publish_choice
                            summary=view! {
                                <IncomingSummary
                                    check=Signal::stored(Some(check))
                                    whole=whole
                                    catalog=catalog()
                                    on_retry=Callback::new(|()| ())
                                    opened=opened
                                />
                            }
                                .into_any()
                        />
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
                                conflicts=conflicts
                            />
                        </div>
                    </div>
                </Provider>
            </PageLayout>
        </div>
    }
    .into_any()
}

const NOTE: &str = "The page at the 1024×560 floor while a newer revision exists, before Get \
    latest. After the header's state label, · 6 file changes opens a popover that counts the \
    files by kind, says what Get latest does with them, and lists them, each linked to its \
    newer version in the catalog. The file list keeps all its rows. A conflict resolves the \
    header to the existing conflict state, which offers Publish.";

/// The conflict cell's changed files: the two it conflicts on, and one more.
const CONFLICT_CHANGED: &[&str] = &[
    "notes/intake-upload.md",
    "notes/kickoff-thread.md",
    "raw/plate-12.csv",
];

#[component]
#[allow(clippy::too_many_lines, reason = "one cell per state, read as a list")]
pub fn IncomingFilesScene() -> impl IntoView {
    view! {
        <Scene title="The installed package page, a newer revision available" note=NOTE>
            <Cell full=true label="3 new files: the summary after the state label">
                {page(Page::new("in-pick", ready(PullOutcome::CleanUpdate, three_added())))}
            </Cell>
            <Cell full=true label="the popover, opened — what Get latest does, then each file with its catalog link">
                {page(Page {
                    opened: true,
                    ..Page::new("in-open", ready(PullOutcome::CleanUpdate, three_added()))
                })}
            </Cell>
            <Cell full=true label="whole-package sync, opened — Get latest downloads them">
                {page(Page {
                    whole: true,
                    opened: true,
                    ..Page::new("in-whole", ready(PullOutcome::CleanUpdate, three_added()))
                })}
            </Cell>
            <Cell full=true label="300 new files — the popover scrolls, the page does not move">
                {page(Page::new("in-many", ready(PullOutcome::CleanUpdate, plates_added(300))))}
            </Cell>
            <Cell full=true label="over the cap, opened — 4,312 new files: the first 1,000 by path, and a line saying so">
                {page(Page {
                    opened: true,
                    ..Page::new("in-capped", ready(PullOutcome::CleanUpdate, plates_added(4_312)))
                })}
            </Cell>
            <Cell full=true label="still checking — Get latest stays usable">
                {page(Page::new("in-checking", PullCheck::Loading))}
            </Cell>
            <Cell full=true label="the check failed — Try again, and Get latest stays usable">
                {page(Page::new("in-failed", PullCheck::Failed))}
            </Cell>
            <Cell full=true label="local changes, which Get latest keeps, opened">
                {page(Page {
                    clean: false,
                    opened: true,
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
            <Cell full=true label="a conflict found before the click, opened — the header offers Publish; the conflicting files are among the changed ones, and say so">
                {page(Page {
                    clean: false,
                    opened: true,
                    ..Page::new(
                        "in-conflict",
                        with(
                            PullOutcome::Blocked {
                                conflicts: CONFLICTS.iter().map(ToString::to_string).collect(),
                            },
                            three_added(),
                            CONFLICT_CHANGED,
                            &[],
                        ),
                    )
                })}
            </Cell>
            <Cell full=true label="a newer revision that adds no files — no summary, no popover">
                {page(Page::new("in-nothing", ready(PullOutcome::CleanUpdate, Vec::new())))}
            </Cell>
            <Cell full=true label="changed and removed files counted too, opened — a deleted file has no link">
                {page(Page {
                    opened: true,
                    ..Page::new(
                        "in-mixed",
                        with(
                            PullOutcome::CleanUpdate,
                            three_added(),
                            &["README.md", "raw/plate-12.csv"],
                            &["notes/handoff-02.md"],
                        ),
                    )
                })}
            </Cell>
        </Scene>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::*;

    /// The popover, which the summary opens.
    const POPOVER: &str = "[popover][aria-label='Files coming with the newer revision']";

    /// The fixture's address for a file at the newer revision.
    fn catalog_href(path: &str) -> String {
        format!(
            "https://quilt-lab.example/b/quilt-lab-plates/packages/user/plate-07/tree/{NEWER}/{path}"
        )
    }

    /// The popover lists the files with a catalog link each and no other
    /// control; a deleted file has no link; a revision that adds nothing
    /// draws no row.
    #[wasm_bindgen_test]
    async fn the_popover_links_each_file_to_the_catalog() {
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
        let href = container
            .query_selector(&format!("#in-pick {POPOVER} ul a"))
            .unwrap()
            .and_then(|a| a.get_attribute("href"))
            .unwrap_or_default();
        let found = (
            count("in-pick", &format!("{POPOVER} ul li")),
            count(
                "in-pick",
                &format!("{POPOVER} ul a[aria-label='Open in catalog']"),
            ),
            count(
                "in-pick",
                &format!("{POPOVER} ul input, {POPOVER} ul button"),
            ),
            count("in-mixed", &format!("{POPOVER} ul li")),
            count("in-mixed", &format!("{POPOVER} ul a")),
            container
                .query_selector(&format!("#in-nothing {POPOVER}"))
                .unwrap()
                .is_some(),
            container
                .query_selector(&format!("#in-conflict {POPOVER} ul"))
                .unwrap()
                .and_then(|ul| ul.text_content())
                .unwrap_or_default()
                .matches("Conflict")
                .count(),
        );
        let capped = (
            count("in-capped", &format!("{POPOVER} ul li")) as usize,
            container
                .query_selector(&format!("#in-capped {POPOVER}"))
                .unwrap()
                .and_then(|e| e.text_content())
                .unwrap_or_default(),
        );
        drop(handle);
        container.remove();

        let (rows, links, controls, mixed_rows, mixed_links, stray, conflicts) = found;
        assert_eq!(
            capped.0,
            crate::pages::LISTED,
            "the popover lists the first 1,000"
        );
        assert!(
            capped
                .1
                .contains("This list covers the first 1,000 of 4,312 by path."),
            "and says so: {}",
            capped.1
        );
        assert_eq!(conflicts, 2, "each conflicting file says so");
        assert_eq!(rows, 3, "three files listed");
        assert_eq!(links, 3, "each one links to the catalog");
        assert_eq!(controls, 0, "no box and no button on a file");
        assert_eq!(
            (mixed_rows, mixed_links),
            (6, 5),
            "the deleted file has no link"
        );
        assert_eq!(href, catalog_href("notes/plate-07-review.md"));
        assert!(!stray, "a revision that adds nothing draws no row");
    }
}
