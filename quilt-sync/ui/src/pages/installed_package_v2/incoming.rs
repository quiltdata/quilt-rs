//! What a newer revision brings, said in the header before *Get latest*.
//!
//! After the state label, `· 6 file changes`, dash-underlined, opens a
//! popover listing them. Its pinned first line counts them by kind and says
//! what Get latest does with them, or, on a conflict, how many conflict and
//! what to do; each file links to its newer version in the catalog.
//!
//! The summary takes the UI's own `PullCheck`, which is what
//! `package_pull_outcome` returns; the header takes the state that check
//! resolves to, [`header_state`]. The page and the gallery's *Newer revision
//! available* scene draw this one component.
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
//! gates nothing.
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
//!   click, Enter or Space pins it until a click outside, Escape or a second
//!   activation. `EntryRow`'s box, size and menu columns left a path about
//!   50px there, so it lists path, labels and link only.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use leptos::prelude::*;
use quilt_uri::RevisionPointer;
use quilt_uri::S3PackageUri;

use crate::commands::PullCheck;
use crate::commands::PullOutcome;
use crate::commands::PullPreview;
use crate::kit::Align;
use crate::kit::AnchoredOverlay;
use crate::kit::Button;
use crate::kit::PackageState;
use crate::kit::StateLabel;
use crate::kit::differs_id;
use crate::kit::icons;
use crate::kit::state_label::StateTone;
use crate::pages::commit_v2::Change;
use crate::util::thousands;

stylance::import_crate_style!(style, "src/pages/installed_package_v2/incoming.module.scss");

/// The catalog link's name, as revision rows word theirs.
pub const OPEN_LABEL: &str = "Open in catalog";

/// `MouseEvent.button` for the middle button, as revision rows read it.
const MIDDLE_BUTTON: i16 = 1;

/// How many files the popover lists: the first by path, as the file list
/// loads the first 1,000. A list drawn whole costs in proportion to its rows,
/// and a revision can touch far more files than anyone reads in a popover.
pub const LISTED: usize = 1_000;

/// What the header resolves to for a check: the conflict state for a
/// `Blocked` verdict, what the page's read said for everything else.
#[must_use]
pub fn header_state(read: &PackageState, check: Option<&PullCheck>) -> PackageState {
    match check {
        Some(PullCheck::Ready(PullPreview {
            outcome: PullOutcome::Blocked { conflicts },
            ..
        })) => PackageState::PullConflict {
            files: conflicts.clone(),
        },
        _ => read.clone(),
    }
}

/// The files a `Blocked` verdict names, which the file list marks as resolve
/// mode marks the files that differ. `None` for any other check.
#[must_use]
pub fn conflicting(check: Option<&PullCheck>) -> Option<Arc<BTreeSet<String>>> {
    match check {
        Some(PullCheck::Ready(PullPreview {
            outcome: PullOutcome::Blocked { conflicts },
            ..
        })) => Some(Arc::new(conflicts.iter().cloned().collect())),
        _ => None,
    }
}

/// The header's label and the summary after it, on one line.
pub fn state_with_summary(label: AnyView, summary: Option<AnyView>) -> AnyView {
    view! { <span class=style::label>{label}{summary}</span> }.into_any()
}

/// Where the newer revision's files are read, and what opens them.
///
/// The package's address, which carries the catalog host, and the opener, so a
/// caller cannot draw a link it has no way to open: a link this app follows
/// would replace the running application. The newer revision's hash comes
/// with the check.
#[derive(Clone)]
pub struct Catalog {
    package: S3PackageUri,
    open: Callback<String>,
}

impl Catalog {
    /// `None` when the package has no catalog host: nothing to link to.
    #[must_use]
    pub fn new(package: Option<&S3PackageUri>, open: Callback<String>) -> Option<Self> {
        let package = package.filter(|p| p.catalog.is_some())?;
        Some(Self {
            package: package.clone(),
            open,
        })
    }

    /// A file at the newer revision, in the catalog.
    fn href(&self, hash: &str, path: &str) -> Option<String> {
        let newer = S3PackageUri {
            revision: RevisionPointer::Hash(hash.to_string()),
            path: None,
            ..self.package.clone()
        };
        crate::util::entry_catalog_url(&newer, path)
    }
}

/// How many files of each kind a newer revision brings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Kinds {
    new: usize,
    changed: usize,
    deleted: usize,
}

impl Kinds {
    fn of(rows: &[Coming]) -> Self {
        let n = |c: Change| rows.iter().filter(|r| r.change == c).count();
        Self {
            new: n(Change::New),
            changed: n(Change::Changed),
            deleted: n(Change::Deleted),
        }
    }
}

/// `it`, or `them`.
const fn them(n: usize) -> &'static str {
    if n == 1 { "it" } else { "them" }
}

/// Removing deleted files, which touches only those this copy has: a pull
/// deletes the paths it tracks, and a deleted file never downloaded here has
/// nothing to remove. `alone` when nothing else is coming.
fn removes(deleted: usize, alone: bool) -> &'static str {
    match (alone, deleted == 1) {
        (true, true) => "removes it if you have it",
        (true, false) => "removes any of them you have",
        (false, true) => "removes the deleted one if you have it",
        (false, false) => "removes the deleted ones you have",
    }
}

/// `one`, or `ones`.
const fn ones(n: usize) -> &'static str {
    if n == 1 { "one" } else { "ones" }
}

/// What *Get latest* does with the files, heading the popover, naming each
/// operation for the kinds there are: new and changed files are downloaded or
/// updated, deleted ones removed, never downloaded. `whole` is the sync
/// scope: under individual-file sync new files are listed, to download when
/// wanted, and only the files this copy has are updated. Local changes the
/// update keeps are named too.
fn scope_words(check: &PullCheck, whole: bool, k: Kinds) -> Option<String> {
    let PullCheck::Ready(preview) = check else {
        return None;
    };
    let Kinds {
        new,
        changed,
        deleted,
    } = k;
    if new + changed + deleted == 0 {
        return None;
    }
    let fate = if whole {
        let fetched = new + changed;
        let download = match (new, changed, deleted) {
            (_, _, 0) => format!("downloads {}", them(fetched)),
            (0, _, _) => format!("downloads the changed {}", ones(changed)),
            (_, 0, _) => format!("downloads the new {}", ones(new)),
            _ => String::from("downloads the new and changed ones"),
        };
        match (fetched, deleted) {
            (_, 0) => download,
            (0, _) => removes(deleted, true).to_string(),
            _ => format!("{download} and {}", removes(deleted, false)),
        }
    } else if changed == 0 && deleted == 0 {
        format!(
            "adds {} to your files, to download when you need {}",
            them(new),
            them(new)
        )
    } else {
        let mut acts = Vec::new();
        if changed > 0 {
            acts.push(String::from("updates the files you have"));
        }
        if deleted > 0 {
            acts.push(removes(deleted, changed == 0 && new == 0).to_string());
        }
        let acts = acts.join(" and ");
        if new > 0 {
            format!(
                "{acts}, and lists the new {}, to download when you need {}",
                ones(new),
                them(new)
            )
        } else {
            acts
        }
    };
    let kept = matches!(preview.outcome, PullOutcome::KeepsLocalChanges { .. });
    Some(if kept {
        format!("Get latest {fate}. Your changes stay.")
    } else {
        format!("Get latest {fate}.")
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

/// The popover's rows: the files the newer revision adds, changes and
/// removes, by path.
fn coming(check: &PullCheck) -> Vec<Coming> {
    let PullCheck::Ready(preview) = check else {
        return Vec::new();
    };
    let conflicts: &[String] = match &preview.outcome {
        PullOutcome::Blocked { conflicts } => conflicts,
        _ => &[],
    };
    let mut rows: Vec<Coming> = [
        (&preview.added, Change::New),
        (&preview.changed, Change::Changed),
        (&preview.removed, Change::Deleted),
    ]
    .into_iter()
    .flat_map(|(paths, change)| paths.iter().map(move |p| (p.clone(), change)))
    .map(|(path, change)| Coming {
        conflict: conflicts.contains(&path),
        path,
        change,
    })
    .collect();
    rows.sort_by(|a, b| a.path.cmp(&b.path));
    rows
}

/// `3 new files`, or `3 new, 2 changed, 1 deleted`.
fn counts_words(rows: &[Coming]) -> String {
    let Kinds {
        new,
        changed,
        deleted,
    } = Kinds::of(rows);
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
        let timer = StoredValue::new(None::<TimeoutHandle>);
        on_cleanup(move || {
            if let Some(Some(pending)) = timer.try_update_value(Option::take) {
                pending.clear();
            }
        });
        Self {
            open,
            pinned,
            timer,
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
///
/// `None` in `check` draws nothing: the package is not behind.
#[component]
pub fn IncomingSummary(
    /// The dry run's answer, or how it stands. A memo on the page, so an
    /// answer equal to the last one draws nothing anew and a popover the reader
    /// has open stays open.
    #[prop(into)]
    check: Signal<Option<PullCheck>>,
    /// `Keeping → The whole package`.
    whole: bool,
    /// Where the files are read; `None` draws no links.
    #[prop(optional_no_strip)]
    catalog: Option<Catalog>,
    /// *Try again*, after a failed check.
    on_retry: Callback<()>,
    /// The popover starts open and pinned, as a reader's click leaves it.
    #[prop(optional)]
    opened: bool,
) -> impl IntoView {
    move || {
        check
            .get()
            .and_then(|check| summary(&check, whole, catalog.clone(), on_retry, opened))
    }
}

fn summary(
    check: &PullCheck,
    whole: bool,
    catalog: Option<Catalog>,
    on_retry: Callback<()>,
    opened: bool,
) -> Option<AnyView> {
    let rows = coming(check);
    let words = summary_words(check, &rows)?;
    let PullCheck::Ready(preview) = check else {
        let failed = check.is_failed();
        return Some(
            view! {
                <span class=style::summary>
                    <span class=style::muted>"\u{b7}"</span>
                    <span class=style::muted>{words}</span>
                    {failed
                        .then(|| {
                            view! { <Button on_click=move |_| on_retry.run(())>"Try again"</Button> }
                        })}
                </span>
            }
            .into_any(),
        );
    };
    let counts = counts_words(&rows);
    let kinds = Kinds::of(&rows);
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
    let id = conflict.is_some().then(differs_id);
    let scope = conflict.or_else(|| scope_words(check, whole, kinds));
    let links = catalog.zip(preview.latest_hash.clone());
    Some(
        view! {
            <span class=style::summary>
                <span class=style::muted>"\u{b7}"</span>
                <AnchoredOverlay
                    trigger=move |surface_id: String| {
                        view! {
                            <button
                                class=style::trigger
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
                        class=style::surface
                        on:mouseenter=move |_| card.enter()
                        on:mouseleave=move |_| card.leave()
                    >
                        <p class=style::scope id=id>
                            <strong>{format!("{}.", capitalised(&counts))}</strong>
                            {scope.map(|w| format!(" {w}"))}
                        </p>
                        {file_list(rows, links.as_ref())}
                        {cut.map(|words| view! { <p class=style::cut>{words}</p> })}
                    </div>
                </AnchoredOverlay>
            </span>
        }
        .into_any(),
    )
}

/// The link to a file's newer version in the catalog, drawn as revision rows
/// draw theirs: a real anchor, so the address can be copied, whose navigation
/// is cancelled and handed to the opener, which on the page is the browser.
fn catalog_link(href: String, open: Callback<String>) -> AnyView {
    let followed = href.clone();
    let middled = href.clone();
    view! {
        <a
            class=style::open
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
/// deleted file keeps the link's room, so the labels stay in a column; with no
/// catalog, no row keeps it.
fn file_list(rows: Vec<Coming>, links: Option<&(Catalog, String)>) -> AnyView {
    view! {
        <ul class=style::rows>
            {rows
                .into_iter()
                .map(|row| {
                    let link = links
                        .map(|(catalog, hash)| {
                            match catalog.href(hash, &row.path).filter(|_| row.change != Change::Deleted)
                            {
                                Some(href) => catalog_link(href, catalog.open),
                                None => view! { <span class=style::open /> }.into_any(),
                            }
                        });
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{mount, sleep_ms};
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::*;

    fn ready(outcome: PullOutcome, added: Vec<String>) -> PullCheck {
        PullCheck::Ready(PullPreview {
            outcome,
            added,
            changed: Vec::new(),
            removed: Vec::new(),
            latest_hash: Some("9f3c1a2b7d4e".to_string()),
        })
    }

    fn three_added() -> Vec<String> {
        vec![
            "a.md".to_string(),
            "b.json".to_string(),
            "c.csv".to_string(),
        ]
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
            header_state(&PackageState::Behind, Some(&blocked)),
            PackageState::PullConflict {
                files: vec!["a.csv".to_string()]
            }
        );
        assert_eq!(
            conflicting(Some(&blocked)).map(|s| s.iter().cloned().collect::<Vec<_>>()),
            Some(vec!["a.csv".to_string()])
        );
        for check in [
            PullCheck::Loading,
            PullCheck::Failed,
            ready(PullOutcome::CleanUpdate, three_added()),
        ] {
            assert_eq!(
                header_state(&PackageState::Behind, Some(&check)),
                PackageState::Behind
            );
            assert_eq!(conflicting(Some(&check)), None);
        }
        assert_eq!(
            header_state(&PackageState::Latest, None),
            PackageState::Latest
        );
    }

    /// The header's summary says how the check stands, or counts the file
    /// changes; a revision that brings no files has none, conflict or not.
    #[test]
    fn the_summary_says_what_the_check_found() {
        let added = |n: usize| (0..n).map(|i| format!("f{i}")).collect::<Vec<_>>();
        let words = |check: &PullCheck| summary_words(check, &coming(check));
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
        let PullCheck::Ready(mut mixed) = ready(PullOutcome::CleanUpdate, added(1)) else {
            unreachable!()
        };
        mixed.changed = vec!["g".to_string(), "h".to_string()];
        mixed.removed = vec!["i".to_string()];
        assert_eq!(
            words(&PullCheck::Ready(mixed)).as_deref(),
            Some("4 file changes"),
            "new, changed and removed alike"
        );
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

    /// The popover's sentence says what Get latest does in the scope's terms,
    /// naming only operations for kinds there are: a deleted file is removed,
    /// never downloaded, and no sentence promises new files that are not
    /// coming.
    #[test]
    fn the_popover_says_what_get_latest_does() {
        let clean = ready(PullOutcome::CleanUpdate, Vec::new());
        let keeps = ready(
            PullOutcome::KeepsLocalChanges {
                added: Vec::new(),
                modified: vec!["a.csv".to_string()],
                removed: Vec::new(),
            },
            Vec::new(),
        );
        let k = |new, changed, deleted| Kinds {
            new,
            changed,
            deleted,
        };
        let cases: Vec<(&PullCheck, bool, Kinds, Option<&str>)> = vec![
            (
                &clean,
                false,
                k(3, 0, 0),
                Some("Get latest adds them to your files, to download when you need them."),
            ),
            (&clean, true, k(1, 0, 0), Some("Get latest downloads it.")),
            (
                &keeps,
                true,
                k(2, 0, 0),
                Some("Get latest downloads them. Your changes stay."),
            ),
            (&clean, true, k(0, 0, 0), None),
            (
                &clean,
                false,
                k(3, 2, 1),
                Some(
                    "Get latest updates the files you have and removes the deleted one if you \
                     have it, and lists the new ones, to download when you need them.",
                ),
            ),
            (
                &clean,
                false,
                k(0, 2, 0),
                Some("Get latest updates the files you have."),
            ),
            (
                &clean,
                false,
                k(0, 0, 2),
                Some("Get latest removes any of them you have."),
            ),
            (
                &clean,
                false,
                k(0, 2, 1),
                Some(
                    "Get latest updates the files you have and removes the deleted one if you have it.",
                ),
            ),
            (
                &clean,
                true,
                k(3, 2, 1),
                Some(
                    "Get latest downloads the new and changed ones and removes the deleted one if you have it.",
                ),
            ),
            (&clean, true, k(0, 2, 0), Some("Get latest downloads them.")),
            (
                &clean,
                true,
                k(0, 0, 1),
                Some("Get latest removes it if you have it."),
            ),
            (
                &clean,
                true,
                k(0, 1, 2),
                Some("Get latest downloads the changed one and removes the deleted ones you have."),
            ),
        ];
        for (check, whole, kinds, expected) in cases {
            assert_eq!(
                scope_words(check, whole, kinds).as_deref(),
                expected,
                "{kinds:?}, whole: {whole}"
            );
        }
    }

    /// The popover counts by kind, and says `new files` when that is all
    /// there is.
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
            all.extend(summary_words(check, &coming(check)));
            all.extend(conflict_words(check));
            for whole in [false, true] {
                for kinds in [
                    Kinds {
                        new: 2,
                        changed: 0,
                        deleted: 0,
                    },
                    Kinds {
                        new: 2,
                        changed: 1,
                        deleted: 1,
                    },
                    Kinds {
                        new: 0,
                        changed: 1,
                        deleted: 2,
                    },
                ] {
                    all.extend(scope_words(check, whole, kinds));
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
        let owner = Owner::new();
        let (open, card) = owner.with(|| {
            let open = RwSignal::new(false);
            (open, HoverCard::new(open, false))
        });
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
        drop(owner);
    }

    /// Links go to each file at the newer revision on the package's catalog
    /// host; a deleted file has none, and a package with no catalog host has
    /// none at all.
    #[wasm_bindgen_test]
    fn links_go_to_the_newer_revision_and_need_a_catalog_host() {
        let check = PullCheck::Ready(PullPreview {
            outcome: PullOutcome::CleanUpdate,
            added: vec!["raw/a.csv".to_string()],
            changed: vec!["b.md".to_string()],
            removed: vec!["c.md".to_string()],
            latest_hash: Some("abc123".to_string()),
        });
        let draw = |host: Option<&'static str>| {
            let check = check.clone();
            mount(move || {
                let package = crate::util::package_uri(
                    "lab-bucket",
                    &"user/plate-07".try_into().unwrap(),
                    host,
                );
                view! {
                    <IncomingSummary
                        check=Signal::stored(Some(check))
                        whole=false
                        catalog=Catalog::new(Some(&package), Callback::new(|_: String| ()))
                        on_retry=Callback::new(|()| ())
                    />
                }
            })
        };
        let hrefs = |el: &web_sys::Element| {
            let found = el.query_selector_all("ul a").unwrap();
            (0..found.length())
                .map(|i| {
                    found
                        .item(i)
                        .unwrap()
                        .unchecked_into::<web_sys::Element>()
                        .get_attribute("href")
                        .unwrap_or_default()
                })
                .collect::<Vec<_>>()
        };
        let linked = draw(Some("lab.example"));
        assert_eq!(
            hrefs(&linked),
            vec![
                "https://lab.example/b/lab-bucket/packages/user/plate-07/tree/abc123/b.md",
                "https://lab.example/b/lab-bucket/packages/user/plate-07/tree/abc123/raw/a.csv",
            ],
            "markup was {}",
            linked.inner_html()
        );
        let bare = draw(None);
        assert!(hrefs(&bare).is_empty(), "no host, no links");
        assert_eq!(bare.query_selector_all("ul li").unwrap().length(), 3);
    }
}
