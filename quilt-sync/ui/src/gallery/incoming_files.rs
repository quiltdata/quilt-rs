//! The installed-package page while a newer revision exists, before *Get latest*.
//!
//! The v1 page names the files a newer revision brings and says up front
//! whether getting it would conflict. The v2 page said only *Newer revision
//! available*. This scene draws what it shows instead: a section between the
//! header and the panes, *Coming with the newer revision*, listing what the
//! revision brings the way the commit page's *What's included* lists what a
//! revision sends, and a conflict found before the click.
//!
//! The header and the panes are the whole-page scene's regions, over fixture
//! props. The section takes the UI's own `PullCheck`, so the page can hand it
//! what `package_pull_outcome` returns; the header takes the state that check
//! resolves to. Callbacks are dropped.
//!
//! # Why a section and not rows in the file list
//!
//! The file list is the installed revision: its facets count it, its folders'
//! checks describe it. Rows for files this copy does not have, mixed into it,
//! read as part of it — grouped at the top, their heading sat at the same
//! indent as the root files under it. A section of its own says whose files
//! these are, and the commit page already has the shape.
//!
//! # What the dry run can supply
//!
//! `PullPreview` carries a verdict and the paths the newer revision adds.
//! Nothing else: no sizes, and nothing about the files the revision changes or
//! removes. So today every row reads `New` and has no size. One cell draws the
//! section with changed and removed files too, labelled as needing backend
//! work: the dry run already holds both manifests, so the lists are there to
//! return. The owner chose on 2026-10-08 to return them when the page is
//! wired up, so that cell is the target.
//!
//! # The header
//!
//! It keeps `Newer revision available` and `Get latest`, with no count: the
//! section counts the files. A `Blocked` verdict resolves it to
//! `PackageState::PullConflict`, which offers `Publish`, not `Resolve`: the
//! merge page cannot act until the local changes are published. That is the
//! state a failed `Get latest` leaves, so the header reads the same before the
//! click and after it. The conflicting files keep their rows in the file list,
//! carrying resolve mode's `Differs` mark, described by the section's line.
//!
//! `Get latest` stays enabled while the check runs and after it fails. The
//! real pull classifies everything again under the lock, so the dry run
//! gates nothing; v1 disabled its button and could leave it stuck.
//!
//! # At 1024x560
//!
//! Open, the section takes about 150px and leaves the file list three rows
//! (about eight without it). So it starts collapsed to one line, about 24px,
//! and the file list keeps about seven; the owner chose that on 2026-10-08.
//! Opened, its list is capped at about three rows and scrolls, so a revision
//! bringing 300 files costs what three files do. The conflict's sentence and
//! a failed check show while collapsed; what Get latest does with the files
//! shows when open.

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
use crate::kit::Button;
use crate::kit::Card;
use crate::kit::DiffersId;
use crate::kit::EntryRow;
use crate::kit::PackageState;
use crate::kit::PageLayout;
use crate::pages::commit_v2::Change;
use quilt_sync_ui::util::thousands;

/// The two local files a conflict names. Both are `Changed` in the fixture.
const CONFLICTS: &[&str] = &["notes/intake-upload.md", "notes/kickoff-thread.md"];

/// Three files a newer revision adds.
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

/// `1 file` or `3 files`, with the app's separators.
fn files_word(n: usize) -> String {
    if n == 1 {
        String::from("1 file")
    } else {
        format!("{} files", thousands(n))
    }
}

/// What the heading says after its title: the count, or how the check
/// stands while it has none.
fn tally(check: &PullCheck, rows: usize) -> Option<String> {
    match check {
        PullCheck::Loading => Some(String::from("checking\u{2026}")),
        PullCheck::Failed => Some(String::from("couldn't check")),
        PullCheck::Ready(_) => (rows > 0).then(|| files_word(rows)),
    }
}

/// The conflict's sentence, shown whether the section is open or not: it is
/// the one thing here the reader has to act on, and it describes the rows the
/// file list marks.
fn conflict_words(check: &PullCheck) -> Option<String> {
    let PullCheck::Ready(PullPreview {
        outcome: PullOutcome::Blocked { conflicts },
        ..
    }) = check
    else {
        return None;
    };
    let them = if conflicts.len() == 1 { "it" } else { "them" };
    Some(format!(
        "{} changed here and in the newer revision. Publish your changes, then resolve {them}.",
        files_word(conflicts.len()),
    ))
}

/// What *Get latest* does with the files, shown when the section is open.
/// `whole` is the sync scope: list new files under individual-file sync,
/// download them under the whole package. `updates` is whether the list also
/// holds changed or removed files, which the dry run does not return yet:
/// then under individual-file sync only the files this copy has are updated.
/// Local changes the update keeps are named too.
fn scope_words(check: &PullCheck, whole: bool, updates: bool) -> Option<String> {
    let PullCheck::Ready(preview) = check else {
        return None;
    };
    let fate = match (preview.added.len(), whole) {
        _ if updates && whole => "Get latest downloads them.",
        _ if updates => {
            "Get latest updates the files you have and lists new ones, to download when you need them."
        }
        (0, _) => return None,
        (1, true) => "Get latest downloads it.",
        (_, true) => "Get latest downloads them.",
        (1, false) => "Get latest adds it to your files, to download when you need it.",
        (_, false) => "Get latest adds them to your files, to download when you need them.",
    };
    let kept = matches!(preview.outcome, PullOutcome::KeepsLocalChanges { .. });
    Some(if kept {
        format!("{fate} Your changes stay.")
    } else {
        fate.to_string()
    })
}

/// One file the newer revision brings, as the section lists it.
#[derive(Clone)]
struct Coming {
    path: String,
    change: Change,
}

/// The section's rows: what the dry run names, as `New`. `extra` stands for
/// the changed and removed files the dry run does not return yet.
fn coming(check: &PullCheck, extra: &[(&str, Change)]) -> Vec<Coming> {
    let PullCheck::Ready(preview) = check else {
        return Vec::new();
    };
    let mut rows: Vec<Coming> = preview
        .added
        .iter()
        .map(|path| Coming {
            path: path.clone(),
            change: Change::New,
        })
        .chain(extra.iter().map(|&(path, change)| Coming {
            path: path.to_string(),
            change,
        }))
        .collect();
    rows.sort_by(|a, b| a.path.cmp(&b.path));
    rows
}

/// The section between the header and the panes: one line, a disclosure
/// over the list, collapsed until the reader opens it, so at the window floor
/// it costs the file list a line rather than half its rows. `None` when there
/// is nothing to say: no files coming and no conflict.
fn incoming_section(
    check: &PullCheck,
    whole: bool,
    extra: &[(&str, Change)],
    opened: bool,
) -> Option<AnyView> {
    let rows = coming(check, extra);
    let conflict = conflict_words(check);
    if matches!(check, PullCheck::Ready(_)) && rows.is_empty() && conflict.is_none() {
        return None;
    }
    let tally = tally(check, rows.len()).map(|t| format!(" · {t}"));
    let scope = scope_words(check, whole, !extra.is_empty());
    let failed = check.is_failed();
    let can_open = !rows.is_empty();
    let open = RwSignal::new(opened && can_open);
    let rows = StoredValue::new(rows);
    let title = view! {
        "Coming with the newer revision"
        <span class="g-in-tally">{tally}</span>
    };
    // A conflict's sentence is what the marked rows' `Differs` describes.
    let id = conflict.is_some().then(crate::kit::differs_id);

    Some(
        view! {
            <section class="g-in-section" aria-label="Coming with the newer revision">
                <div class="g-in-line">
                    {if can_open {
                        view! {
                            <button
                                class="g-in-toggle"
                                aria-expanded=move || open.get().to_string()
                                on:click=move |_| open.update(|o| *o = !*o)
                            >
                                {move || {
                                    if open.get() {
                                        crate::kit::icons::chevron_down()
                                    } else {
                                        crate::kit::icons::chevron_right()
                                    }
                                }}
                                {title}
                            </button>
                        }
                            .into_any()
                    } else {
                        view! { <h3 class="g-in-toggle">{title}</h3> }.into_any()
                    }}
                    {failed.then(|| view! { <Button on_click=|_| ()>"Try again"</Button> })}
                </div>
                {conflict.map(|words| view! { <p class="g-in-words" id=id>{words}</p> })}
                <Show when=move || open.get()>
                    {scope.clone().map(|words| view! { <p class="g-in-words">{words}</p> })}
                    <Card flush=true>
                        <ul class="g-in-rows">
                            {rows
                                .get_value()
                                .into_iter()
                                .map(|row| {
                                    // No size: the dry run sends paths only. No box,
                                    // no click and no `[⋯]`: the file is not here.
                                    view! {
                                        <li>
                                            <EntryRow
                                                name=row.path
                                                state=row.change.words().to_string()
                                                tone=row.change.tone()
                                                size=String::new()
                                            />
                                        </li>
                                    }
                                })
                                .collect_view()}
                        </ul>
                    </Card>
                </Show>
            </section>
        }
        .into_any(),
    )
}

/// How one cell's page stands.
struct Page {
    name: &'static str,
    check: PullCheck,
    /// `Keeping → The whole package`.
    whole: bool,
    /// No local changes, as a clean update has.
    clean: bool,
    /// Changed and removed files the dry run does not return yet.
    extra: &'static [(&'static str, Change)],
    /// The section starts open, as a reader's click leaves it.
    opened: bool,
}

impl Page {
    fn new(name: &'static str, check: PullCheck) -> Self {
        Self {
            name,
            check,
            whole: false,
            clean: true,
            extra: &[],
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
        extra,
        opened,
    } = p;
    let publish_choice = RwSignal::new(0_usize);
    let scope = RwSignal::new(if whole { "all" } else { "pick" }.to_string());
    let state = header_state(&check);
    let conflicts = if matches!(state, PackageState::PullConflict { .. }) {
        CONFLICTS
    } else {
        &[]
    };

    view! {
        <div id=name class="g-window" style="width:1024px; --q-frame-height:560px; max-width:100%">
            <PageLayout heading="QuiltSync" banner=().into_any() actions=appbar_actions()>
                <div class="g-ip-page">
                    <PackageHeaderRegion state=state publish_choice=publish_choice />
                    // The conflict cell's rows carry the `Differs` mark, which
                    // the section's line describes.
                    <Provider value=DiffersId("incoming-differing")>
                        {incoming_section(&check, whole, extra, opened)}
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
                    </Provider>
                </div>
            </PageLayout>
        </div>
    }
    .into_any()
}

const NOTE: &str = "The page at the 1024×560 floor while a newer revision exists, before Get \
    latest. A section between the header and the panes lists what the revision brings, as \
    the commit page's What's included lists what a revision sends. It starts collapsed to one \
    line, so the file list keeps its rows at the floor; opened, its list holds three rows and \
    scrolls. Today the dry run returns only the files a revision adds, so every row \
    reads New; the last cell shows changed and removed files too, which needs backend work. \
    A conflict resolves the header to the existing conflict state, which offers Publish.";

/// The changed and removed files the backend cell stands for.
const EXTRA: &[(&str, Change)] = &[
    ("README.md", Change::Changed),
    ("raw/plate-12.csv", Change::Changed),
    ("notes/handoff-02.md", Change::Deleted),
];

#[component]
#[allow(clippy::too_many_lines, reason = "one cell per state, read as a list")]
pub fn IncomingFilesScene() -> impl IntoView {
    view! {
        <Scene title="The installed package page, a newer revision available" note=NOTE>
            <Cell full=true label="3 new files — the section starts collapsed, one line">
                {page(Page::new("in-pick", ready(PullOutcome::CleanUpdate, three_added())))}
            </Cell>
            <Cell full=true label="3 new files, opened, individual-file sync">
                {page(Page {
                    opened: true,
                    ..Page::new("in-open", ready(PullOutcome::CleanUpdate, three_added()))
                })}
            </Cell>
            <Cell full=true label="3 new files, opened, whole-package sync — Get latest downloads them">
                {page(Page {
                    whole: true,
                    opened: true,
                    ..Page::new("in-whole", ready(PullOutcome::CleanUpdate, three_added()))
                })}
            </Cell>
            <Cell full=true label="300 new files, opened — the section's list holds three rows and scrolls">
                {page(Page {
                    opened: true,
                    ..Page::new("in-many", ready(PullOutcome::CleanUpdate, many_added()))
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
            <Cell full=true label="a conflict found before the click — the header offers Publish, the sentence shows while collapsed">
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
            <Cell full=true label="a newer revision that adds no files — no section">
                {page(Page::new("in-nothing", ready(PullOutcome::CleanUpdate, Vec::new())))}
            </Cell>
            <Cell full=true label="needs backend work: changed and removed files listed too, opened — the target">
                {page(Page {
                    extra: EXTRA,
                    opened: true,
                    ..Page::new("in-backend", ready(PullOutcome::CleanUpdate, three_added()))
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

    /// The heading counts, or says how the check stands; the sentences say
    /// what Get latest does in the scope's own terms, and what conflicts.
    #[test]
    fn the_section_says_what_the_check_found() {
        let added = |n: usize| (0..n).map(|i| format!("f{i}")).collect::<Vec<_>>();
        let keeps = || PullOutcome::KeepsLocalChanges {
            added: Vec::new(),
            modified: vec!["a.csv".to_string()],
            removed: Vec::new(),
        };
        assert_eq!(
            tally(&PullCheck::Loading, 0).as_deref(),
            Some("checking\u{2026}")
        );
        assert_eq!(
            tally(&PullCheck::Failed, 0).as_deref(),
            Some("couldn't check")
        );
        let three = ready(PullOutcome::CleanUpdate, added(3));
        assert_eq!(tally(&three, 3).as_deref(), Some("3 files"));
        assert_eq!(
            scope_words(&three, false, false).as_deref(),
            Some("Get latest adds them to your files, to download when you need them.")
        );
        assert_eq!(
            scope_words(&ready(PullOutcome::CleanUpdate, added(1)), true, false).as_deref(),
            Some("Get latest downloads it.")
        );
        assert_eq!(
            scope_words(&ready(keeps(), added(2)), true, false).as_deref(),
            Some("Get latest downloads them. Your changes stay.")
        );
        assert_eq!(
            scope_words(&ready(PullOutcome::CleanUpdate, Vec::new()), true, false),
            None
        );
        assert_eq!(
            scope_words(&three, false, true).as_deref(),
            Some(
                "Get latest updates the files you have and lists new ones, to download when \
                 you need them."
            )
        );
        assert_eq!(conflict_words(&three), None);
        assert_eq!(
            conflict_words(&ready(
                PullOutcome::Blocked {
                    conflicts: added(2)
                },
                added(1)
            ))
            .as_deref(),
            Some(
                "2 files changed here and in the newer revision. Publish your changes, then \
                 resolve them."
            )
        );
    }

    /// The v2 vocabulary holds in the section: none of the words the package
    /// states keep out.
    #[test]
    fn the_section_uses_no_banned_word() {
        const BANNED: &[&str] = &[
            "commit", "push", "pull", "remote", "behind", "ahead", "diverged", "dirty", "hash",
        ];
        let added = vec!["a".to_string(), "b".to_string()];
        let checks = [
            PullCheck::Loading,
            PullCheck::Failed,
            ready(PullOutcome::CleanUpdate, added.clone()),
            ready(
                PullOutcome::KeepsLocalChanges {
                    added: Vec::new(),
                    modified: Vec::new(),
                    removed: Vec::new(),
                },
                added.clone(),
            ),
            ready(
                PullOutcome::Blocked {
                    conflicts: added.clone(),
                },
                added,
            ),
        ];
        let mut all = vec![String::from("Coming with the newer revision")];
        for check in &checks {
            all.extend(tally(check, 2));
            all.extend(conflict_words(check));
            for whole in [false, true] {
                for updates in [false, true] {
                    all.extend(scope_words(check, whole, updates));
                }
            }
        }
        for words in all {
            let words = words.to_lowercase();
            for bad in BANNED {
                assert!(
                    !words
                        .split_whitespace()
                        .any(|w| w.trim_matches(|c: char| !c.is_alphanumeric()) == *bad),
                    "{words:?} contains the banned word {bad:?}"
                );
            }
        }
    }

    /// The section starts collapsed, its rows carry no control, and a revision that adds nothing
    /// draws no section at all.
    #[wasm_bindgen_test]
    async fn the_section_lists_without_controls() {
        let doc = web_sys::window().unwrap().document().unwrap();
        let container: web_sys::HtmlElement =
            doc.create_element("div").unwrap().dyn_into().unwrap();
        doc.body().unwrap().append_child(&container).unwrap();
        let handle = leptos::mount::mount_to(container.clone(), IncomingFilesScene);
        leptos::task::tick().await;

        let count = |cell: &str, selector: &str| {
            container
                .query_selector_all(&format!("#{cell} .g-in-section {selector}"))
                .unwrap()
                .length()
        };
        let found = (
            count("in-pick", "li"),
            count("in-open", "li"),
            count("in-open", "input"),
            count("in-open", "li button"),
            container
                .query_selector("#in-nothing .g-in-section")
                .unwrap()
                .is_some(),
        );
        drop(handle);
        container.remove();

        let (collapsed, rows, inputs, buttons, stray) = found;
        assert_eq!(collapsed, 0, "the section starts collapsed");
        assert_eq!(rows, 3, "three rows once opened");
        assert_eq!((inputs, buttons), (0, 0), "no box and no menu on a row");
        assert!(!stray, "a revision that adds nothing draws no section");
    }
}
