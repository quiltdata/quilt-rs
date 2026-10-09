//! The v2 package page. Behind `ExperimentalSettings.main_page_v2`, the
//! *New design preview* switch, as the v2 main page is.
//!
//! The header and the context pane are drawn from one authoritative read. The
//! pane has two modes: the ordinary one, and Resolve, which `resolve=1` asks
//! for and a diverged package's comparison makes real. The file pane, the
//! shell's growing left side, comes after the context pane in the DOM, so
//! reading and focus order is context first; the stylesheet places it. Its
//! list arrives with the same read, classified by the status the header's
//! state comes from, so the two cannot disagree and the tree is walked once.
//! In resolve mode it marks the page's one differing set, [`FileMarks`].
//!
//! A header that is behind or in conflict names what the newer revision
//! brings, from the page's own dry run ([`IncomingCheck`]), which runs after
//! each such read; a conflict it finds resolves the header to the conflict
//! state and marks the conflicting rows.

use std::collections::BTreeSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use leptos::prelude::*;
use leptos_router::NavigateOptions;
use leptos_router::hooks::{use_navigate, use_query_map};

use crate::commands;
use crate::kit::{Banner, BannerVariant, LoadFailure, PageLayout};
use crate::routes;

use super::status_watch::StatusWatch;
#[cfg(test)]
use super::v2_page::outcome_band;
use super::v2_page::{PageHandle, V2Page};
use crate::components::{
    IgnorePopup, IgnorePopupData, Notification, UnignorePopup, UnignorePopupData,
};

mod bucket_form;
pub(crate) mod context_pane;
pub(crate) mod file_pane;
mod header;
pub(crate) mod incoming;
pub(crate) mod keeping;
mod local_only_band;
mod mismatch_band;
pub(crate) mod old_revisions;
pub(crate) mod resolve;
mod revision_history;
mod role_dialog;

use crate::kit::PageHeaderSkeleton;
use context_pane::{CurrentRevisionPane, CurrentRevisionPaneSkeleton};
use file_pane::{Facet, FilePane, FilePaneSkeleton, Grouping, Listing, Picking};
use file_pane::{Reach, RowCommand, RowMenu};
use header::PageHeader;
pub use header::{MenuCommand, MenuItem, menu_items};
use incoming::{HoverCard, IncomingSummary};
use resolve::{ResolveCommands, ResolvePane};

stylance::import_crate_style!(style, "src/pages/installed_package_v2.module.scss");

pub use super::v2_page::Outcome;

/// The page's half of a command: what it blocks while it runs, where it
/// reports, what to re-read when it is done, where it goes, and which dialog
/// holds it.
///
/// # Owned by the page, because a re-read rebuilds the header
///
/// Every re-read that finds news — the watcher's, Refresh, a command's own
/// `reload` — re-runs the body and builds a new `PageHeader`, disposing
/// everything the old one owned. (One that finds nothing new rebuilds nothing:
/// see `v2_page`.) So anything that must outlive a re-read lives here: a dialog the
/// reader has open stays open, and a navigation asked for by a command that
/// settles after the rebuild still happens.
///
/// Public so the gallery can hand the live context pane an idle one.
#[derive(Clone, Copy)]
pub struct Wiring {
    /// One command at a time. Every control on the header reads this, so a
    /// second cannot start on top of the first — two writes to one working
    /// tree is a race the page has no way to arbitrate.
    pub busy: RwSignal<bool>,
    /// Whether the command holding `busy` is Keeping's download. Here, not in
    /// the pane: the download writes files, the watcher re-reads, and the
    /// rebuilt button must keep its spinner.
    pub downloading: RwSignal<bool>,
    /// What the last command said, and which package it said it about. Keyed,
    /// because a result arriving for a package the page no longer shows is not
    /// this page's news — see `outcome_band`.
    pub outcome: RwSignal<Option<Outcome>>,
    pub reload: Trigger,
    /// Where the header wants to go. A signal because a `Callback` must be
    /// `Send + Sync` and `use_navigate`'s closure is neither; [`Wiring::follow`]
    /// performs it.
    pub goto: RwSignal<Option<String>>,
    /// Where the page goes in place of the current entry, so *Back* never
    /// returns to it: leaving the resolve mode on a success. Keyed, as
    /// `outcome` is: [`Wiring::follow`] drops it once its package is off screen.
    pub replace_to: RwSignal<Option<Replace>>,
    pub dialogs: Dialogs,
    /// The context pane's removal of old revisions: what runs, and what its
    /// confirmation asks.
    pub removal: old_revisions::Removal,
}

/// Hold `busy` for as long as `task` runs, and retract the band's last
/// outcome as it starts. Every header command does, the dialogs' included: the
/// signal is the page's and a dialog's own seal is not, so a dialog rebuilt
/// mid-submit is drawn sealed by this one.
async fn holding<T>(
    busy: RwSignal<bool>,
    outcome: RwSignal<Option<Outcome>>,
    task: impl std::future::Future<Output = T>,
) -> T {
    super::v2_page::hold(busy, outcome, task).await
}

/// Run a command, hold the page while it runs, and report only what the band
/// is for.
///
/// Success says nothing here. A command whose success IS worth a sentence —
/// undo — sets its own outcome, because it is the exception rather than the
/// rule, and a helper that reported every success would put `Get latest`'s
/// line on the page behind the toast that already carried its report.
///
/// Starting retracts whatever the band said last, in `holding`, so a failure
/// does not outlive the retry that succeeds.
///
/// `on_failure` is the page's own sentence for the command not happening; the
/// backend's text follows it as the detail, which is the split the pause band
/// already makes.
///
/// `after` says when the page reads the package again. A re-read leaves the
/// band alone, so a failure stays on screen above the fresh details.
pub(super) fn run(
    busy: RwSignal<bool>,
    outcome: RwSignal<Option<Outcome>>,
    namespace: String,
    on_failure: &'static str,
    after: Reread,
    task: impl std::future::Future<Output = Result<String, String>> + 'static,
) {
    // The controls are disabled while this is true, so this guard only
    // catches a press already in flight when the signal was written.
    if busy.get_untracked() {
        return;
    }
    leptos::task::spawn_local(async move {
        let answer = holding(busy, outcome, task).await;
        let reload = match (after, answer.is_ok()) {
            (Reread::OnSuccess(reload), true) | (Reread::Always(reload), _) => Some(reload),
            _ => None,
        };
        if let Err(message) = answer {
            outcome.set(Some(Outcome {
                namespace,
                variant: BannerVariant::Critical,
                lead: on_failure.to_string(),
                detail: Some(message),
            }));
        }
        if let Some(reload) = reload {
            reload.notify();
        }
    });
}

/// When a command run by [`run`] has the page read its package again.
#[derive(Clone, Copy)]
pub(super) enum Reread {
    /// Not at all: the command changes nothing the page shows.
    Never,
    /// Only after a success. A failure here left the package as it was, so
    /// what is on screen is still true.
    OnSuccess(Trigger),
    /// Whether it succeeds or not, because it can fail halfway. Publish
    /// commits and then pushes, so a refused push still leaves a new local
    /// revision the page has to show.
    Always(Trigger),
}

/// A navigation in place of the current entry, and the package it leaves.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Replace {
    pub namespace: String,
    pub to: String,
}

/// Which of the header's dialogs is open.
#[derive(Clone, Copy)]
pub struct Dialogs {
    /// The row's `Choose S3 bucket` and the menu's `Change bucket` open this
    /// one dialog: the state calls for it, or the reader chooses it.
    pub bucket: RwSignal<bool>,
    /// What the reader has typed into that dialog. Here beside its flag, for
    /// the flag's reason: a re-read rebuilds the dialog, and the rebuilt one
    /// must not put the package's remote back over the draft. One handle to
    /// the draft's signals rather than the signals inline, because `Wiring`
    /// is passed by value to every part of the header.
    pub bucket_draft: StoredValue<BucketDraft>,
    /// Opened only by the row's `Switch role`, which exists only when the
    /// payload names somewhere to switch to.
    pub role: RwSignal<bool>,
    /// The two Danger items: the menu picks the command, the dialog accepts
    /// the consequence.
    pub undo: RwSignal<bool>,
    pub remove: RwSignal<bool>,
    /// Remove's *Also delete downloaded files from disk*. Here for the flags'
    /// reason: a re-read must not tick a box the reader just cleared.
    pub remove_prune: RwSignal<bool>,
    /// Resolve's *Replace mine*: here, so a re-read mid-reset keeps it open.
    pub replace: RwSignal<bool>,
}

/// A workflow read's successful answer, with the target — host and bucket —
/// it was asked for. Only a success: a failure is shown but not kept, so the
/// next read asks again.
pub type WorkflowAnswer = ((String, String), commands::CommitWorkflows);

/// The bucket dialog's fields, held by the page. Filled from the package's
/// remote only when the dialog opens, going from closed to open; a dialog
/// rebuilt while open finds them as the reader left them.
#[derive(Clone, Copy)]
pub struct BucketDraft {
    pub host: RwSignal<String>,
    pub bucket: RwSignal<String>,
    /// The workflow's label, as the dialog's select names it.
    pub workflow: RwSignal<String>,
    /// The target — host and bucket — the workflow was chosen for. A read of
    /// the same target keeps the choice, because the rebuilt dialog views it
    /// again; a different target, or a fresh opening, restarts at the
    /// bucket's preselection.
    pub workflow_for: RwSignal<Option<(String, String)>>,
    /// The last successful workflow read, with the target it was for. The
    /// rebuilt dialog takes it rather than asking again, because a second
    /// answer could differ — a failure would reset the reader's choice to the
    /// bucket default. A failure is not kept: it is most likely passing, and
    /// kept it would hold the form on the bucket default after the remote
    /// recovers, so the rebuilt dialog asks again instead. A fresh opening
    /// clears it, so each opening reads anew.
    pub workflows: RwSignal<Option<WorkflowAnswer>>,
    /// The package the draft was filled for. One route serves every package,
    /// so a draft left from another one is refilled rather than shown.
    pub namespace: RwSignal<Option<String>>,
}

impl BucketDraft {
    fn new() -> Self {
        Self {
            host: RwSignal::new(String::new()),
            bucket: RwSignal::new(String::new()),
            workflow: RwSignal::new(String::new()),
            workflow_for: RwSignal::new(None),
            workflows: RwSignal::new(None),
            namespace: RwSignal::new(None),
        }
    }
}

impl Wiring {
    #[must_use]
    pub fn new() -> Self {
        Self::over(PageHandle::new())
    }

    /// The header's wiring over the scaffold's lock, band and reload, so the
    /// page's commands and the scaffold's frame share them.
    fn over(page: PageHandle) -> Self {
        Self {
            busy: page.busy,
            downloading: RwSignal::new(false),
            outcome: page.outcome,
            reload: page.reload,
            goto: RwSignal::new(None),
            replace_to: RwSignal::new(None),
            dialogs: Dialogs {
                bucket: RwSignal::new(false),
                bucket_draft: StoredValue::new(BucketDraft::new()),
                role: RwSignal::new(false),
                undo: RwSignal::new(false),
                remove: RwSignal::new(false),
                remove_prune: RwSignal::new(true),
                replace: RwSignal::new(false),
            },
            removal: old_revisions::Removal::new(),
        }
    }

    /// Perform `goto`, and `replace_to` while `showing` is its package. Called
    /// once, by whoever owns the signals, inside a router.
    fn follow(self, showing: Signal<String>) {
        let Self {
            goto, replace_to, ..
        } = self;
        let navigate = use_navigate();
        Effect::new(move |_| {
            if let Some(target) = goto.get() {
                navigate(&target, NavigateOptions::default());
                goto.set(None);
            }
            if let Some(Replace { namespace, to }) = replace_to.get() {
                replace_to.set(None);
                if namespace != showing.get_untracked() {
                    return;
                }
                navigate(
                    &to,
                    NavigateOptions {
                        replace: true,
                        ..NavigateOptions::default()
                    },
                );
            }
        });
    }
}

/// An idle page: nothing running, nothing said, no dialog open.
impl Default for Wiring {
    fn default() -> Self {
        Self::new()
    }
}

/// The file pane's inputs that outlive a re-read of the page.
///
/// The body is rebuilt on every re-read, so the `Group:` choice, the
/// collapsed folders, the search and the facet live with the page: the watcher's news
/// must not reset them. The list itself is the answer's, drawn with its header.
#[derive(Clone, Copy)]
struct Files {
    grouping: RwSignal<String>,
    collapsed: RwSignal<BTreeSet<String>>,
    search: RwSignal<String>,
    facet: RwSignal<String>,
    retry: Callback<()>,
    /// The ticked paths, which a re-read keeps and another package clears.
    ticked: RwSignal<BTreeSet<String>>,
    /// The footer's `[Download]` is running. Here for `Wiring::downloading`'s
    /// reason: the rebuilt button must keep its spinner.
    downloading: RwSignal<bool>,
    /// The ignore popup a row's *Ignore* opened. The page's, so the watcher's
    /// re-read neither closes it nor loses what was typed.
    ignoring: RwSignal<Option<IgnorePopupData>>,
    /// The unignore popup a row's *Stop ignoring* opened, for the same reason.
    unignoring: RwSignal<Option<UnignorePopupData>>,
}

/// What the panes read. Built by `package_body`: the resolve pane counts
/// `differing`, and the file pane marks its rows by it.
#[derive(Clone, Copy)]
pub struct FileMarks {
    /// Keys are `EntryData.filename`'s form: the logical key's `display()`.
    pub differing: Memo<Option<Arc<BTreeSet<String>>>>,
    /// The scope as read back (`KeepingData.scope`), never one still being stored.
    pub scope: commands::KeepingScope,
}

/// The page's one differing set: present only while the mode is open on a
/// diverged package with a comparison (`#resolve-seams`).
pub fn differing_marks(
    open: Signal<bool>,
    resolve: Option<&commands::ResolveData>,
) -> Memo<Option<Arc<BTreeSet<String>>>> {
    let set = match resolve {
        Some(commands::ResolveData::Compared { differing, .. }) => {
            Some(Arc::new(differing.iter().cloned().collect::<BTreeSet<_>>()))
        }
        Some(commands::ResolveData::Refused { .. }) | None => None,
    };
    Memo::new(move |_| if open.get() { set.clone() } else { None })
}

/// The dry run, for one namespace. A seam, like [`PageRead`], so a test can
/// answer without a Tauri host.
pub(crate) type PullRead =
    fn(String) -> Pin<Box<dyn Future<Output = Result<commands::PullPreview, String>>>>;

fn read_pull_outcome(
    namespace: String,
) -> Pin<Box<dyn Future<Output = Result<commands::PullPreview, String>>>> {
    Box::pin(commands::package_pull_outcome(namespace))
}

/// Whether a read's header is one the dry run speaks to: a newer revision
/// available, or the conflict a failed *Get latest* left.
fn checks_incoming(state: &crate::kit::PackageState) -> bool {
    matches!(
        state,
        crate::kit::PackageState::Behind | crate::kit::PackageState::PullConflict { .. }
    )
}

/// The dry run's answer, and what it was asked about.
///
/// Keyed twice. By package, because one route serves every package and an
/// answer can land after the reader moved on. By the revision this copy
/// holds, because an answer is the difference from it: once *Get latest*
/// moves it, the last answer counts files that are no longer coming, even
/// when the read after it finds a newer revision again.
#[derive(Clone, Debug, PartialEq, Eq)]
struct IncomingAnswer {
    namespace: String,
    /// `CurrentRevisionData.hash` of the read that asked.
    holds: String,
    check: commands::PullCheck,
    /// Whether the run that answered finished after the current read. A
    /// re-read that keeps the answer clears it, and the next run's answer
    /// sets it again. Only a fresh answer moves the header's state.
    fresh: bool,
}

/// The newer revision's dry run, owned by the page.
///
/// Above the rebuild line, for `Wiring`'s reason: every re-read rebuilds the
/// header, and an answer held there would go back to `checking…` on each of
/// the watcher's reports. Signals, not a resource, for the commit page's
/// reason: a resource read under `ByDesign`'s `Suspense` would put the loading
/// frame back over the page on every rerun.
///
/// It runs again after every read that shows the package behind or in
/// conflict, since a local edit can make a conflict while the newer revision
/// stays the same. While it runs, the last answer for the same package and
/// the same held revision stays up, marked stale: it still draws the summary
/// and the popover, but the header's state is the read's until the new run
/// answers, so an answer about an earlier read cannot hide a conflict the
/// re-read records, nor paint one it no longer does. Each run is numbered,
/// and only the latest one's answer lands.
#[derive(Clone, Copy)]
struct IncomingCheck {
    answer: RwSignal<Option<IncomingAnswer>>,
    /// The number of the latest run. An answer from an earlier one is dropped.
    runs: StoredValue<u64>,
    /// What the last read asked about, for *Try again*.
    asked: StoredValue<Option<(String, String)>>,
    pull: PullRead,
    /// The summary's popover, open or pinned. Here for the answer's reason:
    /// the header is drawn again on a re-read whose data differs and when the
    /// check moves the state, and a popover the reader pinned stays open
    /// through both.
    card: HoverCard,
}

impl IncomingCheck {
    fn new(pull: PullRead) -> Self {
        Self {
            answer: RwSignal::new(None),
            runs: StoredValue::new(0),
            asked: StoredValue::new(None),
            pull,
            card: HoverCard::new(false),
        }
    }

    /// A read answered. `asks` is its package and the revision it holds when
    /// it shows the package behind or in conflict, and `None` otherwise: then
    /// no answer stands, and any still out is dropped when it lands.
    fn after_read(self, asks: Option<(String, String)>) {
        self.asked.set_value(asks.clone());
        let Some((namespace, holds)) = asks else {
            self.runs.update_value(|n| *n += 1);
            self.answer.set(None);
            return;
        };
        let keep = self.answer.with_untracked(|a| {
            a.as_ref().is_some_and(|a| {
                a.namespace == namespace
                    && a.holds == holds
                    && matches!(a.check, commands::PullCheck::Ready(_))
            })
        });
        if keep {
            self.answer.update(|a| {
                if let Some(a) = a {
                    a.fresh = false;
                }
            });
        }
        self.run(namespace, holds, keep);
    }

    /// *Try again*, after a failed check.
    fn retry(self) {
        if let Some((namespace, holds)) = self.asked.get_value() {
            self.run(namespace, holds, false);
        }
    }

    fn run(self, namespace: String, holds: String, keep: bool) {
        self.runs.update_value(|n| *n += 1);
        let this = self.runs.get_value();
        if !keep {
            self.answer.set(Some(IncomingAnswer {
                namespace: namespace.clone(),
                holds: holds.clone(),
                check: commands::PullCheck::Loading,
                fresh: true,
            }));
        }
        let Self {
            answer, runs, pull, ..
        } = self;
        leptos::task::spawn_local(async move {
            let check = match pull(namespace.clone()).await {
                Ok(preview) => commands::PullCheck::Ready(preview),
                Err(_) => commands::PullCheck::Failed,
            };
            // `try_`: the page can be gone by now.
            if runs.try_get_value() == Some(this) {
                answer.try_set(Some(IncomingAnswer {
                    namespace,
                    holds,
                    check,
                    fresh: true,
                }));
            }
        });
    }

    /// The check for one read's package and held revision, and whether its
    /// answer is fresh: its answer, or `Loading` until one lands for that key.
    fn check_for(self, namespace: String, holds: String) -> Memo<(commands::PullCheck, bool)> {
        let answer = self.answer;
        Memo::new(move |_| {
            answer.with(|a| {
                a.as_ref()
                    .filter(|a| a.namespace == namespace && a.holds == holds)
                    .map_or((commands::PullCheck::Loading, true), |a| {
                        (a.check.clone(), a.fresh)
                    })
            })
        })
    }
}

/// The address's deep-link outcome — a mismatch or the local-only flag —
/// provided by the page for the addresses it builds.
///
/// One carry for both, not one beside the other: the address holds one or
/// the other, and a site that carried one could drop the other.
#[derive(Clone, Copy)]
struct Carried(Memo<Option<routes::DeepLinkOutcome>>);

/// `href` with the page's deep-link outcome after it, so entering Resolve,
/// leaving it, or replacing the address does not end that outcome's band.
fn carrying(href: String) -> String {
    let outcome = use_context::<Carried>().and_then(|Carried(o)| o.get_untracked());
    routes::keeping_outcome(href, outcome.as_ref())
}

/// Where this page should be instead, when it was asked for a mode the
/// package does not have: only an answered read about the package on screen decides.
/// A deep link's outcome stays on the address, so its band does not end with the mode.
fn normalized_address(
    asked: bool,
    showing: &str,
    answered: &commands::PackagePageData,
    outcome: Option<&routes::DeepLinkOutcome>,
) -> Option<String> {
    let namespace = &answered.header.namespace;
    (asked && namespace.to_string() == showing && answered.context.resolve.is_none())
        .then(|| routes::keeping_outcome(routes::package_page_href(namespace), outcome))
}

/// Render one successful page payload. Kept pure so its atomic shape can be
/// tested without pretending the wasm runner has a Tauri host.
///
/// `asked` is the address's `resolve=1`; the mode is open only when this
/// payload also carries a comparison, so the header, the pane and the marks
/// all read one `open`.
fn package_body(
    data: commands::PackagePageData,
    w: Wiring,
    asked: Signal<bool>,
    resolving: ResolveCommands,
    files: Files,
    incoming: IncomingCheck,
) -> AnyView {
    let commands::PackagePageData {
        header,
        context,
        files: listed,
        ..
    } = data;
    // From the same answer as the header, so it is always this package's.
    let listing = Signal::stored(Listing::from(listed));
    let diverged = context.resolve.is_some();
    let open = Memo::new(move |_| asked.get() && diverged);
    let marks = FileMarks {
        differing: differing_marks(open.into(), context.resolve.as_ref()),
        scope: context.keeping.scope,
    };
    let picking = Picking {
        ticked: files.ticked,
        downloading: files.downloading.into(),
        busy: w.busy.into(),
        on_download: file_downloader(header.namespace.to_string(), w, files),
        whole_package: marks.scope == commands::KeepingScope::EntirePackage,
    };
    // The confirmation's flag is the page's, so it outlives the mode unless
    // closed here: a mode that reopens must not find it already open.
    let replace = w.dialogs.replace;
    Effect::new(move |_| {
        if !open.get() && replace.get_untracked() {
            replace.set(false);
        }
    });

    let ns = header.namespace.clone();
    let uri = header.uri.clone();
    let namespace = ns.to_string();
    let (page_header, row_marks) = incoming_header(
        header,
        context.revision.hash.clone(),
        w,
        open,
        marks,
        incoming,
    );
    let open_catalog = catalog_opener(namespace.clone(), w.outcome);
    let open_file = file_opener(namespace.clone(), uri.clone(), w.outcome);
    let menu = RowMenu {
        reach: Reach {
            remote: uri.is_some(),
            catalog: uri.as_ref().is_some_and(|u| u.catalog.is_some()),
        },
        busy: w.busy.into(),
        on_command: row_runner(namespace.clone(), uri.clone(), w, files, open_file),
    };
    let context = StoredValue::new(context);
    let pane = move || {
        let context = context.get_value();
        match context.resolve.clone().filter(|_| open.get()) {
            Some(resolve) => view! {
                <ResolvePane
                    namespace=ns.clone()
                    uri=uri.clone()
                    revision=context.revision
                    resolve=resolve
                    marks=marks.differing
                    back_href=carrying(routes::package_page_href(&ns))
                    w=w
                    commands=resolving
                />
            }
            .into_any(),
            None => view! {
                <CurrentRevisionPane
                    data=context
                    namespace=namespace.clone()
                    fetch=context_pane::fetch_revision_history
                    open_catalog=open_catalog
                    w=w
                    commands=keeping::KeepingCommands::app()
                    remove=old_revisions::remove_revisions
                />
            }
            .into_any(),
        }
    };
    view! {
        <div class=style::page>
            {page_header}
            <div class=style::shell>
                {pane}
                <FilePane
                    listing=listing
                    grouping=files.grouping
                    collapsed=files.collapsed
                    search=files.search
                    facet=files.facet
                    on_open=open_file
                    on_retry=files.retry
                    picking=picking
                    menu=menu
                    differing=row_marks
                />
            </div>
        </div>
    }
    .into_any()
}

/// The header, with the dry run's summary after its label, and the file
/// list's marks: resolve mode's differing set, or the files the check says
/// conflict.
///
/// Only a header that is behind or in conflict asks. A `Blocked` verdict
/// from a run that finished after this read resolves it to the conflict
/// state; the list marks that state's files, whether the read or the check
/// gave it, as resolve mode marks the files that differ, and resolve mode's
/// own marks win while it is open. The header is drawn again when the check
/// moves the state, as a re-read draws it; the summary follows the check by
/// itself.
fn incoming_header(
    header: commands::PackageHeaderData,
    holds: String,
    w: Wiring,
    open: Memo<bool>,
    marks: FileMarks,
    incoming: IncomingCheck,
) -> (AnyView, Memo<Option<Arc<BTreeSet<String>>>>) {
    let namespace = header.namespace.to_string();
    let check =
        checks_incoming(&header.state).then(|| incoming.check_for(namespace.clone(), holds));
    let fresh = Signal::derive(move || check.is_none_or(|c| c.with(|(_, fresh)| *fresh)));
    let check: Signal<Option<commands::PullCheck>> =
        Signal::derive(move || check.map(|c| c.with(|(check, _)| check.clone())));
    let read_state = header.state.clone();
    let state = Memo::new(move |_| {
        self::incoming::header_state(&read_state, check.get().as_ref(), fresh.get())
    });
    let conflicts = Memo::new(move |_| state.with(self::incoming::conflicting));
    let row_marks = Memo::new(move |_| marks.differing.get().or_else(|| conflicts.get()));
    let newer = self::incoming::Catalog::new(
        header.uri.as_ref(),
        browser_opener(
            namespace,
            w.outcome,
            "Could not open this file in the catalog.",
        ),
    );
    let whole = marks.scope == commands::KeepingScope::EntirePackage;
    let header = StoredValue::new(header);
    let page_header = move || {
        let mut data = header.get_value();
        data.state = state.get();
        let summary = view! {
            <IncomingSummary
                check=check
                whole=whole
                catalog=newer.clone()
                on_retry=Callback::new(move |()| incoming.retry())
                card=incoming.card
            />
        }
        .into_any();
        view! { <PageHeader data=data w=w resolving=open summary=summary /> }
    };
    (view! { {page_header} }.into_any(), row_marks)
}

/// The file pane's `[Download]`: `package_download_backlog` over the ticked
/// paths, under the page's one-command lock.
///
/// A success clears the ticks it sent, and the re-read turns the rows `Downloaded`. A
/// file the remote no longer holds stays `Not downloaded`, and the band says
/// which ([`download_outcome`]). A failure keeps the ticks, so a retry is one
/// press. While it runs every box is disabled, as every other command is, so
/// the ticks it clears are the ones it sent.
fn file_downloader(namespace: String, w: Wiring, files: Files) -> Callback<Vec<String>> {
    Callback::new(move |paths: Vec<String>| {
        let (ns, outcome, downloading, ticked) = (
            namespace.clone(),
            w.outcome,
            files.downloading,
            files.ticked,
        );
        let task = async move {
            let asked = paths.len();
            downloading.set(true);
            let answer = commands::package_download_backlog(ns.clone(), paths.clone()).await;
            downloading.try_set(false);
            answer.map(|skipped| {
                // Safe on any package: every box is disabled while the page's
                // lock is held, so nothing was ticked since this was sent, and
                // moving on only took ticks away.
                ticked.try_update(|t| file_pane::selection::tick_all(t, &paths, false));
                if let Some(said) = download_outcome(ns, asked, &skipped) {
                    outcome.try_set(Some(said));
                }
                String::new()
            })
        };
        run(
            w.busy,
            w.outcome,
            namespace.clone(),
            "Could not download the files.",
            Reread::OnSuccess(w.reload),
            task,
        );
    })
}

/// Runs what a row's `[⋯]` chose.
///
/// Opening and copying write nothing, so they take no lock, as `file_opener`
/// does not; the two `.quiltignore` items open popups whose state is the
/// page's ([`row_popups`]), and the menu holds them back while the lock is held.
fn row_runner(
    namespace: String,
    uri: Option<quilt_uri::S3PackageUri>,
    w: Wiring,
    files: Files,
    open_file: Callback<String>,
) -> Callback<RowCommand> {
    Callback::new(move |command: RowCommand| {
        let (namespace, uri, outcome) = (namespace.clone(), uri.clone(), w.outcome);
        match command {
            RowCommand::Open(path) => open_file.run(path),
            RowCommand::OpenInCatalog(path) => {
                let Some(url) = uri
                    .as_ref()
                    .and_then(|u| crate::util::entry_catalog_url(u, &path))
                else {
                    return;
                };
                leptos::task::spawn_local(async move {
                    if let Err(detail) = commands::open_in_web_browser(url).await {
                        outcome.try_set(Some(Outcome {
                            namespace,
                            variant: BannerVariant::Critical,
                            lead: "Could not open this file in the catalog.".to_string(),
                            detail: Some(detail),
                        }));
                    }
                });
            }
            RowCommand::CopyUri(_) | RowCommand::CopyPath(_) => {
                leptos::task::spawn_local(async move {
                    let copied = match clipped(&command, &namespace, uri.as_ref(), local_path).await
                    {
                        Ok(None) => return,
                        Ok(Some((text, event))) => {
                            commands::copy_to_clipboard(text, event).await.map(|_| ())
                        }
                        Err(detail) => Err(detail),
                    };
                    outcome.try_set(Some(copy_outcome(namespace, &command, copied)));
                });
            }
            RowCommand::Ignore(path) => files.ignoring.set(Some(IgnorePopupData {
                namespace,
                suggested_pattern: path.clone(),
                path,
                uri,
            })),
            RowCommand::StopIgnoring { pattern, .. } => {
                files.unignoring.set(Some(UnignorePopupData {
                    namespace,
                    pattern,
                    uri,
                }));
            }
        }
    })
}

/// Where a package's file is on disk: `(namespace, path)` to the full path.
/// A seam, like [`PageRead`], so a test can answer without a Tauri host.
pub(crate) type LocalPath =
    fn(String, String) -> Pin<Box<dyn Future<Output = Result<String, String>>>>;

fn local_path(
    namespace: String,
    path: String,
) -> Pin<Box<dyn Future<Output = Result<String, String>>>> {
    Box::pin(commands::package_file_path(namespace, path))
}

/// What a copy puts on the clipboard, and the address its event names: v1's
/// `util::file_uri` for *Copy URI*, and for *Copy path* the file's full path on
/// disk, which a `New` file has though no revision holds it. `Ok(None)` for a
/// command that copies nothing, or an address with no remote to build it from;
/// an error when the path cannot be found.
///
/// The header's remote names the revision this copy holds; the address is
/// built at `Latest` instead, as the recent-files list builds its own
/// (`util::package_uri`), so a copied address points at the package rather
/// than at the revision the copier happens to have.
async fn clipped(
    command: &RowCommand,
    namespace: &str,
    uri: Option<&quilt_uri::S3PackageUri>,
    local: LocalPath,
) -> Result<Option<(String, Option<quilt_uri::S3PackageUri>)>, String> {
    let file = |path: &str| {
        uri.map(|u| {
            let latest = quilt_uri::S3PackageUri {
                revision: quilt_uri::RevisionPointer::Tag(quilt_uri::Tag::Latest),
                ..u.clone()
            };
            crate::util::file_uri(&latest, path)
        })
    };
    match command {
        RowCommand::CopyUri(path) => Ok(file(path).map(|f| (f.display(), Some(f)))),
        RowCommand::CopyPath(path) => {
            let on_disk = local(namespace.to_string(), path.clone()).await?;
            Ok(Some((on_disk, file(path))))
        }
        _ => Ok(None),
    }
}

/// What the band says after a copy. The clipboard shows nothing, so a copy
/// that worked says what it copied, which is the band's success case: no other
/// surface says it.
fn copy_outcome(namespace: String, command: &RowCommand, copied: Result<(), String>) -> Outcome {
    let (variant, lead, detail) = match (copied, command) {
        (Err(detail), _) => (
            BannerVariant::Critical,
            "Could not copy.".to_string(),
            Some(detail),
        ),
        (Ok(()), RowCommand::CopyPath(path)) => (
            BannerVariant::Success,
            format!("Copied the path of {path}"),
            None,
        ),
        (Ok(()), command) => (
            BannerVariant::Success,
            format!("Copied the address of {}", command.path()),
            None,
        ),
    };
    Outcome {
        namespace,
        variant,
        lead,
        detail,
    }
}

/// The popups a row's `[⋯]` opens, drawn outside the body so a re-read keeps
/// them.
///
/// They are v1's, which report through a `Notification`; this page reports on
/// its band instead, so a failure becomes the band's, keyed to the package the
/// popup was opened for ([`report_popup`]), and a success says nothing there:
/// the re-read is the report. Adding a pattern re-reads the page itself.
/// *Stop ignoring* opens `.quiltignore` in the reader's editor; the page
/// re-reads once it opened, and the watcher, which never screens
/// `.quiltignore` out, re-reads again when the edit is saved.
///
/// The page survives a switch of package, so an answer can land after the
/// reader moved on. Both popups hold the page's lock while they wait, so no
/// other `.quiltignore` popup opens before the answer lands, and an answer
/// closes only its own popup ([`close_own`]).
fn row_popups(files: Files, w: Wiring) -> impl IntoView {
    let said = RwSignal::new(None::<Notification>);
    let unignore_said = RwSignal::new(None::<Notification>);
    // The package each popup was opened for, so its answer is that package's
    // news even if it lands after the reader moved on (the band drops it
    // then). Sound because both popups hold the page's lock while they wait,
    // so no other `.quiltignore` popup can open before the answer lands.
    let ignore_for = StoredValue::new(String::new());
    let unignore_for = StoredValue::new(String::new());
    report_popup(
        said,
        ignore_for,
        "Could not ignore this file.",
        None,
        w.outcome,
    );
    report_popup(
        unignore_said,
        unignore_for,
        "Could not open .quiltignore.",
        Some(w.reload),
        w.outcome,
    );
    view! {
        {move || {
            files.ignoring.get().map(|data| {
                ignore_for.set_value(data.namespace.clone());
                let (ns, path) = (data.namespace.clone(), data.path.clone());
                view! {
                    <IgnorePopup
                        data=data
                        notification=said
                        refetch=w.reload
                        on_close=move || {
                            files
                                .ignoring
                                .update(|open| {
                                    close_own(open, |d| d.namespace == ns && d.path == path);
                                });
                        }
                        lock=w.busy
                    />
                }
            })
        }}
        {move || {
            files.unignoring.get().map(|data| {
                unignore_for.set_value(data.namespace.clone());
                let (ns, pattern) = (data.namespace.clone(), data.pattern.clone());
                view! {
                    <UnignorePopup
                        data=data
                        notification=unignore_said
                        on_close=move || {
                            files
                                .unignoring
                                .update(|open| {
                                    close_own(open, |d| d.namespace == ns && d.pattern == pattern);
                                });
                        }
                        lock=w.busy
                    />
                }
            })
        }}
    }
}

/// Turn a v1 popup's `Notification` into this page's band: a failure is news
/// about `opened_for`, the package the popup was opened for, whichever package
/// is on screen when it lands (the band drops it if that is another one). A
/// success says nothing; `then` is what it does besides, since the ignore popup
/// re-reads the page itself and the unignore popup does not.
fn report_popup(
    notice: RwSignal<Option<Notification>>,
    opened_for: StoredValue<String>,
    lead: &'static str,
    then: Option<Trigger>,
    outcome: RwSignal<Option<Outcome>>,
) {
    Effect::new(move |_| {
        let Some(n) = notice.get() else { return };
        notice.set(None);
        if let Notification::Error(detail) = n {
            outcome.set(Some(Outcome {
                namespace: opened_for.get_value(),
                variant: BannerVariant::Critical,
                lead: lead.to_string(),
                detail: Some(detail),
            }));
        } else if let Some(reload) = then {
            reload.notify();
        }
    });
}

/// Close the popup `open` holds only if it is the one `mine` names: an answer
/// that lands after the reader moved on must not close a popup opened since.
fn close_own<T>(open: &mut Option<T>, mine: impl Fn(&T) -> bool) {
    if open.as_ref().is_some_and(mine) {
        *open = None;
    }
}

/// What the band says after a download: nothing when every file came down,
/// and a warning naming the files the remote no longer holds when some did not.
fn download_outcome(namespace: String, asked: usize, skipped: &[String]) -> Option<Outcome> {
    file_pane::selection::unavailable(asked, skipped).map(|(lead, paths)| Outcome {
        namespace,
        variant: BannerVariant::Warning,
        lead,
        detail: Some(paths),
    })
}

/// Opens a revision's catalog page, reporting a failure on the keyed band.
///
/// Not `run`: opening a browser is not a working-tree command, so it neither
/// takes `busy` nor clears the band. A link cannot be disabled, and one that
/// silently did nothing while busy would be worse than one that opens.
fn catalog_opener(namespace: String, outcome: RwSignal<Option<Outcome>>) -> Callback<String> {
    browser_opener(
        namespace,
        outcome,
        "Could not open this revision in the catalog.",
    )
}

/// Opens an address in the browser, reporting a failure on the keyed band
/// with `lead`. [`catalog_opener`]'s, for any catalog link.
fn browser_opener(
    namespace: String,
    outcome: RwSignal<Option<Outcome>>,
    lead: &'static str,
) -> Callback<String> {
    Callback::new(move |url: String| {
        let namespace = namespace.clone();
        leptos::task::spawn_local(async move {
            if let Err(detail) = commands::open_in_web_browser(url).await {
                outcome.try_set(Some(Outcome {
                    namespace,
                    variant: BannerVariant::Critical,
                    lead: lead.to_string(),
                    detail: Some(detail),
                }));
            }
        });
    })
}

/// Opens a downloaded file in its default application, reporting a failure on
/// the keyed band. Not `run`, for `catalog_opener`'s reason: opening a file
/// writes nothing, so it neither takes `busy` nor clears the band.
fn file_opener(
    namespace: String,
    uri: Option<quilt_uri::S3PackageUri>,
    outcome: RwSignal<Option<Outcome>>,
) -> Callback<String> {
    Callback::new(move |path: String| {
        let namespace = namespace.clone();
        let uri = uri.clone();
        leptos::task::spawn_local(async move {
            if let Err(detail) =
                commands::open_in_default_application(namespace.clone(), path, uri).await
            {
                outcome.try_set(Some(Outcome {
                    namespace,
                    variant: BannerVariant::Critical,
                    lead: "Could not open this file.".to_string(),
                    detail: Some(detail),
                }));
            }
        });
    })
}

/// The whole package page before its read answers: the appbar, the banner's
/// row, and [`package_skeleton`].
///
/// What `/installed-package` draws while it reads which package page the reader
/// has switched on, when the root marker says it will be this one — see
/// `main.rs`'s `design_loading`. It is [`PackageScreen`]'s own first paint, so
/// the handover moves nothing: the appbar, the header and both panes land on
/// the pixels they already hold.
///
/// The banner is passed empty, not left out. The page always fills
/// `PageLayout`'s slot — its bands are drawn from the answer, inside the slot —
/// and the slot's row takes its padding whether a band is in it or not, so a
/// skeleton without one would sit that much higher than the page it hands to.
///
/// `actions` is the appbar's, passed in because the app's Settings button
/// navigates and the gallery, which draws this too, has no router.
#[component]
pub fn PackagePageSkeleton(actions: AnyView) -> impl IntoView {
    view! {
        <PageLayout heading="Package" banner=().into_any() actions=actions>
            {package_skeleton()}
        </PageLayout>
    }
}

/// The page's body while its read is out: the header, then both panes, in the
/// shell the answered page uses.
fn package_skeleton() -> AnyView {
    view! {
        <div class=style::page>
            <PageHeaderSkeleton />
            <div class=style::shell>
                <CurrentRevisionPaneSkeleton />
                <FilePaneSkeleton />
            </div>
        </div>
    }
    .into_any()
}

fn package_failure(namespace: String, reload: Trigger) -> AnyView {
    view! {
        <div class=style::page>
            <h2 class=style::identity>{namespace}</h2>
            <LoadFailure
                words="Could not load this package."
                on_retry=Callback::new(move |()| reload.notify())
            />
        </div>
    }
    .into_any()
}

/// What `/installed-package` renders when *New design preview* is on.
///
/// `main.rs`'s `PackagePage` decides, from the same answer `/` renders the main
/// page by, so a reader who has the new main page gets this one with it and
/// every link to a package lands here rather than on v1's.
#[component]
pub fn InstalledPackageV2() -> impl IntoView {
    view! {
        <PackageScreen
            read=read_page
            pull=read_pull_outcome
            resolving=ResolveCommands::app()
            revision_message=mismatch_band::app_revision_message
        />
    }
}

/// The page's one read: the header, the pane and the pause, for one namespace.
pub(crate) type PageRead = super::v2_page::PageRead<commands::PackagePageData>;

fn read_page(
    namespace: String,
) -> Pin<Box<dyn Future<Output = Result<commands::PackagePageData, String>>>> {
    Box::pin(commands::get_package_page_data(namespace))
}

/// The page over whichever read, resolve commands and revision lookup it is
/// given, so a routed test can feed it a payload without a Tauri host.
#[component]
fn PackageScreen(
    read: PageRead,
    pull: PullRead,
    resolving: ResolveCommands,
    revision_message: mismatch_band::RevisionMessage,
) -> impl IntoView {
    let query = use_query_map();
    // The address is the only input, and changes without a remount: one route serves every package.
    // Memos, so only `namespace` re-runs the read: the mode opens with no loading state.
    let ns = Memo::new(move |_| query.read().get("namespace").unwrap_or_default());
    let asked = Memo::new(move |_| query.read().get("resolve").as_deref() == Some("1"));
    // What a deep link found: another revision, or no remote to check it on.
    // Given to every address the page builds for this package; the other
    // revision is looked up here rather than by the band, which every re-read
    // rebuilds. `link_outcome`, not `outcome`, which is the last command's.
    let link_outcome = Memo::new(move |_| routes::DeepLinkOutcome::from_query(&query.read()));
    provide_context(Carried(link_outcome));
    let mismatch = Memo::new(move |_| {
        link_outcome
            .get()
            .as_ref()
            .and_then(|o| o.mismatch().cloned())
    });
    let local_only =
        Memo::new(move |_| link_outcome.get() == Some(routes::DeepLinkOutcome::LocalOnly));
    let requested = mismatch_band::requested_message(mismatch, ns, revision_message);

    // What the reader has already read and closed. Keyed on the message, so a
    // different pause is news again — see `pause_banner`.
    let dismissed: RwSignal<Option<String>> = RwSignal::new(None);
    // Here and not in the header, which every re-read rebuilds — see `Wiring`.
    let page = PageHandle::new();
    let w = Wiring::over(page);
    w.follow(ns.into());
    let reload = page.reload;
    // One read for the whole page. Re-runs when the namespace changes, and
    // whenever `reload` fires — the watcher reporting news about this package,
    // or the failure arm's way out.
    let answer = page.read(ns, read);

    // Not remembered: another package, or another visit, starts at the
    // default with every folder open, no search and the `All` facet. `ns` is a memo on
    // the namespace alone, so the rest of the address — Resolve's `resolve=1` — is not
    // a new package.
    let grouping = RwSignal::new(Grouping::BaseFolder.label().to_string());
    let collapsed = RwSignal::new(BTreeSet::new());
    let search = RwSignal::new(String::new());
    let facet = RwSignal::new(Facet::All.key().to_string());
    // Ticks name paths in one package, so another package starts with none.
    let ticked = RwSignal::new(BTreeSet::new());
    // A popup names one package's file, so another package closes it.
    let (ignoring, unignoring) = (RwSignal::new(None), RwSignal::new(None));
    Effect::new(move |_| {
        ns.track();
        grouping.set(Grouping::BaseFolder.label().to_string());
        collapsed.set(BTreeSet::new());
        search.set(String::new());
        facet.set(Facet::All.key().to_string());
        ticked.set(BTreeSet::new());
        ignoring.set(None);
        unignoring.set(None);
    });
    // The list comes with the page read, so trying again is reading the page.
    let files = Files {
        grouping,
        collapsed,
        search,
        facet,
        retry: Callback::new(move |()| reload.notify()),
        ticked,
        downloading: RwSignal::new(false),
        ignoring,
        unignoring,
    };

    // The newer revision's dry run, after each read that shows the package
    // behind or in conflict: after the page is drawn, so it never holds the
    // page back. A settled read alone decides, as below; `in_flight` rather
    // than the answer, which notifies nothing when a re-read finds the same
    // data, while a local edit or a moved newer revision can still change what
    // the check says.
    let incoming = IncomingCheck::new(pull);
    Effect::new(move |_| {
        if page.in_flight.get() {
            return;
        }
        let Some(answered) = answer.get() else {
            return;
        };
        let asks = answered
            .ok()
            .filter(|d| checks_incoming(&d.header.state))
            .map(|d| (ns.get_untracked(), d.context.revision.hash));
        incoming.after_read(asks);
    });
    // A pin names one package's files, so another package starts closed.
    Effect::new(move |_| {
        ns.track();
        incoming.card.close();
    });

    // A `resolve=1` the package cannot honour is replaced by the plain address,
    // so *Back* never re-enters a mode that does not exist. The answer stays up
    // while a re-read is out, so only a settled read decides.
    Effect::new(move |_| {
        if page.in_flight.get() {
            return;
        }
        let Some(Ok(answered)) = answer.get() else {
            return;
        };
        if let Some(to) = normalized_address(
            asked.get(),
            &ns.get(),
            &answered,
            link_outcome.get().as_ref(),
        ) {
            w.replace_to.set(Some(Replace {
                namespace: ns.get(),
                to,
            }));
        }
    });

    // The outcome band carries only what no other surface says. A navigation
    // reports by arriving, a dialog holds its own refusal, and a pull posts its
    // report to the notification stack. What is left is a non-dialog command's
    // failure, an undo that succeeded (the state label can read the same before
    // and after, so the re-read is not a report), and a remote set whose
    // workflow could not be resolved.
    //
    // `heading` is not reactive and one route serves every package, so it names
    // the page rather than the package; the package's own name is on screen.
    view! {
        <PackageEventListener reload=reload />
        {row_popups(files, w)}
        <V2Page
            heading="Package"
            page=page
            answer=answer
            showing=ns
            skeleton=package_skeleton
            bands=move |d: commands::PackagePageData| {
                view! {
                    {pause_banner(d.sync_paused.clone(), dismissed)}
                    {local_only_band::local_only_band(local_only, &d.header.state)}
                    {mismatch_band::mismatch_band(
                        mismatch,
                        requested.into(),
                        d.context.revision.clone(),
                        &d.header.state,
                    )}
                }
                .into_any()
            }
            body=move |d| package_body(d, w, asked.into(), resolving, files, incoming)
            // The page keeps its frame and states the failure in place. A read
            // that failed for a reason the header could have worded — no
            // session, a refused role — never reaches here: the command resolves
            // those to a state, and the header draws them.
            failure=move || package_failure(ns.get(), reload)
        />
    }
}

/// The band that says autosync has stopped, and why.
///
/// # Beside the state, not instead of it
///
/// The header says what the package needs; this says why the worker stopped.
/// Different sentences, and a package can need both at once — a newer revision
/// upstream and a workflow that rejected the last one. Folding the second into
/// the header's one label would hide the first.
///
/// Only the residue reaches here. Every other pause resolves into a state the
/// header words — a conflict names its files, a denial names the refusal — and
/// the backend sends `None` for those, so the two surfaces cannot say the same
/// thing twice.
///
/// # The words are the page's; the detail is the engine's
///
/// The lead sentence is written here, because the vocabulary is UI-owned. The
/// message is the engine's own refusal text — a workflow's complaint, a hash
/// mismatch — and nothing else knows it, so it renders as the detail after it.
///
/// # Critical, because a stopped sync is a failure
///
/// `DESIGN.md` requires a band and a chip to agree — *"a warning on the page and
/// a warning on a row cannot disagree about what amber means"* — and the kit
/// tones `PackageState::Paused` Danger. Amber here said the opposite of red
/// there about one fact.
///
/// The cost is taken deliberately rather than worked around. `BannerVariant`
/// welds colour to announcement, so `Critical` is also `role="alert"`, which
/// interrupts a reader on arrival for something that was already true before
/// they opened the page. That is the wrong shape of announcement and the right
/// colour, and the colour wins: autosync having stopped is a failure, and a band
/// that says so quietly in amber understates it.
///
/// # Dismissal is keyed on the message
///
/// The bar has no timer and the caller owns its dismissal. A pause is a standing
/// fact, so dismissing hides a thing that is still true — which is the reader's
/// call to make about a message they have read. But a *different* pause is news
/// again, so what is remembered is the message dismissed rather than a flag.
fn pause_banner(message: Option<String>, dismissed: RwSignal<Option<String>>) -> AnyView {
    // `StoredValue` so the derived closure is `Copy` and can be handed to both
    // the `when` and the body without cloning the message at each use.
    let message = StoredValue::new(message);
    let showing = move || {
        message
            .get_value()
            .filter(|m| dismissed.get().as_ref() != Some(m))
    };

    view! {
        <Show when=move || showing().is_some() fallback=|| ()>
            {
                let message = showing().unwrap_or_default();
                let remembered = message.clone();
                view! {
                    <Banner
                        variant=BannerVariant::Critical
                        on_dismiss=move |_| dismissed.set(Some(remembered.clone()))
                    >
                        "Autosync has stopped for this package. "
                        {message}
                    </Banner>
                }
            }
        </Show>
    }
    .into_any()
}

/// Ask the backend again when the watcher reports news about **this** package.
///
/// Renders nothing; it exists for the two subscriptions, which are dropped with
/// it. Its own component so they are registered once rather than rebuilt with a
/// payload — `main_page`'s `PackageStatusListener` for the same reason.
///
/// # Two streams, because one of them cannot see a pause
///
/// `package-status-changed` carries a fingerprint of the observation — the
/// upstream state and the changed paths — and [`StatusWatch`] drops an event
/// that repeats the last one. A pause moves neither: a workflow rejection or a
/// refused role stops syncing over a tree that has not changed, so the status
/// stream reports the same observation and the fingerprint rule correctly calls
/// it old news. `autosync-paused` is the only thing that says a pause happened,
/// so the header would never learn of one without it.
///
/// # What is still missed, and why it is not fixed here
///
/// Nothing announces a pause **clearing**. `Watcher::clear_paused` drops the
/// entry and notifies the tray, and emits no event, so the page learns a pause
/// is over only from the next status event whose fingerprint differs. For the
/// ordinary case that is enough — a pull that resolves a conflict rewrites the
/// tree, and the filesystem watcher reports it — but re-enabling autosync clears
/// every pause while moving nothing, and this page would keep the stale answer
/// until something else moved. That is an engine gap, not a page one.
///
/// # Both filters are on the namespace
///
/// One route serves every package and the watcher reports all of them, so
/// without the filter this page would refetch on every other package's news.
/// Read untracked: the listener reads the address at the moment an event
/// arrives, and must not subscribe to it.
#[component]
fn PackageEventListener(reload: Trigger) -> impl IntoView {
    let query = use_query_map();
    let watch = StatusWatch::new(reload);

    let is_ours = move |namespace: &quilt_uri::Namespace| {
        query.read_untracked().get("namespace").as_deref() == Some(namespace.to_string().as_str())
    };

    let status = crate::tauri::listen::<commands::PackageStatusEvent>(
        commands::PACKAGE_STATUS_EVENT,
        move |event| {
            if is_ours(&event.namespace) {
                watch.observe(&event);
            }
        },
    );
    let paused = crate::tauri::listen::<commands::PausedEvent>(
        commands::AUTOSYNC_PAUSED_EVENT,
        move |event| {
            if is_ours(&event.namespace) {
                // No fingerprint to compare: a pause is news the status stream
                // cannot report. The refetch re-reads the watcher's map, which
                // is authoritative, so a repeat costs one read and says the
                // same thing.
                watch.nudge();
            }
        },
    );
    on_cleanup(move || {
        drop(status);
        drop(paused);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{button_saying, element_saying, mount, sleep_ms};
    use leptos_router::components::{Route, Router, Routes};
    use leptos_router::path;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::*;

    /// Put the browser on an address before the router reads one. Same origin,
    /// so the history write is allowed; the runner's own page is whatever it is.
    fn go_to(address: &str) {
        web_sys::window()
            .unwrap()
            .history()
            .unwrap()
            .replace_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some(address))
            .unwrap();
    }

    /// A settled page for `team/dataset` at its latest revision. The child
    /// modules' tests start from it and change only what they are about.
    pub(super) fn page_data() -> commands::PackagePageData {
        commands::PackagePageData {
            header: commands::PackageHeaderData {
                namespace: "team/dataset".try_into().unwrap(),
                uri: None,
                state: crate::kit::PackageState::Latest,
                remote_locked: false,
                has_local_commit: false,
                commit_has_parent: false,
                role_switch: None,
            },
            context: commands::PackageContextData {
                revision: commands::CurrentRevisionData {
                    hash: "0123456789abcdef".to_string(),
                    message: Some("Initial upload".to_string()),
                    obtained_at: 1_758_500_000_000.0,
                },
                bucket: Some("quilt-lab-plates".to_string()),
                revision_count: 1,
                keeping: commands::KeepingData {
                    scope: commands::KeepingScope::IndividualFiles,
                    total: 1,
                    remote_only: Vec::new(),
                    deleted_here: 0,
                },
                size: Some(commands::PackageSize {
                    total: 7,
                    downloaded: 7,
                }),
                resolve: None,
            },
            sync_paused: None,
            files: commands::FilesData::Listed(commands::EntryList {
                entries: vec![commands::EntryData {
                    filename: "a.csv".to_string(),
                    size: 7,
                    status: "pristine".to_string(),
                    junky_pattern: None,
                    ignored_by: None,
                    namespace: "team/dataset".try_into().unwrap(),
                }],
                counts: commands::EntryCounts {
                    all: 1,
                    ..commands::EntryCounts::default()
                },
                total: 1,
                truncated: false,
            }),
        }
    }

    /// A popup's failure is news about the package it was opened for, even
    /// when it lands after the reader moved to another: the band, keyed by
    /// package, then shows it on neither the new package nor under its name.
    #[wasm_bindgen_test]
    async fn a_popup_s_failure_is_the_package_it_was_opened_for() {
        // Scoped, not `set`: the wasm tests share one thread, and an owner left
        // current would outlive this test and own the next one's reactive work.
        let owner = Owner::new();
        let (notice, outcome) = owner.with(|| {
            let notice = RwSignal::new(None);
            let outcome = RwSignal::new(None);
            let opened_for = StoredValue::new("team/a".to_string());
            report_popup(
                notice,
                opened_for,
                "Could not ignore this file.",
                None,
                outcome,
            );
            (notice, outcome)
        });

        // The reader is on team/b by now; nothing the popup recorded moves.
        notice.set(Some(Notification::Error("denied".to_string())));
        leptos::task::tick().await;
        assert_eq!(
            outcome.get_untracked(),
            Some(said(
                "team/a",
                BannerVariant::Critical,
                "Could not ignore this file.",
                Some("denied"),
            ))
        );
        assert!(notice.get_untracked().is_none(), "the notice is consumed");

        let el = mount(move || outcome_band(outcome, Signal::stored("team/b".to_string())));
        assert!(
            !el.inner_html().contains("Could not ignore"),
            "team/b's band does not carry team/a's failure",
        );
    }

    /// A popup's answer closes that popup only: one that lands after the
    /// reader moved on, and opened another, leaves the other one open.
    #[test]
    fn a_popup_s_answer_closes_only_that_popup() {
        let theirs = |ns: &str| UnignorePopupData {
            namespace: ns.to_string(),
            pattern: "*.log".to_string(),
            uri: None,
        };
        let mut open = Some(theirs("team/b"));
        close_own(&mut open, |d| d.namespace == "team/a");
        assert_eq!(open.map(|d| d.namespace), Some("team/b".to_string()));

        let mut open = Some(theirs("team/a"));
        close_own(&mut open, |d| d.namespace == "team/a");
        assert!(open.is_none());
    }

    /// The clipboard shows nothing, so a copy says what it copied, in the
    /// recent-files list's words for an address, and a refusal says so.
    #[test]
    fn a_copy_says_what_it_copied() {
        let uri = RowCommand::CopyUri("raw/a.csv".to_string());
        let path = RowCommand::CopyPath("raw/a.csv".to_string());
        assert_eq!(
            copy_outcome("team/a".to_string(), &uri, Ok(())),
            said(
                "team/a",
                BannerVariant::Success,
                "Copied the address of raw/a.csv",
                None
            )
        );
        assert_eq!(
            copy_outcome("team/a".to_string(), &path, Ok(())),
            said(
                "team/a",
                BannerVariant::Success,
                "Copied the path of raw/a.csv",
                None
            )
        );
        assert_eq!(
            copy_outcome("team/a".to_string(), &path, Err("denied".to_string())),
            said(
                "team/a",
                BannerVariant::Critical,
                "Could not copy.",
                Some("denied")
            )
        );
    }

    /// Where this test's package is on disk, as the backend would answer.
    fn at_home(
        namespace: String,
        path: String,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, String>>>> {
        Box::pin(async move { Ok(format!("/Users/me/QuiltSync/{namespace}/{path}")) })
    }

    fn not_there(
        _: String,
        path: String,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, String>>>> {
        Box::pin(async move { Err(format!("{path} is not there")) })
    }

    /// Copy URI puts `util::file_uri`'s address on the clipboard, at `Latest`;
    /// Copy path puts the file's full path on disk, which needs no remote.
    #[wasm_bindgen_test]
    async fn copy_uri_copies_the_address_and_copy_path_the_full_path_on_disk() {
        let package = crate::util::package_uri(
            "team-bucket",
            &"user/plate-07".try_into().unwrap(),
            Some("quilt.test"),
        );
        let address =
            "quilt+s3://team-bucket#package=user/plate-07&path=runs/one.csv&catalog=quilt.test";
        let copy_uri = RowCommand::CopyUri("runs/one.csv".to_string());
        let (text, event) = clipped(&copy_uri, "user/plate-07", Some(&package), at_home)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(text, address);
        assert_eq!(event.map(|u| u.display()), Some(text));

        // A New row: the full path on disk, as opening the file resolves it.
        let copy_path = RowCommand::CopyPath("runs/new.csv".to_string());
        let (text, _) = clipped(&copy_path, "user/plate-07", None, at_home)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(text, "/Users/me/QuiltSync/user/plate-07/runs/new.csv");
        assert_eq!(
            clipped(&copy_path, "user/plate-07", None, not_there).await,
            Err("runs/new.csv is not there".to_string()),
            "a path that cannot be found is a failed copy, not an empty one",
        );

        assert_eq!(
            clipped(
                &RowCommand::CopyUri("a.csv".to_string()),
                "user/plate-07",
                None,
                at_home
            )
            .await,
            Ok(None),
            "no remote, no address",
        );
        assert_eq!(
            clipped(
                &RowCommand::Ignore("a.csv".to_string()),
                "user/plate-07",
                Some(&package),
                at_home
            )
            .await,
            Ok(None),
        );

        // The header's remote names the revision this copy holds; the address
        // names the package, as the recent-files list's does.
        let pinned = quilt_uri::S3PackageUri {
            revision: quilt_uri::RevisionPointer::Hash("abc123".to_string()),
            ..package.clone()
        };
        let (text, _) = clipped(&copy_uri, "user/plate-07", Some(&pinned), at_home)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(text, address);
    }

    /// A download the remote could not finish is a warning that names the
    /// files left behind; one it finished says nothing, as success does here.
    #[test]
    fn a_download_that_left_files_behind_warns_and_names_them() {
        assert_eq!(download_outcome("team/a".to_string(), 3, &[]), None);
        assert_eq!(
            download_outcome("team/a".to_string(), 3, &["raw/b.csv".to_string()]),
            Some(said(
                "team/a",
                BannerVariant::Warning,
                "Downloaded 2 of 3. 1 file is no longer on the remote at this revision.",
                Some("raw/b.csv"),
            ))
        );
    }

    /// The pane's page-owned state at rest, which is all these tests need of it.
    pub(super) fn idle_files() -> Files {
        Files {
            grouping: RwSignal::new(Grouping::BaseFolder.label().to_string()),
            collapsed: RwSignal::new(BTreeSet::new()),
            search: RwSignal::new(String::new()),
            facet: RwSignal::new(Facet::All.key().to_string()),
            retry: Callback::new(|()| ()),
            ticked: RwSignal::new(BTreeSet::new()),
            downloading: RwSignal::new(false),
            ignoring: RwSignal::new(None),
            unignoring: RwSignal::new(None),
        }
    }

    /// A dry run that never answers, for the tests not about it.
    pub(super) fn never_pulls(
        _: String,
    ) -> Pin<Box<dyn Future<Output = Result<commands::PullPreview, String>>>> {
        Box::pin(std::future::pending())
    }

    /// The page's dry run at rest.
    pub(super) fn idle_incoming() -> IncomingCheck {
        IncomingCheck::new(never_pulls)
    }

    fn body(data: commands::PackagePageData) -> web_sys::Element {
        mount(move || {
            let w = Wiring::new();
            let resolving = ResolveCommands::app();
            view! {
                <Router>{package_body(data, w, Signal::stored(false), resolving, idle_files(), idle_incoming())}</Router>
            }
        })
    }

    /// The list is the answer's, drawn with its header: package B's body can
    /// only ever hold package B's files.
    #[wasm_bindgen_test]
    fn the_body_draws_the_file_list_its_own_answer_carries() {
        let el = body(page_data());
        assert!(
            el.query_selector("section[aria-label='Files'] [title='a.csv']")
                .unwrap()
                .is_some(),
            "markup was {}",
            el.inner_html()
        );
    }

    /// No status, no list: the pane states why and draws no row.
    #[wasm_bindgen_test]
    fn an_answer_without_a_list_draws_the_pane_s_failure() {
        let mut data = page_data();
        data.files = commands::FilesData::Unlisted {
            reason: "Access denied".to_string(),
        };
        let el = body(data);
        element_saying(&el, "Could not list this package's files.");
        assert!(
            el.query_selector("section[aria-label='Files'] [title]")
                .unwrap()
                .is_none(),
            "markup was {}",
            el.inner_html()
        );
    }

    fn said(namespace: &str, variant: BannerVariant, lead: &str, detail: Option<&str>) -> Outcome {
        Outcome {
            namespace: namespace.to_string(),
            variant,
            lead: lead.to_string(),
            detail: detail.map(ToString::to_string),
        }
    }

    /// The band is the surface's sentence first and the engine's text after it —
    /// the same split the pause band makes. A band that only repeated the
    /// backend would be the vocabulary leaving the UI; one that dropped it would
    /// lose the only part naming what went wrong.
    #[wasm_bindgen_test]
    fn the_outcome_band_leads_with_the_page_s_sentence() {
        let outcome = RwSignal::new(Some(said(
            "team/dataset",
            BannerVariant::Critical,
            "Could not get the latest revision.",
            Some("Failed to pull package: connection reset"),
        )));
        let el =
            mount(move || outcome_band(outcome, Signal::derive(|| "team/dataset".to_string())));

        let text = el.text_content().unwrap_or_default();
        assert!(text.contains("Could not get the latest revision."));
        assert!(
            text.contains("connection reset"),
            "markup was {}",
            el.inner_html()
        );
    }

    /// The keying. A result arriving for a package the page no longer shows is
    /// dropped WHOLE — not greyed, not queued. A reader cannot tell a stale
    /// outcome from a fresh one by its text, which is the defect quilt-rs#974's
    /// review found.
    #[wasm_bindgen_test]
    fn an_outcome_for_another_package_is_dropped_whole() {
        let outcome = RwSignal::new(Some(said(
            "team/other",
            BannerVariant::Success,
            "The last revision was undone.",
            None,
        )));
        let el =
            mount(move || outcome_band(outcome, Signal::derive(|| "team/dataset".to_string())));

        assert_eq!(el.text_content().unwrap_or_default().trim(), "");
    }

    /// Both bands at once. A pause is a standing condition and an outcome is
    /// what just happened; a package can be both, and neither replaces the other.
    #[wasm_bindgen_test]
    fn an_outcome_and_a_pause_stack_rather_than_replacing_each_other() {
        let outcome = RwSignal::new(Some(said(
            "team/dataset",
            BannerVariant::Success,
            "The last revision was undone.",
            None,
        )));
        let dismissed = RwSignal::new(None);
        let el = mount(move || {
            view! {
                {outcome_band(outcome, Signal::derive(|| "team/dataset".to_string()))}
                {pause_banner(Some("workflow rejected the revision".to_string()), dismissed)}
            }
        });

        let text = el.text_content().unwrap_or_default();
        assert!(text.contains("The last revision was undone."));
        assert!(text.contains("Autosync has stopped for this package"));
    }

    /// A success waits for a pause in the reader's work; a failure cuts across
    /// it. That is `BannerVariant`'s own rule and the band must not quietly
    /// invert it.
    #[wasm_bindgen_test]
    fn a_failure_interrupts_and_a_success_does_not() {
        for (variant, role) in [
            (BannerVariant::Critical, "alert"),
            (BannerVariant::Success, "status"),
        ] {
            let outcome = RwSignal::new(Some(said("team/dataset", variant, "Something.", None)));
            let el =
                mount(move || outcome_band(outcome, Signal::derive(|| "team/dataset".to_string())));
            let band = el.query_selector("[role]").unwrap().expect("a band");
            assert_eq!(band.get_attribute("role").as_deref(), Some(role));
        }
    }

    /// A second outcome for the same package replaces the first while the band is
    /// up — `Show`'s guard stays true across it, so only the children's own read
    /// of `outcome` redraws it.
    #[wasm_bindgen_test]
    async fn a_newer_outcome_replaces_the_one_on_screen() {
        let outcome = RwSignal::new(Some(said(
            "team/dataset",
            BannerVariant::Critical,
            "Could not open this package's folder.",
            None,
        )));
        let el =
            mount(move || outcome_band(outcome, Signal::derive(|| "team/dataset".to_string())));
        outcome.set(Some(said(
            "team/dataset",
            BannerVariant::Success,
            "Undid the last revision.",
            None,
        )));
        leptos::task::tick().await;

        let band = el.query_selector("[role]").unwrap().expect("a band");
        assert_eq!(band.get_attribute("role").as_deref(), Some("status"));
        let text = band.text_content().unwrap_or_default();
        assert!(text.contains("Undid the last revision."), "got: {text}");
        assert!(
            !text.contains("folder"),
            "the first outcome is gone: {text}"
        );
    }

    /// A command that starts retracts the band's last outcome, so a failure
    /// cannot outlive the retry that succeeds — success says nothing, and would
    /// otherwise leave the failure up. Read from inside the command, because
    /// under the runner every real one fails and would set its own.
    #[wasm_bindgen_test]
    async fn a_command_that_starts_retracts_the_last_outcome() {
        let busy = RwSignal::new(false);
        let outcome = RwSignal::new(Some(said(
            "team/dataset",
            BannerVariant::Critical,
            "Could not open this package's folder.",
            None,
        )));

        let during = holding(busy, outcome, async move {
            (busy.get_untracked(), outcome.get_untracked())
        })
        .await;

        assert_eq!(during, (true, None), "held, and the band cleared");
        assert!(!busy.get_untracked(), "released once it settles");
    }

    /// A command refused with `Error::to_frontend_string`'s JSON puts the message
    /// on the band, never the envelope it travelled in.
    #[wasm_bindgen_test]
    async fn a_failed_command_shows_the_backend_s_message_not_its_json() {
        const DENIED: &str = "The active role does not have access to this object.";
        let busy = RwSignal::new(false);
        let outcome = RwSignal::new(None);
        let el =
            mount(move || outcome_band(outcome, Signal::derive(|| "team/dataset".to_string())));
        run(
            busy,
            outcome,
            "team/dataset".to_string(),
            "Could not make your revision the shared one.",
            Reread::Never,
            async {
                Err(
                    r#"{"kind":"access_denied","message":"The active role does not have access to this object."}"#
                        .to_string(),
                )
            },
        );
        sleep_ms(0).await;
        leptos::task::tick().await;

        let text = el.text_content().unwrap_or_default();
        assert!(
            text.contains("Could not make your revision the shared one."),
            "markup was {}",
            el.inner_html()
        );
        assert!(
            text.contains(DENIED),
            "the message is drawn; markup was {}",
            el.inner_html()
        );
        assert!(
            !text.contains("\"kind\""),
            "the JSON is not; markup was {}",
            el.inner_html()
        );
    }

    /// A successful payload swaps the header and pane together. The old loose
    /// paragraph was only scaffolding; package identity now belongs to the
    /// header while the pane is a named complementary landmark.
    #[wasm_bindgen_test]
    fn a_successful_payload_draws_the_real_body_without_the_placeholder() {
        // Inside a `Router`, where the page always is.
        let el = mount(|| {
            let w = Wiring::new();
            let resolving = ResolveCommands::app();
            view! {
                <Router>{package_body(page_data(), w, Signal::stored(false), resolving, idle_files(), idle_incoming())}</Router>
            }
        });
        let aside = el
            .query_selector("aside")
            .unwrap()
            .expect("the context pane");
        assert_eq!(
            aside.get_attribute("aria-label").as_deref(),
            Some("About this package")
        );
        // The header's closed dialogs and Keeping's group carry their own paragraphs;
        // only loose ones count.
        let paragraphs = el.query_selector_all("p").unwrap();
        let loose = (0..paragraphs.length())
            .filter_map(|i| paragraphs.item(i))
            .filter_map(|n| n.dyn_into::<web_sys::Element>().ok())
            .any(|p| p.closest("dialog, [role=radiogroup]").unwrap().is_none());
        assert!(
            !loose,
            "the loose namespace placeholder is gone; markup was {}",
            el.inner_html()
        );
    }

    /// Reading and focus order is context first, files second: the context
    /// pane precedes the file pane in the DOM, whatever the stylesheet draws.
    #[wasm_bindgen_test]
    fn the_context_pane_precedes_the_file_pane_in_document_order() {
        let el = mount(|| {
            let w = Wiring::new();
            let resolving = ResolveCommands::app();
            view! {
                <Router>{package_body(page_data(), w, Signal::stored(false), resolving, idle_files(), idle_incoming())}</Router>
            }
        });
        let context = el
            .query_selector("aside[aria-label='About this package']")
            .unwrap()
            .expect("the context pane");
        let files = el
            .query_selector("section[aria-label='Files']")
            .unwrap()
            .expect("the file pane");
        let following = context.compare_document_position(&files);
        assert_ne!(
            following & web_sys::Node::DOCUMENT_POSITION_FOLLOWING,
            0,
            "the file pane follows the context pane; markup was {}",
            el.inner_html()
        );
    }

    /// The live pane's trigger arrives with the body, stating the page read's
    /// count, and nothing opens until it is pressed: the list is lazy.
    #[wasm_bindgen_test]
    fn the_body_carries_the_revision_trigger() {
        let el = mount(|| {
            let w = Wiring::new();
            let resolving = ResolveCommands::app();
            view! {
                <Router>{package_body(page_data(), w, Signal::stored(false), resolving, idle_files(), idle_incoming())}</Router>
            }
        });
        let trigger = element_saying(&el, "Revisions you have (1)")
            .closest("button")
            .unwrap()
            .expect("the trigger is a button");
        assert_eq!(
            trigger.get_attribute("aria-expanded").as_deref(),
            Some("false")
        );
        assert!(
            el.query_selector(":popover-open").unwrap().is_none(),
            "no surface is open; markup was {}",
            el.inner_html()
        );
    }

    /// The live pane's Keeping arrives with the body, drawn from the page read:
    /// the stored scope chosen, the present count, and no download at zero.
    #[wasm_bindgen_test]
    fn the_body_carries_keeping() {
        let el = mount(|| {
            let w = Wiring::new();
            let resolving = ResolveCommands::app();
            view! {
                <Router>{package_body(page_data(), w, Signal::stored(false), resolving, idle_files(), idle_incoming())}</Router>
            }
        });
        let group = el
            .query_selector("[role=radiogroup]")
            .unwrap()
            .expect("a radiogroup");
        let label = web_sys::window()
            .unwrap()
            .document()
            .unwrap()
            .get_element_by_id(&group.get_attribute("aria-labelledby").unwrap())
            .expect("the group's label");
        assert_eq!(label.text_content().unwrap_or_default().trim(), "Keeping");
        let pick: web_sys::HtmlInputElement = element_saying(&group, "Files I pick")
            .closest("label")
            .unwrap()
            .expect("the option's label")
            .query_selector("input[type=radio]")
            .unwrap()
            .expect("the option's radio")
            .unchecked_into();
        assert!(pick.checked(), "the stored scope is chosen");
        // The page read's size reaches the caption.
        element_saying(&el, "All files are downloaded · 7\u{a0}B.");
        let buttons = el.query_selector_all("button").unwrap();
        let download = (0..buttons.length())
            .filter_map(|i| buttons.item(i))
            .any(|b| b.text_content().unwrap_or_default().contains("Download"));
        assert!(
            !download,
            "nothing outstanding, no download; markup was {}",
            el.inner_html()
        );
    }

    /// A re-read rebuilds the body, and with it the header. A dialog the reader
    /// has open is the page's, so it survives — the watcher reporting news
    /// mid-form must not shut the form.
    #[wasm_bindgen_test]
    async fn an_open_dialog_stays_open_when_the_body_is_re_read() {
        let w = Wiring::new();
        let reads = RwSignal::new(0_u32);
        let el = mount(move || {
            view! {
                <Router>
                    {move || {
                        reads.track();
                        package_body(page_data(), w, Signal::stored(false), ResolveCommands::app(), idle_files(), idle_incoming())
                    }}
                </Router>
            }
        });
        w.dialogs.remove.set(true);
        leptos::task::tick().await;
        let opened = || {
            el.query_selector("dialog[open]")
                .unwrap()
                .and_then(|d| d.text_content())
                .unwrap_or_default()
        };
        assert!(
            opened().contains("Remove this package"),
            "open before the re-read; markup was {}",
            el.inner_html()
        );

        reads.update(|n| *n += 1);
        leptos::task::tick().await;

        assert!(
            opened().contains("Remove this package"),
            "and still open after it; markup was {}",
            el.inner_html()
        );
    }

    /// The pane's fixed measure is a wide-layout decision, and the page's own
    /// inline container releases it — and stacks the shell — when it narrows.
    #[test]
    fn the_shell_and_pane_styles_own_the_responsive_width() {
        const PAGE: &str = include_str!("installed_package_v2.module.scss");
        const PANE: &str = include_str!("installed_package_v2/context_pane.module.scss");

        assert!(PAGE.contains("container-type: inline-size"));
        assert!(PAGE.contains("@container (max-width: 800px)"));
        assert!(
            PAGE.contains(".shell > aside {\n  order: 1;"),
            "files on the leading side while the two share a row"
        );
        assert!(
            PAGE.contains("order: 0"),
            "context above files, in DOM order, when stacked"
        );
        assert!(PANE.contains("width: 280px"));
        assert!(PANE.contains("@container (max-width: 800px)"));
        assert!(PANE.contains("width: 100%"));
        assert!(
            !PANE.contains('#'),
            "the slice introduces no literal colour"
        );
    }

    /// The route parameter arrives and the page names it. A page that drew a
    /// fixed string would pass a weaker test and tell the next unit nothing.
    ///
    /// The seam's `It works!` is gone — the header took that space — and its
    /// assertion went with it. What replaced it is stronger: there is no Tauri
    /// host under the test runner, so the page's read fails, and this now also
    /// holds the failure arm to keeping the frame and the package's name rather
    /// than blanking the page.
    ///
    /// Async because the router resolves a location one tick after the mount —
    /// queried synchronously the container is still a comment marker.
    #[wasm_bindgen_test]
    async fn the_page_names_the_package_the_address_asked_for() {
        go_to("/installed-package?namespace=team%2Fdataset");
        let el = mount(|| {
            view! {
                <Router>
                    <Routes fallback=|| view! { "no route" }>
                        <Route path=path!("/installed-package") view=InstalledPackageV2 />
                    </Routes>
                </Router>
            }
        });
        sleep_ms(50).await;

        element_saying(&el, "team/dataset");
        element_saying(&el, "Could not load this package.");
    }

    /// The residue's own words, and the engine's message after them. A band that
    /// only repeated the backend's sentence would be the vocabulary leaving the
    /// UI; one that dropped it would lose the only part naming what to fix.
    #[wasm_bindgen_test]
    fn the_pause_band_says_what_stopped_and_what_the_engine_reported() {
        let dismissed = RwSignal::new(None);
        let el = mount(move || {
            pause_banner(
                Some("workflow rejected the revision".to_string()),
                dismissed,
            )
        });

        let text = el.text_content().unwrap_or_default();
        assert!(
            text.contains("Autosync has stopped for this package"),
            "the page writes the sentence; markup was {}",
            el.inner_html()
        );
        assert!(
            text.contains("workflow rejected the revision"),
            "and the engine's own reason is the detail; markup was {}",
            el.inner_html()
        );
    }

    /// The band agrees with the chip, which `DESIGN.md` requires of every tone
    /// and this one got wrong: `PackageState::Paused` is Danger, so the band is
    /// `Critical`. Asserted through `role`, which is what the variant produces
    /// — a test on the enum would restate the call site.
    #[wasm_bindgen_test]
    fn the_pause_band_is_toned_as_the_failure_it_reports() {
        let dismissed = RwSignal::new(None);
        let el = mount(move || {
            pause_banner(
                Some("workflow rejected the revision".to_string()),
                dismissed,
            )
        });

        let band = el
            .query_selector("[role]")
            .unwrap()
            .expect("the band carries a role");
        assert_eq!(
            band.get_attribute("role").as_deref(),
            Some("alert"),
            "markup was {}",
            el.inner_html()
        );
    }

    /// No pause, no band. The slot must not reserve a strip for news that is
    /// usually absent.
    #[wasm_bindgen_test]
    fn an_unpaused_package_draws_no_band() {
        let dismissed = RwSignal::new(None);
        let el = mount(move || pause_banner(None, dismissed));
        assert_eq!(el.text_content().unwrap_or_default().trim(), "");
    }

    /// Dismissal is keyed on the message, not a flag: a pause the reader has
    /// read and closed stays closed, and a different one is news again. Both
    /// halves, because a flag would pass the first and fail the second.
    #[wasm_bindgen_test]
    fn a_dismissed_pause_stays_closed_and_a_different_one_does_not() {
        let dismissed = RwSignal::new(Some("workflow rejected the revision".to_string()));

        let closed = mount(move || {
            pause_banner(
                Some("workflow rejected the revision".to_string()),
                dismissed,
            )
        });
        assert_eq!(
            closed.text_content().unwrap_or_default().trim(),
            "",
            "the message the reader closed stays closed"
        );

        let fresh = mount(move || pause_banner(Some("hash mismatch".to_string()), dismissed));
        assert!(
            fresh
                .text_content()
                .unwrap_or_default()
                .contains("hash mismatch"),
            "a different pause is news again; markup was {}",
            fresh.inner_html()
        );
    }

    /// The appbar carries the v2 pair, in order.
    ///
    /// *Refresh* matters more here than it looks: the page follows the watcher,
    /// so this is not the staleness fix it would once have been — it is the
    /// escape hatch for the one thing the streams cannot report, a pause
    /// CLEARING, which emits no event at all. A page showing the pause band can
    /// otherwise keep showing it after autosync is re-enabled.
    ///
    /// *Settings* is the way off a page the logo cannot leave: `/` renders
    /// whichever main page is switched on, so it is not an exit from v2.
    #[wasm_bindgen_test]
    async fn the_appbar_offers_refresh_and_settings() {
        go_to("/installed-package?namespace=team%2Fdataset");
        let el = mount(|| {
            view! {
                <Router>
                    <Routes fallback=|| view! { "no route" }>
                        <Route path=path!("/installed-package") view=InstalledPackageV2 />
                    </Routes>
                </Router>
            }
        });
        sleep_ms(50).await;

        element_saying(&el, "Refresh");
        element_saying(&el, "Settings");
    }

    /// v2's frame, not v1's. `data-v2-page` is what the stylesheet keys the
    /// palette and `color-scheme` on, so without it the page is drawn in v1's
    /// fixed light chrome whatever the desktop is set to.
    #[wasm_bindgen_test]
    async fn the_page_is_drawn_in_the_v2_frame() {
        go_to("/installed-package?namespace=team%2Fdataset");
        let el = mount(|| {
            view! {
                <Router>
                    <Routes fallback=|| view! { "no route" }>
                        <Route path=path!("/installed-package") view=InstalledPackageV2 />
                    </Routes>
                </Router>
            }
        });
        sleep_ms(50).await;

        assert!(
            el.query_selector("[data-v2-page]").unwrap().is_some(),
            "the placeholder sits in the v2 page frame; markup was {}",
            el.inner_html()
        );
    }

    const PLAIN: &str = "/installed-package?namespace=team%2Fdataset&filter=unmodified";
    const RESOLVE: &str = "/installed-package?namespace=team%2Fdataset&filter=unmodified&resolve=1";

    fn compared() -> commands::ResolveData {
        commands::ResolveData::Compared {
            published_message: Some("Theirs".to_string()),
            differing: vec!["plate/a.csv".to_string(), "plate/b.csv".to_string()],
            unpublished: 1,
            uncommitted: 0,
        }
    }

    fn refused() -> commands::ResolveData {
        commands::ResolveData::Refused {
            reason: "AccessDenied".to_string(),
        }
    }

    fn diverged(resolve: commands::ResolveData) -> commands::PackagePageData {
        let mut data = page_data();
        data.header.state = crate::kit::PackageState::Diverged;
        data.context.resolve = Some(resolve);
        data
    }

    #[test]
    fn only_an_answered_read_that_is_not_diverged_normalises() {
        assert_eq!(
            normalized_address(true, "team/dataset", &page_data(), None).as_deref(),
            Some(PLAIN)
        );
        assert_eq!(
            normalized_address(true, "team/dataset", &diverged(compared()), None),
            None
        );
        assert_eq!(
            normalized_address(true, "team/dataset", &diverged(refused()), None),
            None
        );
        assert_eq!(
            normalized_address(false, "team/dataset", &page_data(), None),
            None
        );
    }

    #[test]
    fn a_read_about_another_package_decides_nothing() {
        assert_eq!(
            normalized_address(true, "other/pkg", &page_data(), None),
            None
        );
    }

    /// The deep link's mismatch survives the replacement, so leaving a mode
    /// the package cannot have does not end the band.
    #[test]
    fn the_normalised_address_keeps_the_mismatch() {
        let mismatch = routes::DeepLinkOutcome::Mismatch(routes::RevisionMismatch {
            hash: "c41d8f02".to_string(),
            bucket: "quilt-lab".to_string(),
            catalog: Some("https://quilt.test".to_string()),
        });
        assert_eq!(
            normalized_address(true, "team/dataset", &page_data(), Some(&mismatch)),
            Some(format!(
                "{PLAIN}&mismatch=c41d8f02&mrbucket=quilt-lab&mrcatalog=https%3A%2F%2Fquilt.test"
            ))
        );
    }

    /// The local-only flag survives the replacement the same way.
    #[test]
    fn the_normalised_address_keeps_local_only() {
        assert_eq!(
            normalized_address(
                true,
                "team/dataset",
                &page_data(),
                Some(&routes::DeepLinkOutcome::LocalOnly)
            ),
            Some(format!("{PLAIN}&localOnly=1"))
        );
    }

    thread_local! {
        static READS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    }

    type Read = std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<commands::PackagePageData, String>>>,
    >;

    fn counted(data: commands::PackagePageData) -> Read {
        READS.with(|r| r.set(r.get() + 1));
        Box::pin(async move { Ok(data) })
    }

    fn diverged_read(_: String) -> Read {
        counted(diverged(compared()))
    }

    fn refused_read(_: String) -> Read {
        counted(diverged(refused()))
    }

    fn settled_read(_: String) -> Read {
        counted(page_data())
    }

    /// Answers the first read, then never answers again: a re-read in flight.
    fn answers_once(_: String) -> Read {
        if READS.with(std::cell::Cell::get) == 0 {
            counted(page_data())
        } else {
            pending_read(String::new())
        }
    }

    fn pending_read(_: String) -> Read {
        READS.with(|r| r.set(r.get() + 1));
        Box::pin(std::future::pending())
    }

    /// The page at `address`, reading through `read`, inside a router.
    async fn screen_at(address: &str, read: PageRead) -> web_sys::Element {
        screen_resolving(address, read, ResolveCommands::app()).await
    }

    /// [`screen_at`], mounted as `/installed-package` mounts it: under
    /// `ByDesign`'s `Suspense`, whose fallback is the loading frame.
    async fn suspended_screen_at(address: &str, read: PageRead) -> web_sys::Element {
        crate::test_support::unmount_earlier();
        READS.with(|r| r.set(0));
        go_to(address);
        let el = mount(move || {
            view! {
                <Router>
                    <Routes fallback=|| view! { "no route" }>
                        <Route
                            path=path!("/installed-package")
                            view=move || {
                                view! {
                                    <Suspense fallback=|| view! { <p data-fallback>"loading"</p> }>
                                        <PackageScreen
                                            read=read
                                            pull=never_pulls
                                            resolving=ResolveCommands::app()
                                            revision_message=mismatch_band::app_revision_message
                                        />
                                    </Suspense>
                                }
                            }
                        />
                    </Routes>
                </Router>
            }
        });
        sleep_ms(50).await;
        el
    }

    /// [`screen_at`], with the resolve mode's commands answered by `resolving`.
    async fn screen_resolving(
        address: &str,
        read: PageRead,
        resolving: ResolveCommands,
    ) -> web_sys::Element {
        crate::test_support::unmount_earlier();
        READS.with(|r| r.set(0));
        go_to(address);
        let el = mount(move || {
            view! {
                <Router>
                    <Routes fallback=|| view! { "no route" }>
                        <Route
                            path=path!("/installed-package")
                            view=move || {
                                view! {
                                    <PackageScreen
                                        read=read
                                        pull=never_pulls
                                        resolving=resolving
                                        revision_message=mismatch_band::app_revision_message
                                    />
                                }
                            }
                        />
                    </Routes>
                </Router>
            }
        });
        sleep_ms(50).await;
        el
    }

    /// `PackagePageSkeleton` is the page's own first paint, so
    /// `/installed-package` handing its loading frame over to the page moves
    /// nothing: the same boxes from the appbar down, the banner's empty row
    /// included, as the page draws while its one read is out.
    #[wasm_bindgen_test]
    async fn the_skeleton_is_the_page_s_first_paint() {
        use crate::components::appbar::appbar_actions;
        use crate::test_support::shape;

        let page = screen_at("/installed-package?namespace=team%2Fdataset", pending_read).await;
        let skeleton = mount(|| {
            view! {
                <Router>
                    <PackagePageSkeleton actions=appbar_actions(|| (), Signal::stored(true)) />
                </Router>
            }
        });
        leptos::task::tick().await;

        let frame = |el: &web_sys::Element| {
            shape(
                &el.query_selector("[data-v2-page]")
                    .unwrap()
                    .expect("a v2 page"),
            )
        };
        assert_eq!(frame(&skeleton), frame(&page));
    }

    /// The file pane's toolbar is the answered pane's, spot for spot: the
    /// select-all line on the left, which an answered pane with files always
    /// draws, and the view controls on the right. A skeleton without the first
    /// sat a line short of the pane that replaced it once the toolbar stacked.
    #[wasm_bindgen_test]
    async fn the_skeleton_keeps_select_all_s_spot() {
        use crate::components::appbar::appbar_actions;

        // The classes of the toolbar's direct children, which name the spots.
        fn spots(el: &web_sys::Element) -> Vec<String> {
            let bar = el
                .query_selector("section[aria-label=Files] > [class^=listing] > :first-child")
                .unwrap()
                .expect("the file pane's toolbar");
            let children = bar.children();
            (0..children.length())
                .map(|i| {
                    children
                        .item(i)
                        .unwrap()
                        .get_attribute("class")
                        .unwrap_or_default()
                })
                .collect()
        }

        let page = screen_at("/installed-package?namespace=team%2Fdataset", settled_read).await;
        let answered = spots(&page);
        let skeleton = mount(|| {
            view! {
                <Router>
                    <PackagePageSkeleton actions=appbar_actions(|| (), Signal::stored(true)) />
                </Router>
            }
        });
        leptos::task::tick().await;

        assert_eq!(answered.len(), 2, "select-all and the views: {answered:?}");
        assert_eq!(spots(&skeleton), answered);
    }

    fn search() -> String {
        web_sys::window().unwrap().location().search().unwrap()
    }

    fn history_length() -> u32 {
        web_sys::window()
            .unwrap()
            .history()
            .unwrap()
            .length()
            .unwrap()
    }

    #[wasm_bindgen_test]
    async fn resolve_1_on_a_diverged_package_opens_the_mode() {
        let el = screen_at(RESOLVE, diverged_read).await;

        element_saying(&el, "Yours");
        element_saying(&el, "Published");
        assert!(
            !el.text_content()
                .unwrap_or_default()
                .contains("Revisions you have (1)"),
            "the ordinary pane is swapped out; markup was {}",
            el.inner_html()
        );
        assert!(search().ends_with("&resolve=1"), "search was {}", search());
    }

    #[wasm_bindgen_test]
    async fn resolve_1_on_a_package_that_is_not_diverged_normalises_in_place() {
        let before = history_length();
        let el = screen_at(RESOLVE, settled_read).await;

        assert_eq!(search(), "?namespace=team%2Fdataset&filter=unmodified");
        assert_eq!(history_length(), before, "replaced, not pushed");
        element_saying(&el, "Revisions you have (1)");
    }

    /// The watcher's re-read, or Refresh, must not blank the list while it is
    /// out: the list is on screen before and still there during it.
    #[wasm_bindgen_test]
    async fn a_re_read_in_flight_keeps_the_file_list() {
        let el = screen_at(PLAIN, answers_once).await;
        let row = || {
            el.query_selector("section[aria-label='Files'] [title='a.csv']")
                .unwrap()
                .is_some()
        };
        assert!(row(), "listed; markup was {}", el.inner_html());

        button_saying(&el, "Refresh").click();
        sleep_ms(50).await;

        assert_eq!(READS.with(std::cell::Cell::get), 2, "the page re-read");
        assert!(row(), "still listed; markup was {}", el.inner_html());
    }

    /// Mounted as the route mounts it, a re-read still in flight leaves the
    /// page up: `ByDesign`'s `Suspense` must not put its loading frame back.
    #[wasm_bindgen_test]
    async fn a_re_read_under_by_design_keeps_the_page() {
        let el = suspended_screen_at(PLAIN, answers_once).await;
        let row = || {
            el.query_selector("section[aria-label='Files'] [title='a.csv']")
                .unwrap()
        };
        assert!(row().is_some(), "listed; markup was {}", el.inner_html());

        button_saying(&el, "Refresh").click();
        sleep_ms(50).await;

        assert_eq!(READS.with(std::cell::Cell::get), 2, "the page re-read");
        assert!(
            el.query_selector("[data-fallback]").unwrap().is_none(),
            "the loading frame came back; markup was {}",
            el.inner_html()
        );
        assert!(
            row().is_some(),
            "still listed; markup was {}",
            el.inner_html()
        );
    }

    /// A re-read that answers what is already on screen changes nothing on
    /// it. The watcher's first event after the page opens is such a re-read.
    #[wasm_bindgen_test]
    async fn an_unchanged_re_read_rebuilds_nothing() {
        let el = suspended_screen_at(PLAIN, settled_read).await;
        let row = el
            .query_selector("section[aria-label='Files'] [title='a.csv']")
            .unwrap()
            .expect("listed");

        button_saying(&el, "Refresh").click();
        sleep_ms(50).await;

        assert_eq!(READS.with(std::cell::Cell::get), 2, "the page re-read");
        assert!(row.is_connected(), "the row was rebuilt");
    }

    /// Every package answered as the namespace asked, with one folder of two.
    fn a_folder_as_asked(namespace: String) -> Read {
        let namespace: quilt_uri::Namespace = namespace.try_into().unwrap();
        let mut data = page_data();
        data.header.namespace = namespace.clone();
        let entry = |path: &str| commands::EntryData {
            filename: path.to_string(),
            size: 7,
            status: "pristine".to_string(),
            junky_pattern: None,
            ignored_by: None,
            namespace: namespace.clone(),
        };
        data.files = commands::FilesData::Listed(commands::EntryList {
            entries: vec![entry("notes/a.md"), entry("notes/b.md")],
            counts: commands::EntryCounts {
                all: 2,
                ..commands::EntryCounts::default()
            },
            total: 2,
            truncated: false,
        });
        counted(data)
    }

    /// A row's Ignore opens the ignore popup with the row's path as its
    /// pattern, and the watcher's re-read neither closes it nor loses what was
    /// typed: its state is the page's, not the body's.
    #[wasm_bindgen_test]
    async fn ignore_opens_the_popup_with_the_path_and_a_re_read_keeps_it() {
        let el = screen_at(PLAIN, settled_read).await;
        el.query_selector(
            "section[aria-label='Files'] button[aria-label='More actions for this file']",
        )
        .unwrap()
        .expect("the row's [⋯]")
        .unchecked_into::<web_sys::HtmlElement>()
        .click();
        sleep_ms(10).await;
        button_saying(&el, "Ignore").click();
        sleep_ms(10).await;
        let field = || -> web_sys::HtmlInputElement {
            el.query_selector(".ignore-input")
                .unwrap()
                .unwrap_or_else(|| panic!("the ignore popup; markup was {}", el.inner_html()))
                .unchecked_into()
        };
        assert_eq!(field().value(), "a.csv");

        field().set_value("*.csv");
        field()
            .dispatch_event(&web_sys::Event::new("input").unwrap())
            .unwrap();
        button_saying(&el, "Refresh").click();
        sleep_ms(50).await;
        assert_eq!(READS.with(std::cell::Cell::get), 2, "the page re-read");
        assert_eq!(field().value(), "*.csv", "kept across the re-read");
    }

    /// A collapsed folder is the page's: a re-read keeps it, and another
    /// package starts with every folder open.
    #[wasm_bindgen_test]
    async fn a_collapsed_folder_survives_a_re_read_but_not_another_package() {
        let el = screen_at(PLAIN, a_folder_as_asked).await;
        let expanded = || {
            el.query_selector("section[aria-label='Files'] [aria-expanded]")
                .unwrap()
                .expect("the folder's disclosure")
                .get_attribute("aria-expanded")
        };
        el.query_selector("section[aria-label='Files'] [aria-expanded]")
            .unwrap()
            .expect("the folder's disclosure")
            .unchecked_into::<web_sys::HtmlElement>()
            .click();
        sleep_ms(10).await;
        assert_eq!(expanded().as_deref(), Some("false"));

        button_saying(&el, "Refresh").click();
        sleep_ms(50).await;
        assert_eq!(READS.with(std::cell::Cell::get), 2, "the page re-read");
        assert_eq!(
            expanded().as_deref(),
            Some("false"),
            "kept across the re-read"
        );

        move_to("/installed-package?namespace=team%2Fother");
        sleep_ms(50).await;
        assert_eq!(
            expanded().as_deref(),
            Some("true"),
            "open again on another package; markup was {}",
            el.inner_html()
        );
    }

    /// The search is the page's, like the folders: a re-read keeps it, and
    /// another package starts with none.
    #[wasm_bindgen_test]
    async fn the_search_survives_a_re_read_but_not_another_package() {
        let el = screen_at(PLAIN, a_folder_as_asked).await;
        let field = || -> web_sys::HtmlInputElement {
            el.query_selector("section[aria-label='Files'] input[type=search]")
                .unwrap()
                .expect("the search field")
                .unchecked_into()
        };
        field().set_value("b.md");
        field()
            .dispatch_event(&web_sys::Event::new("input").unwrap())
            .unwrap();
        sleep_ms(40).await;
        let shown = |path: &str| {
            el.query_selector(&format!("section[aria-label='Files'] [title='{path}']"))
                .unwrap()
                .is_some()
        };
        assert!(
            !shown("notes/a.md"),
            "narrowed; markup was {}",
            el.inner_html()
        );

        button_saying(&el, "Refresh").click();
        sleep_ms(50).await;
        assert_eq!(READS.with(std::cell::Cell::get), 2, "the page re-read");
        assert_eq!(field().value(), "b.md", "kept across the re-read");
        assert!(!shown("notes/a.md"), "still narrowed");

        move_to("/installed-package?namespace=team%2Fother");
        sleep_ms(50).await;
        assert_eq!(field().value(), "", "cleared on another package");
        assert!(
            shown("notes/a.md"),
            "every row again; markup was {}",
            el.inner_html()
        );
    }

    /// Typing never reads the page: search narrows the rows the last read
    /// sent, so no keystroke asks for the package's status again.
    #[wasm_bindgen_test]
    async fn typing_a_search_never_reads_the_page() {
        let el = screen_at(PLAIN, a_folder_as_asked).await;
        assert_eq!(READS.with(std::cell::Cell::get), 1, "the page's one read");
        let field: web_sys::HtmlInputElement = el
            .query_selector("section[aria-label='Files'] input[type=search]")
            .unwrap()
            .expect("the search field")
            .unchecked_into();
        for query in ["n", "no", "not", "notes/", "notes/b", "", "a"] {
            field.set_value(query);
            field
                .dispatch_event(&web_sys::Event::new("input").unwrap())
                .unwrap();
            sleep_ms(5).await;
        }
        sleep_ms(40).await;
        assert_eq!(
            READS.with(std::cell::Cell::get),
            1,
            "no keystroke reads the page again"
        );
    }

    /// Answers for `team/dataset` only; every other package's read is out.
    fn answers_dataset_only(namespace: String) -> Read {
        if namespace == "team/dataset" {
            counted(page_data())
        } else {
            pending_read(namespace)
        }
    }

    /// Moving on, the last package's files are not drawn at the new address.
    #[wasm_bindgen_test]
    async fn another_package_s_read_in_flight_draws_no_stale_files() {
        let el = screen_at(PLAIN, answers_dataset_only).await;
        let row = || {
            el.query_selector("section[aria-label='Files'] [title='a.csv']")
                .unwrap()
                .is_some()
        };
        assert!(row(), "listed; markup was {}", el.inner_html());

        move_to("/installed-package?namespace=team%2Fother");
        sleep_ms(50).await;

        assert!(
            !row(),
            "not under another package; markup was {}",
            el.inner_html()
        );
        assert!(
            el.query_selector("[aria-busy=true]").unwrap().is_some(),
            "the skeleton instead; markup was {}",
            el.inner_html()
        );
    }

    #[wasm_bindgen_test]
    async fn a_read_still_loading_decides_nothing() {
        let _el = screen_at(RESOLVE, pending_read).await;

        assert!(search().ends_with("&resolve=1"), "search was {}", search());
    }

    #[wasm_bindgen_test]
    async fn a_refused_comparison_keeps_the_mode() {
        let el = screen_at(RESOLVE, refused_read).await;

        element_saying(&el, "Could not compare the revisions.");
        assert_eq!(
            search(),
            "?namespace=team%2Fdataset&filter=unmodified&resolve=1"
        );
    }

    #[wasm_bindgen_test]
    async fn the_back_link_leaves_the_mode_in_place() {
        let el = screen_at(RESOLVE, diverged_read).await;
        let before = history_length();

        el.query_selector(&format!("a[href='{PLAIN}']"))
            .unwrap()
            .expect("the back link")
            .unchecked_into::<web_sys::HtmlElement>()
            .click();
        sleep_ms(50).await;

        assert_eq!(search(), "?namespace=team%2Fdataset&filter=unmodified");
        assert_eq!(
            history_length(),
            before,
            "replaced, so Back does not return to the mode"
        );
        element_saying(&el, "Revisions you have (1)");
        assert_eq!(
            READS.with(std::cell::Cell::get),
            1,
            "leaving does not re-read"
        );
    }

    thread_local! {
        static CERTIFY_RELEASED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }

    /// Every package diverged, each answered as the namespace asked.
    fn diverged_as_asked(namespace: String) -> Read {
        let mut data = diverged(compared());
        data.header.namespace = namespace.try_into().unwrap();
        counted(data)
    }

    /// A certify that succeeds once the test releases it.
    fn certifies_when_released(_: String, _: Option<quilt_uri::S3PackageUri>) -> Choice {
        Box::pin(async {
            while !CERTIFY_RELEASED.get() {
                sleep_ms(5).await;
            }
            Ok("certified".to_string())
        })
    }

    type Choice = std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, String>>>>;

    /// Moving between packages the way the router hears it: a new entry, then `popstate`.
    fn move_to(address: &str) {
        let window = web_sys::window().unwrap();
        window
            .history()
            .unwrap()
            .push_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some(address))
            .unwrap();
        window
            .dispatch_event(&web_sys::Event::new("popstate").unwrap())
            .unwrap();
    }

    #[wasm_bindgen_test]
    async fn a_late_certify_for_a_package_left_behind_does_not_drag_the_reader_back() {
        const OTHER: &str = "/installed-package?namespace=team%2Fother";
        CERTIFY_RELEASED.set(false);
        let el = screen_resolving(
            RESOLVE,
            diverged_as_asked,
            ResolveCommands {
                certify: certifies_when_released,
                reset: |_, _| Box::pin(std::future::pending()),
            },
        )
        .await;
        button_saying(&el, "Share mine").click();
        sleep_ms(10).await;

        move_to(OTHER);
        sleep_ms(50).await;
        assert_eq!(search(), "?namespace=team%2Fother", "on the other package");

        CERTIFY_RELEASED.set(true);
        sleep_ms(50).await;

        assert_eq!(search(), "?namespace=team%2Fother", "still on it");
        assert!(
            !el.text_content()
                .unwrap_or_default()
                .contains("Your revision is now the shared one."),
            "the first package's success is not drawn here; markup was {}",
            el.inner_html()
        );
    }

    #[wasm_bindgen_test]
    fn the_marks_exist_only_while_the_mode_is_open() {
        let open = RwSignal::new(false);
        let resolve = compared();
        let marks = differing_marks(open.into(), Some(&resolve));
        assert_eq!(marks.get_untracked(), None, "closed");

        open.set(true);
        let expected: std::collections::BTreeSet<String> = ["plate/a.csv", "plate/b.csv"]
            .into_iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(
            marks.get_untracked(),
            Some(std::sync::Arc::new(expected)),
            "open"
        );

        let refused = refused();
        let marks = differing_marks(Signal::stored(true), Some(&refused));
        assert_eq!(marks.get_untracked(), None, "nothing to mark");
    }

    /// A diverged package whose list holds one differing file and one that
    /// is the same in both revisions.
    fn diverged_with_files(_: String) -> Read {
        let mut data = diverged(compared());
        let entry = |path: &str| commands::EntryData {
            filename: path.to_string(),
            size: 7,
            status: "pristine".to_string(),
            junky_pattern: None,
            ignored_by: None,
            namespace: "team/dataset".try_into().unwrap(),
        };
        data.files = commands::FilesData::Listed(commands::EntryList {
            entries: vec![entry("plate/a.csv"), entry("plate/c.csv")],
            counts: commands::EntryCounts {
                all: 2,
                ..commands::EntryCounts::default()
            },
            total: 2,
            truncated: false,
        });
        counted(data)
    }

    /// Whether the file list marks the row drawing `path`.
    fn row_marked(el: &web_sys::Element, path: &str) -> bool {
        el.query_selector(&format!("section[aria-label='Files'] [title='{path}']"))
            .unwrap()
            .unwrap_or_else(|| panic!("{path} is listed; markup was {}", el.inner_html()))
            .closest(&format!("[aria-describedby='{}']", crate::kit::DIFFERS_ID))
            .unwrap()
            .is_some()
    }

    /// In resolve mode the list marks the rows in the page's one differing
    /// set, and each names the resolve pane's sentence, which is on screen.
    #[wasm_bindgen_test]
    async fn resolve_mode_marks_the_differing_rows_in_the_file_list() {
        let el = screen_at(RESOLVE, diverged_with_files).await;

        assert!(row_marked(&el, "plate/a.csv"));
        assert!(
            !row_marked(&el, "plate/c.csv"),
            "the same in both revisions"
        );
        assert!(
            el.query_selector(&format!("#{}", crate::kit::DIFFERS_ID))
                .unwrap()
                .is_some(),
            "the description a marked row names is the resolve pane's sentence"
        );
        assert_eq!(READS.with(std::cell::Cell::get), 1, "no read of its own");
    }

    /// The same diverged package outside the mode marks nothing.
    #[wasm_bindgen_test]
    async fn outside_resolve_mode_no_row_is_marked() {
        let el = screen_at(PLAIN, diverged_with_files).await;

        assert!(!row_marked(&el, "plate/a.csv"));
        assert!(!row_marked(&el, "plate/c.csv"));
    }

    // ── The newer revision's dry run, through re-reads ──

    thread_local! {
        /// What the page read answers now; a test changes it between reads.
        static PAGE: std::cell::RefCell<Option<commands::PackagePageData>> =
            const { std::cell::RefCell::new(None) };
        /// The dry run's answers, in order, each after its delay in ms. An
        /// empty script never answers.
        static PULLS: std::cell::RefCell<std::collections::VecDeque<(i32, Result<commands::PullPreview, String>)>> =
            std::cell::RefCell::default();
        static PULLS_ASKED: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    }

    fn scripted_read(_: String) -> Read {
        counted(PAGE.with(|p| p.borrow().clone()).expect("a scripted page"))
    }

    fn scripted_pull(
        _: String,
    ) -> Pin<Box<dyn Future<Output = Result<commands::PullPreview, String>>>> {
        PULLS_ASKED.with(|n| n.set(n.get() + 1));
        match PULLS.with(|p| p.borrow_mut().pop_front()) {
            Some((delay, answer)) => Box::pin(async move {
                sleep_ms(delay).await;
                answer
            }),
            None => Box::pin(std::future::pending()),
        }
    }

    fn script(pulls: Vec<(i32, Result<commands::PullPreview, String>)>) {
        PULLS.with(|p| *p.borrow_mut() = pulls.into());
    }

    fn pulls_asked() -> u32 {
        PULLS_ASKED.with(std::cell::Cell::get)
    }

    /// `team/dataset` with a newer revision available, holding `holds`, with
    /// `a.csv` and `b.csv` listed.
    fn behind(holds: &str) -> commands::PackagePageData {
        let mut data = page_data();
        data.header.state = crate::kit::PackageState::Behind;
        data.context.revision.hash = holds.to_string();
        let entry = |path: &str| commands::EntryData {
            filename: path.to_string(),
            size: 7,
            status: "pristine".to_string(),
            junky_pattern: None,
            ignored_by: None,
            namespace: "team/dataset".try_into().unwrap(),
        };
        data.files = commands::FilesData::Listed(commands::EntryList {
            entries: vec![entry("a.csv"), entry("b.csv")],
            counts: commands::EntryCounts {
                all: 2,
                ..commands::EntryCounts::default()
            },
            total: 2,
            truncated: false,
        });
        data
    }

    fn adds(n: usize) -> commands::PullPreview {
        commands::PullPreview {
            outcome: commands::PullOutcome::CleanUpdate,
            added: (0..n).map(|i| format!("new/{i}.csv")).collect(),
            changed: Vec::new(),
            removed: Vec::new(),
            latest_hash: Some("feedbeef".to_string()),
        }
    }

    /// The page as `/installed-package` mounts it, under `ByDesign`'s
    /// `Suspense`, reading `PAGE` and checking through `PULLS`.
    async fn suspended_screen(page: commands::PackagePageData) -> web_sys::Element {
        crate::test_support::unmount_earlier();
        READS.with(|r| r.set(0));
        PULLS_ASKED.with(|n| n.set(0));
        PAGE.with(|p| *p.borrow_mut() = Some(page));
        go_to(PLAIN);
        let el = mount(move || {
            view! {
                <Router>
                    <Routes fallback=|| view! { "no route" }>
                        <Route
                            path=path!("/installed-package")
                            view=move || {
                                view! {
                                    <Suspense fallback=|| view! { <p data-fallback>"loading"</p> }>
                                        <PackageScreen
                                            read=scripted_read
                                            pull=scripted_pull
                                            resolving=ResolveCommands::app()
                                            revision_message=mismatch_band::app_revision_message
                                        />
                                    </Suspense>
                                }
                            }
                        />
                    </Routes>
                </Router>
            }
        });
        sleep_ms(80).await;
        el
    }

    /// The page reads its package again, as the watcher's news does.
    async fn re_read(el: &web_sys::Element, page: commands::PackagePageData) {
        PAGE.with(|p| *p.borrow_mut() = Some(page));
        button_saying(el, "Refresh").click();
        sleep_ms(30).await;
    }

    fn text(el: &web_sys::Element) -> String {
        el.text_content().unwrap_or_default()
    }

    /// The header's summary trigger, `· N file changes`.
    fn summary(el: &web_sys::Element) -> Option<String> {
        el.query_selector("[aria-controls][aria-expanded]:not([aria-haspopup])")
            .unwrap()
            .filter(|b| b.text_content().unwrap_or_default().contains("file change"))
            .and_then(|b| b.text_content())
    }

    fn no_fallback(el: &web_sys::Element) {
        assert!(
            el.query_selector("[data-fallback]").unwrap().is_none(),
            "the loading frame came back; markup was {}",
            el.inner_html()
        );
    }

    /// The watcher's news re-reads the page, which runs the check again; the
    /// answer for the same package and revision stays up while it runs, so
    /// the summary does not flash to `checking…`, and the page stays.
    #[wasm_bindgen_test]
    async fn a_re_read_with_the_same_newer_revision_keeps_the_answer() {
        script(vec![(0, Ok(adds(3))), (150, Ok(adds(3)))]);
        let el = suspended_screen(behind("aaa")).await;
        assert_eq!(
            summary(&el).as_deref(),
            Some("3 file changes"),
            "markup was {}",
            el.inner_html()
        );

        re_read(&el, behind("aaa")).await;
        no_fallback(&el);
        assert_eq!(READS.with(std::cell::Cell::get), 2, "the page re-read");
        assert_eq!(pulls_asked(), 2, "and checked again");
        assert_eq!(summary(&el).as_deref(), Some("3 file changes"));
        assert!(!text(&el).contains("checking"), "no flash: {}", text(&el));

        sleep_ms(200).await;
        no_fallback(&el);
        assert_eq!(summary(&el).as_deref(), Some("3 file changes"));
    }

    /// Get latest finishing leaves the package settled, and the read that
    /// says so drops the summary. A read that finds a newer revision again
    /// after it never paints the count from before: that answer was a
    /// difference from the revision this copy no longer holds.
    #[wasm_bindgen_test]
    async fn getting_the_latest_drops_the_summary() {
        script(vec![(0, Ok(adds(3)))]);
        let el = suspended_screen(behind("aaa")).await;
        assert_eq!(summary(&el).as_deref(), Some("3 file changes"));

        let mut settled = page_data();
        settled.context.revision.hash = "bbb".to_string();
        re_read(&el, settled).await;
        assert_eq!(summary(&el), None, "markup was {}", el.inner_html());
        assert!(!text(&el).contains("checking"));
        assert!(!text(&el).contains("Newer revision available"));

        // Behind again after the settled read: a fresh check.
        script(vec![(120, Ok(adds(1)))]);
        re_read(&el, behind("ccc")).await;
        assert_eq!(summary(&el), None);
        assert!(
            text(&el).contains("checking\u{2026}"),
            "markup was {}",
            el.inner_html()
        );
        sleep_ms(150).await;
        assert_eq!(summary(&el).as_deref(), Some("1 file change"));

        // Behind again straight after a Get latest, with no settled read
        // between: the count from before is not painted while the check runs.
        script(vec![(120, Ok(adds(2)))]);
        re_read(&el, behind("ddd")).await;
        assert_eq!(summary(&el), None, "the old count is not painted");
        assert!(
            text(&el).contains("checking\u{2026}"),
            "markup was {}",
            el.inner_html()
        );
        sleep_ms(150).await;
        assert_eq!(summary(&el).as_deref(), Some("2 file changes"));
    }

    /// A local edit can make a conflict while the newer revision stays the
    /// same: the check after the watcher's re-read finds it, the header
    /// resolves to the conflict state, offering Publish, and the list marks
    /// the conflicting row.
    #[wasm_bindgen_test]
    async fn a_conflict_found_on_a_re_read_switches_to_the_conflict_state() {
        let changes = |outcome| {
            Ok(commands::PullPreview {
                outcome,
                added: Vec::new(),
                changed: vec!["a.csv".to_string()],
                removed: Vec::new(),
                latest_hash: Some("feedbeef".to_string()),
            })
        };
        script(vec![
            (0, changes(commands::PullOutcome::CleanUpdate)),
            (
                0,
                changes(commands::PullOutcome::Blocked {
                    conflicts: vec!["a.csv".to_string()],
                }),
            ),
        ]);
        let el = suspended_screen(behind("aaa")).await;
        element_saying(&el, "Newer revision available");
        assert!(!row_marked(&el, "a.csv"));

        re_read(&el, behind("aaa")).await;
        sleep_ms(30).await;
        no_fallback(&el);
        element_saying(&el, "conflict in 1 file");
        button_saying(&el, "Publish");
        assert_eq!(summary(&el).as_deref(), Some("1 file change"));
        assert!(row_marked(&el, "a.csv"), "the conflicting row is marked");
        assert!(!row_marked(&el, "b.csv"));
        let sentence = el
            .query_selector(&format!("#{}", crate::kit::DIFFERS_ID))
            .unwrap()
            .expect("the sentence a marked row names");
        assert!(
            sentence
                .text_content()
                .unwrap_or_default()
                .contains("1 of them conflicts with yours. Publish your changes, then resolve it."),
        );
    }

    /// A local edit can appear after a check found the update clean, so Get
    /// latest pauses on a conflict and the re-read records it. The answer
    /// kept while the check runs again is about the read before: the header
    /// takes the recorded conflict at once, and the list marks its file,
    /// even if the new check never answers.
    #[wasm_bindgen_test]
    async fn a_kept_answer_does_not_hide_a_fresh_conflict() {
        script(vec![(
            0,
            Ok(commands::PullPreview {
                outcome: commands::PullOutcome::CleanUpdate,
                added: Vec::new(),
                changed: vec!["a.csv".to_string()],
                removed: Vec::new(),
                latest_hash: Some("feedbeef".to_string()),
            }),
        )]);
        let el = suspended_screen(behind("aaa")).await;
        element_saying(&el, "Newer revision available");
        assert!(!row_marked(&el, "a.csv"));

        // The rerun stays pending: nothing more is scripted.
        let mut recorded = behind("aaa");
        recorded.header.state = crate::kit::PackageState::PullConflict {
            files: vec!["a.csv".to_string()],
        };
        re_read(&el, recorded).await;
        no_fallback(&el);
        assert_eq!(pulls_asked(), 2, "the check runs again");
        assert_eq!(summary(&el).as_deref(), Some("1 file change"), "kept");
        element_saying(&el, "conflict in 1 file");
        button_saying(&el, "Publish");
        assert!(row_marked(&el, "a.csv"), "the recorded conflict's row");
        assert!(!row_marked(&el, "b.csv"));
    }

    /// The other way round: a kept conflict answer does not paint a conflict
    /// over a re-read that says only Behind while its rerun is out.
    #[wasm_bindgen_test]
    async fn a_kept_conflict_does_not_override_a_fresh_read() {
        script(vec![(
            0,
            Ok(commands::PullPreview {
                outcome: commands::PullOutcome::Blocked {
                    conflicts: vec!["a.csv".to_string()],
                },
                added: Vec::new(),
                changed: vec!["a.csv".to_string()],
                removed: Vec::new(),
                latest_hash: Some("feedbeef".to_string()),
            }),
        )]);
        let el = suspended_screen(behind("aaa")).await;
        element_saying(&el, "conflict in 1 file");
        assert!(row_marked(&el, "a.csv"));

        re_read(&el, behind("aaa")).await;
        no_fallback(&el);
        assert_eq!(pulls_asked(), 2, "the check runs again");
        element_saying(&el, "Newer revision available");
        button_saying(&el, "Get latest");
        assert!(
            !text(&el).contains("conflict in"),
            "markup was {}",
            el.inner_html()
        );
        assert!(!row_marked(&el, "a.csv"));
    }

    /// A check that fails says so and offers Try again; Get latest stays
    /// usable, since the real pull checks again under the lock.
    #[wasm_bindgen_test]
    async fn a_failed_check_offers_try_again() {
        script(vec![(0, Err("offline".to_string())), (0, Ok(adds(2)))]);
        let el = suspended_screen(behind("aaa")).await;
        assert!(
            text(&el).contains("couldn't check"),
            "markup was {}",
            el.inner_html()
        );
        assert!(!button_saying(&el, "Get latest").disabled());

        button_saying(&el, "Try again").click();
        sleep_ms(30).await;
        assert_eq!(summary(&el).as_deref(), Some("2 file changes"));
        assert!(!text(&el).contains("couldn't check"));
        assert_eq!(pulls_asked(), 2);
    }

    /// Two re-reads in a row: the first one's check answers last, and is
    /// dropped, because the second replaced it.
    #[wasm_bindgen_test]
    async fn a_late_answer_from_a_replaced_check_is_dropped() {
        script(vec![(0, Ok(adds(3))), (200, Ok(adds(5))), (0, Ok(adds(1)))]);
        let el = suspended_screen(behind("aaa")).await;
        assert_eq!(summary(&el).as_deref(), Some("3 file changes"));

        re_read(&el, behind("aaa")).await;
        re_read(&el, behind("aaa")).await;
        assert_eq!(pulls_asked(), 3);
        assert_eq!(summary(&el).as_deref(), Some("1 file change"));

        sleep_ms(250).await;
        assert_eq!(
            summary(&el).as_deref(),
            Some("1 file change"),
            "the replaced check's late answer did not land"
        );
    }

    /// The summary's trigger, to click, and whether its popover is open, by
    /// the trigger's word and the surface the platform shows.
    fn popover_open(el: &web_sys::Element) -> bool {
        let trigger = el
            .query_selector("[aria-controls][aria-expanded]:not([aria-haspopup])")
            .unwrap()
            .expect("the summary's trigger");
        let surface = el
            .query_selector("[aria-label='Files coming with the newer revision']")
            .unwrap()
            .expect("the summary's popover");
        let said = trigger.get_attribute("aria-expanded").as_deref() == Some("true");
        let shown = surface.matches(":popover-open").unwrap_or(false);
        assert_eq!(said, shown, "the trigger and the surface disagree");
        said
    }

    fn pin(el: &web_sys::Element) {
        el.query_selector("[aria-controls][aria-expanded]:not([aria-haspopup])")
            .unwrap()
            .expect("the summary's trigger")
            .unchecked_into::<web_sys::HtmlElement>()
            .click();
    }

    /// A popover the reader pinned stays open through the watcher's re-read,
    /// even one whose data differs and so draws the page's body again, while
    /// the check finds the same files coming.
    #[wasm_bindgen_test]
    async fn a_pinned_popover_stays_open_across_a_re_read() {
        script(vec![(0, Ok(adds(3))), (0, Ok(adds(3)))]);
        let el = suspended_screen(behind("aaa")).await;
        assert_eq!(summary(&el).as_deref(), Some("3 file changes"));
        pin(&el);
        sleep_ms(30).await;
        assert!(popover_open(&el), "the click pins it");

        let mut edited = behind("aaa");
        if let commands::FilesData::Listed(list) = &mut edited.files {
            list.entries[0].size = 9;
        }
        re_read(&el, edited).await;
        sleep_ms(30).await;
        no_fallback(&el);
        assert_eq!(pulls_asked(), 2, "the re-read checked again");
        assert_eq!(summary(&el).as_deref(), Some("3 file changes"));
        assert!(popover_open(&el), "the re-read closed a pinned popover");
    }

    /// The check moving the header to the conflict state draws the header
    /// again; a pinned popover over the same file stays open.
    #[wasm_bindgen_test]
    async fn a_pinned_popover_stays_open_when_the_header_is_drawn_again() {
        let changes = |outcome| {
            Ok(commands::PullPreview {
                outcome,
                added: Vec::new(),
                changed: vec!["a.csv".to_string()],
                removed: Vec::new(),
                latest_hash: Some("feedbeef".to_string()),
            })
        };
        script(vec![
            (0, changes(commands::PullOutcome::CleanUpdate)),
            (
                0,
                changes(commands::PullOutcome::Blocked {
                    conflicts: vec!["a.csv".to_string()],
                }),
            ),
        ]);
        let el = suspended_screen(behind("aaa")).await;
        pin(&el);
        sleep_ms(30).await;
        assert!(popover_open(&el), "the click pins it");

        re_read(&el, behind("aaa")).await;
        sleep_ms(30).await;
        element_saying(&el, "conflict in 1 file");
        assert!(popover_open(&el), "drawing the header again closed it");
    }

    /// A pin belongs to one package: another package's summary starts closed.
    #[wasm_bindgen_test]
    async fn a_pin_does_not_follow_to_another_package() {
        script(vec![(0, Ok(adds(3))), (0, Ok(adds(2)))]);
        let el = suspended_screen(behind("aaa")).await;
        pin(&el);
        sleep_ms(30).await;
        assert!(popover_open(&el));

        let mut other = behind("bbb");
        other.header.namespace = "team/other".try_into().unwrap();
        PAGE.with(|p| *p.borrow_mut() = Some(other));
        move_to("/installed-package?namespace=team%2Fother");
        sleep_ms(80).await;
        assert_eq!(summary(&el).as_deref(), Some("2 file changes"));
        assert!(!popover_open(&el), "the pin followed to another package");
    }
}
