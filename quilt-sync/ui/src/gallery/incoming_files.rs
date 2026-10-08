//! The installed-package page while a newer revision exists, before *Get latest*.
//!
//! The v1 page names the files a newer revision brings and says up front
//! whether getting it would conflict. The v2 page said only *Newer revision
//! available*. This scene draws what it shows instead, in the header, where
//! the reader already looks: after the state label, `· 6 file changes`,
//! dash-underlined, opens a popover listing them. Its pinned first line counts
//! them by kind and says what Get latest does with them, or, on a conflict,
//! how many conflict and what to do; each file links to its newer version in
//! the catalog.
//!
//! The header and the panes are the whole-page scene's regions, over fixture
//! props. The summary takes the UI's own `PullCheck`, so the page can hand it
//! what `package_pull_outcome` returns; the header takes the state that check
//! resolves to. Callbacks are dropped.
//!
//! # Why the header
//!
//! *Newer revision available* and *conflicts in 2 files* are where the eye
//! lands, and the files are what they are about, so the summary sits beside
//! them and the popover costs the page nothing. Three other placements were
//! drawn and dropped: rows in the file list, whose heading sat at the root
//! files' indent and so seemed to own them; a section between the header and
//! the panes, which cost the file list a line collapsed and five rows open;
//! and a row in the context pane, which a reader would not think to look at
//! from the header's words, and which said the conflict a second time.
//!
//! # What the dry run can supply
//!
//! `PullPreview` carries a verdict and the paths the newer revision adds.
//! Nothing else: no sizes, no hash for the newer revision, and nothing about
//! the files it changes or removes. The owner chose on 2026-10-08 to have the
//! dry run return the changed and removed paths and the newer revision's hash
//! when the page is wired up, so the catalog links here use a fixture hash and
//! one cell counts changed and removed files, labelled as needing backend
//! work. A `Deleted` file has no link: the newer revision does not hold it.
//!
//! # The header
//!
//! It keeps `Newer revision available` and `Get latest`; the summary after
//! it is the only count. A revision that brings no files has no summary and
//! no popover. A `Blocked` verdict resolves the header to
//! `PackageState::PullConflict`, which offers `Publish`, not `Resolve`: the
//! merge page cannot act until the local changes are published. That is the
//! state a failed `Get latest` leaves, so the header reads the same before the
//! click and after it. In the popover the conflicting files carry a
//! `Conflict` label beside their own; in the file list their rows carry
//! resolve mode's `Differs` mark, which the popover's sentence describes.
//!
//! `Get latest` stays enabled while the check runs and after it fails. The
//! real pull classifies everything again under the lock, so the dry run
//! gates nothing; v1 disabled its button and could leave it stuck.
//!
//! # Measured at 1024x560, in Chromium
//!
//! - The page does not move: the summary shares the header's one row, and
//!   the popover takes no room.
//! - The popover reaches down to the window's edge, `100vh - 160px`: 400px
//!   at the floor against the 336 other overlays' 60vh allows, starting 4px
//!   under the summary. Not 90vh: the placement slides a surface up to fit,
//!   and 90vh would cover the summary it hangs from. 360px wide; its first
//!   line is pinned while the list scrolls under it.
//! - It lists the first 1,000 files by path and says so under the list when
//!   there are more, as the file list loads its first 1,000: a list drawn
//!   whole costs in proportion to its rows. The summary still counts them all.
//! - It opens on hover and closes 200ms after the pointer leaves the summary
//!   and the popover both, which is time to cross the gap between them; a
//!   click pins it until a click outside, Escape or a second click.
//!   `EntryRow`'s box, size and menu columns left a path about 50px there, so
//!   it lists path, labels and link only.

use std::time::Duration;

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
use crate::kit::Align;
use crate::kit::AnchoredOverlay;
use crate::kit::Button;
use crate::kit::DiffersId;
use crate::kit::PackageState;
use crate::kit::PageLayout;
use crate::kit::StateLabel;
use crate::kit::icons;
use crate::kit::state_label::StateTone;
use crate::pages::commit_v2::Change;
use quilt_sync_ui::util::thousands;

/// The id of the conflict sentence the marked rows' `Differs` points at.
const DIFFERS: &str = "incoming-differing";

/// The two local files a conflict names. Both are `Changed` in the fixture.
const CONFLICTS: &[&str] = &["notes/intake-upload.md", "notes/kickoff-thread.md"];

/// The newer revision's hash, which the dry run does not return yet.
const NEWER: &str = "9f3c1a2b7d4e";

/// The catalog link's name, as revision rows word theirs.
const OPEN_LABEL: &str = "Open in catalog";

/// `MouseEvent.button` for the middle button, as revision rows read it.
const MIDDLE_BUTTON: i16 = 1;

/// A file in the newer revision, in the catalog.
fn catalog_href(path: &str) -> String {
    format!(
        "https://quilt-lab.example/b/quilt-lab-plates/packages/user/plate-07/tree/{NEWER}/{path}"
    )
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

/// How many files the popover lists: the first by path, as the file list
/// loads the first 1,000. A list drawn whole costs in proportion to its rows,
/// and a revision can touch far more files than anyone reads in a popover.
const LISTED: usize = 1_000;

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

/// What *Get latest* does with the files, heading the popover. `whole` is the
/// sync scope: list new files under individual-file sync, download them under
/// the whole package. `updates` is whether the list also holds changed or
/// removed files, which the dry run does not return yet: then under
/// individual-file sync only the files this copy has are updated. Local
/// changes the update keeps are named too.
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

/// One file the newer revision brings, as the popover lists it.
#[derive(Clone)]
struct Coming {
    path: String,
    change: Change,
    /// Changed here too, differently: a `Blocked` verdict names it.
    conflict: bool,
}

/// The popover's rows: what the dry run names, as `New`. `extra` stands for
/// the changed and removed files the dry run does not return yet.
fn coming(check: &PullCheck, extra: &[(&str, Change)]) -> Vec<Coming> {
    let PullCheck::Ready(preview) = check else {
        return Vec::new();
    };
    let conflicts: &[String] = match &preview.outcome {
        PullOutcome::Blocked { conflicts } => conflicts,
        _ => &[],
    };
    let mut rows: Vec<Coming> = preview
        .added
        .iter()
        .map(|path| (path.as_str(), Change::New))
        .chain(extra.iter().copied())
        .map(|(path, change)| Coming {
            path: path.to_string(),
            change,
            conflict: conflicts.iter().any(|c| c == path),
        })
        .collect();
    rows.sort_by(|a, b| a.path.cmp(&b.path));
    rows
}

/// `3 new files`, or `3 new, 2 changed, 1 deleted` once the dry run returns
/// changed and removed files too.
fn counts_words(rows: &[Coming]) -> String {
    let n = |c: Change| rows.iter().filter(|r| r.change == c).count();
    let (new, changed, deleted) = (n(Change::New), n(Change::Changed), n(Change::Deleted));
    if changed == 0 && deleted == 0 {
        return if new == 1 {
            String::from("1 new file")
        } else {
            format!("{} new files", thousands(new))
        };
    }
    [(new, "new"), (changed, "changed"), (deleted, "deleted")]
        .into_iter()
        .filter(|(k, _)| *k > 0)
        .map(|(k, word)| format!("{} {word}", thousands(k)))
        .collect::<Vec<_>>()
        .join(", ")
}

/// `3 new files` as the start of a sentence.
fn capitalised(words: &str) -> String {
    let mut chars = words.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

/// What the header says after the state label: how the check stands, or
/// the files coming as a trigger's words. `None` when there is nothing to
/// say: a revision that brings no files, conflict or not, has no popover, and
/// the state label says the rest.
fn summary_words(check: &PullCheck, rows: &[Coming]) -> Option<String> {
    match check {
        PullCheck::Loading => Some(String::from("checking\u{2026}")),
        PullCheck::Failed => Some(String::from("couldn't check")),
        PullCheck::Ready(_) if rows.is_empty() => None,
        PullCheck::Ready(_) => Some(changes_word(rows.len())),
    }
}

/// How long the pointer may be away from the summary and its popover before
/// a hover-opened popover closes: enough to cross the 4px gap between them.
const HOVER_GRACE: Duration = Duration::from_millis(200);

/// The summary's popover opens on hover and closes when the pointer leaves
/// the summary and the popover both; a click pins it open until a click
/// outside, Escape, or a second click on the summary. The overlay's own
/// light dismiss does the closing, so a pinned popover unpins when it closes.
#[derive(Clone, Copy)]
struct HoverCard {
    open: RwSignal<bool>,
    pinned: RwSignal<bool>,
    timer: StoredValue<Option<TimeoutHandle>>,
}

impl HoverCard {
    fn new(open: RwSignal<bool>, pinned: bool) -> Self {
        let pinned = RwSignal::new(pinned);
        Effect::new(move |_| {
            if !open.get() {
                pinned.set(false);
            }
        });
        Self {
            open,
            pinned,
            timer: StoredValue::new(None),
        }
    }

    fn cancel(self) {
        if let Some(Some(pending)) = self.timer.try_update_value(Option::take) {
            pending.clear();
        }
    }

    fn enter(self) {
        self.cancel();
        if !self.open.get_untracked() {
            self.open.set(true);
        }
    }

    fn leave(self) {
        if self.pinned.get_untracked() {
            return;
        }
        self.cancel();
        let open = self.open;
        let pending =
            set_timeout_with_handle(move || open.try_set(false).map_or((), drop), HOVER_GRACE).ok();
        self.timer.set_value(pending);
    }

    fn click(self) {
        self.cancel();
        if self.pinned.get_untracked() {
            self.pinned.set(false);
            self.open.set(false);
        } else {
            self.pinned.set(true);
            self.open.set(true);
        }
    }
}

/// What a `Blocked` verdict's popover says after the counts.
fn conflict_words(check: &PullCheck) -> Option<String> {
    let PullCheck::Ready(PullPreview {
        outcome: PullOutcome::Blocked { conflicts },
        ..
    }) = check
    else {
        return None;
    };
    let n = conflicts.len();
    let them = if n == 1 { "it" } else { "them" };
    let verb = if n == 1 { "conflicts" } else { "conflict" };
    Some(format!(
        "{} of them {verb} with yours. Publish your changes, then resolve {them}.",
        thousands(n)
    ))
}

/// `1 file change`, `6 file changes`: new, changed and removed files alike.
fn changes_word(n: usize) -> String {
    if n == 1 {
        String::from("1 file change")
    } else {
        format!("{} file changes", thousands(n))
    }
}

/// The header's summary after the state label: `· checking…`, `· couldn't
/// check` with *Try again*, or `· 6 file changes`, dash-underlined, which
/// opens the popover. The popover's pinned first line counts the files by
/// kind and says what Get latest does with them; then each file, its label,
/// a *Conflict* label when the verdict names it, and its catalog link.
fn header_summary(
    check: &PullCheck,
    whole: bool,
    extra: &[(&str, Change)],
    opened: bool,
) -> Option<AnyView> {
    let rows = coming(check, extra);
    let words = summary_words(check, &rows)?;
    let failed = check.is_failed();
    if !matches!(check, PullCheck::Ready(_)) {
        return Some(
            view! {
                <span class="g-hs">
                    <span class="g-hs-dot">"\u{b7}"</span>
                    <span class="g-hs-muted">{words}</span>
                    {failed.then(|| view! { <Button on_click=|_| ()>"Try again"</Button> })}
                </span>
            }
            .into_any(),
        );
    }
    let counts = counts_words(&rows);
    let total = rows.len();
    let mut rows = rows;
    rows.truncate(LISTED);
    let cut = (total > rows.len()).then(|| {
        format!(
            "This list covers the first {} of {} by path.",
            thousands(LISTED),
            thousands(total)
        )
    });
    let open = RwSignal::new(opened);
    let card = HoverCard::new(open, opened);
    // On a conflict the line says what to do instead of what Get latest
    // would, and it is the sentence the file list's `Differs` marks describe.
    let conflict = conflict_words(check);
    let id = conflict.is_some().then_some(DIFFERS);
    let scope = conflict.or_else(|| scope_words(check, whole, !extra.is_empty()));
    Some(
        view! {
            <span class="g-hs">
                <span class="g-hs-dot">"\u{b7}"</span>
                <AnchoredOverlay
                    trigger=move |surface_id: String| {
                        view! {
                            <button
                                class="g-hs-trigger"
                                aria-expanded=move || open.get().to_string()
                                aria-controls=surface_id
                                on:click=move |_| card.click()
                                on:mouseenter=move |_| card.enter()
                                on:mouseleave=move |_| card.leave()
                            >
                                {words.clone()}
                            </button>
                        }
                            .into_any()
                    }
                    open=open
                    aria_label="Files coming with the newer revision"
                    align=Align::Start
                >
                    <div
                        class="g-nr-surface"
                        on:mouseenter=move |_| card.enter()
                        on:mouseleave=move |_| card.leave()
                    >
                        <p class="g-nr-scope" id=id>
                            <strong>{format!("{}.", capitalised(&counts))}</strong>
                            {scope.map(|w| format!(" {w}"))}
                        </p>
                        {file_list(rows, Callback::new(|_url: String| ()))}
                        {cut.map(|words| view! { <p class="g-nr-cut">{words}</p> })}
                    </div>
                </AnchoredOverlay>
            </span>
        }
        .into_any(),
    )
}

/// The link to a file's newer version in the catalog, drawn as revision rows
/// draw theirs: a real anchor, so the address can be copied, whose navigation
/// is cancelled and handed to `open`, which on the page is the browser.
fn catalog_link(path: &str, open: Callback<String>) -> AnyView {
    let href = catalog_href(path);
    let followed = href.clone();
    let middled = href.clone();
    view! {
        <a
            class="g-nr-open"
            href=href
            aria-label=OPEN_LABEL
            title=OPEN_LABEL
            on:click=move |ev| {
                ev.prevent_default();
                open.run(followed.clone());
            }
            on:auxclick=move |ev| {
                if ev.button() == MIDDLE_BUTTON {
                    ev.prevent_default();
                    open.run(middled.clone());
                }
            }
        >
            {icons::link_external()}
        </a>
    }
    .into_any()
}

/// The popover's list: the path, its label and its catalog link. `EntryRow`
/// keeps columns for a box, a size and a menu, which leave a path no room in
/// the overlay's 360px. No size, because the dry run sends paths only. A
/// deleted file keeps the link's room, so the labels stay in a column.
fn file_list(rows: Vec<Coming>, open: Callback<String>) -> AnyView {
    view! {
        <ul class="g-nr-rows">
            {rows
                .into_iter()
                .map(|row| {
                    let link = if row.change == Change::Deleted {
                        view! { <span class="g-nr-open" /> }.into_any()
                    } else {
                        catalog_link(&row.path, open)
                    };
                    view! {
                        <li>
                            <span title=row.path.clone()>{row.path.clone()}</span>
                            // A second label beside the first, as `Differs`
                            // sits beside a file row's own.
                            {row
                                .conflict
                                .then(|| {
                                    view! { <StateLabel tone=StateTone::Danger>"Conflict"</StateLabel> }
                                })}
                            <StateLabel tone=row.change.tone()>{row.change.words()}</StateLabel>
                            {link}
                        </li>
                    }
                })
                .collect_view()}
        </ul>
    }
    .into_any()
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
    let summary = header_summary(&check, whole, extra, opened);

    view! {
        <div id=name class="g-window" style="width:1024px; --q-frame-height:560px; max-width:100%">
            <PageLayout heading="QuiltSync" banner=().into_any() actions=appbar_actions()>
                <div class="g-ip-page">
                    <PackageHeaderRegion
                        state=state
                        publish_choice=publish_choice
                        summary=summary.unwrap_or_else(|| ().into_any())
                    />
                    // The conflict cell's rows carry the `Differs` mark, which
                    // the pane's row describes.
                    <Provider value=DiffersId(DIFFERS)>
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
    latest. After the header's state label, · 6 file changes opens a popover that counts the \
    files by kind, says what Get latest does with them, and lists them, each linked to its \
    newer version in the catalog. The file list keeps \
    all its rows. Today the dry run returns only the files a revision adds and no hash for it; \
    the links use a fixture hash, and the last cell counts changed and removed files too, which \
    needs backend work. A conflict resolves the header to the existing conflict state, which \
    offers Publish.";

/// The conflict cell's changed files, once the dry run returns them: the two
/// it conflicts on, and one more.
const CONFLICT_EXTRA: &[(&str, Change)] = &[
    ("notes/intake-upload.md", Change::Changed),
    ("notes/kickoff-thread.md", Change::Changed),
    ("raw/plate-12.csv", Change::Changed),
];

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
            <Cell full=true label="needs backend work: a conflict found before the click, opened — the header offers Publish; the conflicting files are among the changed ones, and say so">
                {page(Page {
                    clean: false,
                    opened: true,
                    extra: CONFLICT_EXTRA,
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
            <Cell full=true label="a newer revision that adds no files — no summary, no popover">
                {page(Page::new("in-nothing", ready(PullOutcome::CleanUpdate, Vec::new())))}
            </Cell>
            <Cell full=true label="needs backend work: changed and removed files counted too, opened — a deleted file has no link">
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

    /// The header's summary says how the check stands, or counts the file
    /// changes; a revision that brings no files has none, conflict or not.
    #[test]
    fn the_summary_says_what_the_check_found() {
        let added = |n: usize| (0..n).map(|i| format!("f{i}")).collect::<Vec<_>>();
        let words = |check: &PullCheck| summary_words(check, &coming(check, &[]));
        assert_eq!(
            words(&PullCheck::Loading).as_deref(),
            Some("checking\u{2026}")
        );
        assert_eq!(words(&PullCheck::Failed).as_deref(), Some("couldn't check"));
        assert_eq!(
            words(&ready(PullOutcome::CleanUpdate, added(6))).as_deref(),
            Some("6 file changes")
        );
        assert_eq!(
            words(&ready(PullOutcome::CleanUpdate, added(1))).as_deref(),
            Some("1 file change")
        );
        assert_eq!(words(&ready(PullOutcome::CleanUpdate, Vec::new())), None);
        let blocked = |n| {
            ready(
                PullOutcome::Blocked {
                    conflicts: added(n),
                },
                added(3),
            )
        };
        assert_eq!(words(&blocked(2)).as_deref(), Some("3 file changes"));
        assert_eq!(
            conflict_words(&blocked(2)).as_deref(),
            Some("2 of them conflict with yours. Publish your changes, then resolve them.")
        );
        assert_eq!(
            conflict_words(&blocked(1)).as_deref(),
            Some("1 of them conflicts with yours. Publish your changes, then resolve it.")
        );
    }

    /// The popover's sentence says what Get latest does in the scope's terms.
    #[test]
    fn the_popover_says_what_get_latest_does() {
        let added = |n: usize| (0..n).map(|i| format!("f{i}")).collect::<Vec<_>>();
        let keeps = || PullOutcome::KeepsLocalChanges {
            added: Vec::new(),
            modified: vec!["a.csv".to_string()],
            removed: Vec::new(),
        };
        let three = ready(PullOutcome::CleanUpdate, added(3));
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
    }

    /// The popover counts by kind, and says `new files` while that is all
    /// the dry run returns.
    #[test]
    fn the_row_counts_by_kind() {
        let row = |change| Coming {
            path: String::from("a"),
            change,
            conflict: false,
        };
        assert_eq!(counts_words(&[row(Change::New)]), "1 new file");
        assert_eq!(
            counts_words(&[row(Change::New), row(Change::New), row(Change::New)]),
            "3 new files"
        );
        assert_eq!(
            counts_words(&[
                row(Change::New),
                row(Change::Changed),
                row(Change::Changed),
                row(Change::Deleted)
            ]),
            "1 new, 2 changed, 1 deleted"
        );
        assert_eq!(counts_words(&[row(Change::Deleted)]), "1 deleted");
        assert_eq!(capitalised("3 new files"), "3 new files");
        assert_eq!(capitalised("new"), "New");
    }

    /// The v2 vocabulary holds in the row and the popover: none of the words
    /// the package states keep out.
    #[test]
    fn the_words_use_no_banned_word() {
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
        let mut all = vec![String::from(OPEN_LABEL)];
        for check in &checks {
            all.extend(summary_words(check, &coming(check, EXTRA)));
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

    /// Hovering opens the popover and leaving closes it after the grace; a
    /// click pins it through a leave, and a second click unpins and closes.
    #[wasm_bindgen_test]
    async fn hover_opens_and_a_click_pins() {
        use crate::gallery::forms::after_a_beat as sleep_ms;
        let open = RwSignal::new(false);
        let card = HoverCard::new(open, false);
        let grace = i32::try_from(HOVER_GRACE.as_millis()).unwrap() + 50;

        card.enter();
        assert!(open.get_untracked(), "hover opens");
        card.leave();
        card.enter();
        sleep_ms(grace).await;
        assert!(
            open.get_untracked(),
            "coming back within the grace keeps it"
        );
        card.leave();
        sleep_ms(grace).await;
        assert!(!open.get_untracked(), "leaving closes it");

        card.click();
        card.leave();
        sleep_ms(grace).await;
        assert!(open.get_untracked(), "a click pins it through a leave");
        card.click();
        assert!(!open.get_untracked(), "a second click closes it");
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
            .query_selector("#in-pick .g-nr-rows a")
            .unwrap()
            .and_then(|a| a.get_attribute("href"))
            .unwrap_or_default();
        let found = (
            count("in-pick", ".g-nr-rows li"),
            count("in-pick", ".g-nr-rows a[aria-label='Open in catalog']"),
            count("in-pick", ".g-nr-rows input, .g-nr-rows button"),
            count("in-backend", ".g-nr-rows li"),
            count("in-backend", ".g-nr-rows a"),
            container
                .query_selector("#in-nothing .g-hs")
                .unwrap()
                .is_some(),
            container
                .query_selector("#in-conflict .g-nr-rows")
                .unwrap()
                .and_then(|ul| ul.text_content())
                .unwrap_or_default()
                .matches("Conflict")
                .count(),
        );
        let capped = (
            count("in-capped", ".g-nr-rows li") as usize,
            container
                .query_selector("#in-capped .g-nr-surface")
                .unwrap()
                .and_then(|e| e.text_content())
                .unwrap_or_default(),
        );
        drop(handle);
        container.remove();

        let (rows, links, controls, backend_rows, backend_links, stray, conflicts) = found;
        assert_eq!(capped.0, LISTED, "the popover lists the first 1,000");
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
            (backend_rows, backend_links),
            (6, 5),
            "the deleted file has no link"
        );
        assert_eq!(href, catalog_href("notes/plate-07-review.md"));
        assert!(!stray, "a revision that adds nothing draws no row");
    }
}
