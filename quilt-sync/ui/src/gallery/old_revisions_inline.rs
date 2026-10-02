//! Remove old revisions from the "Revisions you have" popover — the chosen
//! design, staged with stub data.
//!
//! # Where it lives
//!
//! In the "Revisions you have" surface the context pane already opens, at the
//! pane's 280px. The list is the thing a reader is looking at when they wonder
//! why there are six of them, so the removal sits on the rows.
//!
//! # Every removal is confirmed by the kit's `ConfirmDialog`
//!
//! The footer's "Remove N older" and each row's trash open the same dialog,
//! whose sentence says what goes and what it frees. Cancel is first and the
//! verb is the Danger `Remove`, so the kit's rule — only `ConfirmDialog` draws a
//! Danger button — holds without an exception.
//!
//! **The dialog lives outside the popover.** The popover closes on light
//! dismiss — any scroll or resize, as well as `showModal()` hiding every `auto`
//! popover that is not the dialog's ancestor. A dialog mounted inside it would
//! be unmounted mid-question. So the dialog and the question it asks belong to
//! the pane, and the popover may close under it: Remove still runs, the
//! notification reports it, and the popover, reopened, shows the list as it
//! stands — every remove button disabled while the removal runs.
//!
//! # The dialog closes on Remove; the work shows elsewhere
//!
//! The verb's action starts the removal and answers `Ok` at once, unlike the
//! header's commands, which hold the dialog until they settle. A sweep can take a
//! while, and the owner asked for its progress in the appbar and for the remove
//! buttons to refuse meanwhile — neither readable behind a modal. While it runs,
//! the appbar's activity line says "Removing 4 old revisions of user/plate-07…",
//! and every remove button is disabled, as it is while the package syncs.
//!
//! # The result is a notification from the stack
//!
//! The toast stack's, not the package page's outcome band. The band sits in the
//! flow under the appbar and pushes the page down, and an open popover is in the
//! top layer: it would stay where its trigger used to be. The stack floats, it
//! outlives the page — a sweep the reader navigated away from still reports —
//! and it is how autopull already reports a finished pull (`pull_toast.rs`). The
//! gallery cannot mount `ToastStack`, which reads the backend on mount, so the
//! card here is a copy of its markup and its v2 styles.
//!
//! # Sizes are what removing frees
//!
//! Never a revision's full size: only objects no other revision or package uses.
//! A row's figure is what it frees *alone, now* — so removing "Initial upload"
//! makes "Add Caihong folder-upload note" free more, because objects the two
//! shared are now its alone. The footer's figure is computed for the set, which
//! is why it is larger than the rows added up.
//!
//! # Built from the kit
//!
//! The trash is `icons::trash()`; the figure or the tag is `RevisionRow`'s
//! detail after the time (`3 days ago · frees 1.2 MB`), and the trash its
//! trailing action. The sync and the removal each set their own
//! `ActivityKind`'s slot of the line, so the end of one never wipes the other.
//! The app's popover (`pages/installed_package_v2/old_revisions.rs`) draws the
//! same surface over the backend's answer.

use leptos::context::Provider;
use leptos::prelude::*;

use crate::Cell;
use crate::Scene;
use crate::gallery::forms::after_a_beat;
use crate::kit::Activities;
use crate::kit::Activity;
use crate::kit::ActivityKind;
use crate::kit::Align;
use crate::kit::AnchoredOverlay;
use crate::kit::Button;
use crate::kit::ButtonVariant;
use crate::kit::Card;
use crate::kit::CatalogLink;
use crate::kit::ConfirmDialog;
use crate::kit::IconButton;
use crate::kit::IconButtonVariant;
use crate::kit::PageLayout;
use crate::kit::PaneSection;
use crate::kit::RevisionRow;
use crate::kit::Submit;
use crate::kit::icons;

const NAMESPACE: &str = "user/plate-07";

const HOUR: f64 = 3_600_000.0;
const DAY: f64 = 24.0 * HOUR;
const MINUTE: f64 = HOUR / 60.0;

/// How long the stub removal runs, and the stub sync beside it.
const REMOVING_MS: i32 = 2500;
const SYNCING_MS: i32 = 4000;

fn ago(ms: f64) -> f64 {
    js_sys::Date::now() - ms
}

/// One revision this computer has, from the shared fixture.
struct Rev {
    message: &'static str,
    obtained: f64,
    /// The catalog address's hash; `None` for the one not pushed.
    hash: Option<&'static str>,
    /// Why it is kept: the short tag on the row, and the sentence behind it in
    /// `title`. `None` is removable.
    kept: Option<(&'static str, &'static str)>,
    /// Tenths of a MB freed by objects only this revision uses. Integers, so
    /// the arithmetic on sets never rounds twice.
    own: u32,
}

/// Newest obtained first. #3 was made before #4 and re-fetched a day ago, which
/// is why it sits above it.
const REVS: [Rev; 6] = [
    Rev {
        message: "Normalize well IDs",
        obtained: 20.0 * MINUTE,
        hash: None,
        kept: Some((
            "current · not pushed",
            "Kept: your files are at this revision, and it has not been pushed.",
        )),
        own: 0,
    },
    Rev {
        message: "Re-run plate 7 with the corrected layout",
        obtained: 2.0 * HOUR,
        hash: Some("c41d8f"),
        kept: Some((
            "latest · base",
            "Kept: the latest published revision, and the one your changes are based on.",
        )),
        own: 0,
    },
    Rev {
        message: "Add plate 6 controls",
        obtained: DAY,
        hash: Some("7be0c2"),
        kept: None,
        own: 2,
    },
    Rev {
        message: "Add Caihong folder-upload note",
        obtained: 3.0 * DAY,
        hash: Some("b5e013"),
        kept: None,
        own: 12,
    },
    Rev {
        message: "Initial upload",
        obtained: 12.0 * DAY,
        hash: Some("06e3ad"),
        kept: None,
        own: 48,
    },
    Rev {
        message: "",
        obtained: 20.0 * DAY,
        hash: Some("9a2b71"),
        kept: None,
        own: 0,
    },
];

const ALL: [usize; 6] = [0, 1, 2, 3, 4, 5];
const KEPT: [usize; 2] = [0, 1];
const OLDER: [usize; 4] = [2, 3, 4, 5];

/// Objects shared by exactly two revisions and nothing else, in tenths of a MB.
/// Freed only once neither of the pair is left, which is how the set of 3–6
/// frees 6.9 MB against rows that add up to 6.2.
const SHARED: [(usize, usize, u32); 2] = [(2, 4, 1), (3, 4, 6)];

/// What removing `set` frees, given what is still here. The stand-in for the
/// backend's answer — a real build asks, because only the store knows which
/// objects are referenced from where.
fn freed(set: &[usize], remaining: &[usize]) -> u32 {
    let gone = |i: usize| set.contains(&i) || !remaining.contains(&i);
    let own: u32 = set.iter().map(|&i| REVS[i].own).sum();
    let shared: u32 = SHARED
        .iter()
        .filter(|&&(a, b, _)| gone(a) && gone(b) && (set.contains(&a) || set.contains(&b)))
        .map(|&(_, _, size)| size)
        .sum();
    own + shared
}

/// A size on a row or the footer, honestly: most old revisions free little or
/// nothing, and `0.2 MB` would claim a precision nobody has a use for.
fn size(tenths: u32) -> String {
    match tenths {
        0 => "nothing".to_string(),
        1..=9 => "< 1 MB".to_string(),
        _ => format!("{}.{} MB", tenths / 10, tenths % 10),
    }
}

/// The same figure in a sentence, where "frees nothing" and "< 1" read badly.
fn spelled(tenths: u32) -> String {
    match tenths {
        0 => "no space".to_string(),
        1..=9 => "less than 1 MB".to_string(),
        _ => size(tenths),
    }
}

fn plural(count: usize, one: &str, many: &str) -> String {
    if count == 1 {
        format!("1 {one}")
    } else {
        format!("{count} {many}")
    }
}

/// A row's name in a sentence: its message in quotes, or what it lacks.
fn named(i: usize) -> String {
    match REVS[i].message {
        "" => "the revision with no message".to_string(),
        message => format!("\u{201c}{message}\u{201d}"),
    }
}

fn catalog(hash: &str) -> CatalogLink {
    CatalogLink::new(
        format!("https://quilt-lab.example/b/quilt-lab-plates/packages/{NAMESPACE}/tree/{hash}/"),
        Callback::new(|_url: String| ()),
    )
}

/// What the confirmation asks: the set, and the words for its title and its
/// one sentence.
#[derive(Clone, Debug, PartialEq)]
struct Ask {
    set: Vec<usize>,
    title: &'static str,
    consequence: String,
}

impl Ask {
    /// The footer's: every removable row, freed as a set.
    fn older(set: &[usize], remaining: &[usize]) -> Self {
        Self {
            set: set.to_vec(),
            title: "Remove old revisions",
            consequence: format!(
                "Remove {}? This frees {}.",
                plural(set.len(), "older revision", "older revisions"),
                spelled(freed(set, remaining)),
            ),
        }
    }

    /// A row's: that revision alone.
    fn one(i: usize, remaining: &[usize]) -> Self {
        Self {
            set: vec![i],
            title: "Remove a revision",
            consequence: format!(
                "Remove {}? This frees {}.",
                named(i),
                spelled(freed(&[i], remaining)),
            ),
        }
    }
}

/// One surface's state, with the page around it: its own activity line and its
/// own notification, so every cell is live from the point it starts at and no
/// two cells share a bar.
#[derive(Clone, Copy)]
struct Flow {
    remaining: RwSignal<Vec<usize>>,
    /// The set being removed, while it is.
    removing: RwSignal<Option<Vec<usize>>>,
    /// The package is locked by a sync. Every remove button refuses.
    syncing: RwSignal<bool>,
    activities: Activities,
    /// The notification's sentence, once a removal has ended.
    toast: RwSignal<Option<String>>,
    /// The confirmation: what it asks, and whether it is open.
    ask: RwSignal<Option<Ask>>,
    open: RwSignal<bool>,
    /// Bumped by each removal and by Reset. A stub removal that wakes to a
    /// newer number was overtaken, and does nothing.
    removal_run: StoredValue<u64>,
    /// The same for the stub sync.
    sync_run: StoredValue<u64>,
}

impl Flow {
    fn new(remaining: &[usize]) -> Self {
        Self {
            remaining: RwSignal::new(remaining.to_vec()),
            removing: RwSignal::new(None),
            syncing: RwSignal::new(false),
            activities: Activities::new(),
            toast: RwSignal::new(None),
            ask: RwSignal::new(None),
            open: RwSignal::new(false),
            removal_run: StoredValue::new(0),
            sync_run: StoredValue::new(0),
        }
    }

    /// A new run of `counter`, whose number a task holds through its delay.
    fn bump(counter: StoredValue<u64>) -> u64 {
        counter.update_value(|run| *run += 1);
        counter.get_value()
    }

    /// Held mid-removal of `set`, for a cell to show still.
    fn removing(remaining: &[usize], set: &[usize]) -> Self {
        let flow = Self::new(remaining);
        flow.removing.set(Some(set.to_vec()));
        flow.say();
        flow
    }

    /// Held mid-sync, for a cell to show still.
    fn syncing(remaining: &[usize]) -> Self {
        let flow = Self::new(remaining);
        flow.syncing.set(true);
        flow.say();
        flow
    }

    fn removable(self) -> Vec<usize> {
        self.remaining
            .get()
            .into_iter()
            .filter(|&i| REVS[i].kept.is_none())
            .collect()
    }

    /// Every remove button's `disabled`: a sync holds the package's lock, and a
    /// removal already running owns the store.
    fn blocked(self) -> bool {
        self.syncing.get() || self.removing.get().is_some()
    }

    fn progress(count: usize) -> String {
        format!(
            "Removing {} of {NAMESPACE}\u{2026}",
            plural(count, "old revision", "old revisions")
        )
    }

    /// The appbar's line, drawn from what is running now: a sync's entry and a
    /// removal's each go to their own kind's slot, so the end of one never
    /// wipes the other's.
    fn say(self) {
        let activity = |kind, label| Activity {
            kind,
            label,
            package: Some(NAMESPACE.to_string()),
        };
        let syncing = self.syncing.get_untracked().then(|| {
            activity(
                ActivityKind::Autopull,
                format!("Getting latest for {NAMESPACE}\u{2026}"),
            )
        });
        let removing = self
            .removing
            .get_untracked()
            .map(|set| activity(ActivityKind::RemoveRevisions, Self::progress(set.len())));
        self.activities
            .set(ActivityKind::Autopull, syncing.into_iter().collect());
        self.activities.set(
            ActivityKind::RemoveRevisions,
            removing.into_iter().collect(),
        );
    }

    fn confirm(self, ask: Ask) {
        self.ask.set(Some(ask));
        self.open.set(true);
    }

    /// Starts removing `set` and returns, as the verb's action does: the dialog
    /// closes, the line takes over, and the notification ends it.
    fn start(self, set: Vec<usize>) {
        let run = Self::bump(self.removal_run);
        self.removing.set(Some(set.clone()));
        self.toast.set(None);
        self.say();
        leptos::task::spawn_local(async move {
            after_a_beat(REMOVING_MS).await;
            if self.removal_run.get_value() != run {
                return;
            }
            let freed = freed(&set, &self.remaining.get_untracked());
            self.remaining
                .update(|rows| rows.retain(|i| !set.contains(i)));
            self.removing.set(None);
            self.say();
            self.toast.set(Some(format!(
                "Removed {} of {NAMESPACE} · freed {}",
                plural(set.len(), "old revision", "old revisions"),
                spelled(freed),
            )));
        });
    }

    /// The stub sync: the lock for a few seconds, and autopull's own line.
    fn sync(self) {
        let run = Self::bump(self.sync_run);
        self.syncing.set(true);
        self.say();
        leptos::task::spawn_local(async move {
            after_a_beat(SYNCING_MS).await;
            if self.sync_run.get_value() != run {
                return;
            }
            self.syncing.set(false);
            self.say();
        });
    }

    /// Back to the start. Bumping both runs is what makes a removal or a sync
    /// still in its delay give up instead of landing on the fresh list.
    fn reset(self) {
        Self::bump(self.removal_run);
        Self::bump(self.sync_run);
        self.remaining.set(ALL.to_vec());
        self.removing.set(None);
        self.syncing.set(false);
        self.toast.set(None);
        self.say();
    }
}

/// One row: the kit's revision row, its detail after the time saying what
/// removing it frees — or why it cannot be removed — and the trash after its
/// catalog icon.
fn row(i: usize, flow: Flow) -> AnyView {
    let rev = &REVS[i];
    if let Some((tag, why)) = rev.kept {
        // A kept row holds the trash's width empty, so every catalog icon in the
        // column lines up — except when the whole list is kept and no row has one.
        let holds = move || flow.removable().is_empty().then_some("display:none");
        let gap = view! { <span class="g-ori-gap" aria-hidden="true" style=holds></span> };
        return view! {
            <RevisionRow
                message=rev.message
                at=ago(rev.obtained)
                published=rev.hash.is_some()
                catalog=rev.hash.map(catalog)
                detail=tag
                detail_title=why
                trailing=gap.into_any()
            />
        }
        .into_any();
    }
    let label = format!("Remove {}", named(i));
    let trash = view! {
        <IconButton
            icon=icons::trash()
            aria_label=label
            variant=IconButtonVariant::Invisible
            disabled=Signal::derive(move || flow.blocked())
            on_click=move |_| flow.confirm(Ask::one(i, &flow.remaining.get_untracked()))
        />
    };
    // The figure is what it frees given what is left, so it is read once per
    // drawing of the list, which redraws when a removal lands.
    let words = format!(
        "frees {}",
        size(freed(&[i], &flow.remaining.get_untracked()))
    );
    view! {
        <RevisionRow
            message=rev.message
            at=ago(rev.obtained)
            published=rev.hash.is_some()
            catalog=rev.hash.map(catalog)
            detail=words
            trailing=trash.into_any()
        />
    }
    .into_any()
}

/// The footer. Absent when the list is the current revision alone: there is
/// nothing to say about removing, not even that nothing can be.
fn footer(flow: Flow) -> impl IntoView {
    move || {
        if flow.remaining.get() == [0] {
            return None;
        }
        let removable = flow.removable();
        let line = if flow.syncing.get() {
            Some(format!(
                "{NAMESPACE} is busy syncing \u{2014} try again in a moment"
            ))
        } else if removable.is_empty() {
            Some("Nothing to remove \u{2014} every revision here is in use".to_string())
        } else {
            None
        };
        let button = (!removable.is_empty()).then(|| {
            let count = removable.len();
            let total = size(freed(&removable, &flow.remaining.get_untracked()));
            // Spinning when the set it names is the one being removed; only
            // disabled when a row's removal or a sync holds it.
            let named = removable.clone();
            let loading =
                Signal::derive(move || flow.removing.get().as_deref() == Some(&named[..]));
            let disabled = Signal::derive(move || flow.blocked());
            let set = removable.clone();
            view! {
                <Button
                    loading=loading
                    disabled=disabled
                    on_click=move |_| {
                        flow.confirm(Ask::older(&set, &flow.remaining.get_untracked()));
                    }
                >
                    {format!("Remove {count} older · frees {total}")}
                </Button>
            }
        });
        Some(view! {
            <PaneSection>
                <div class="g-ori-footer">
                    {line.map(|line| view! { <p class="g-ori-muted">{line}</p> })}
                    {button}
                </div>
            </PaneSection>
        })
    }
}

/// The real confirmation, rebuilt for each question — `ConfirmDialog` takes its
/// words once. Mounted beside the surface, never inside the popover, so the
/// popover's light dismiss cannot take it away (see the module comment).
fn dialog(flow: Flow) -> impl IntoView {
    move || {
        flow.ask.get().map(|ask| {
            let Ask {
                set,
                title,
                consequence,
            } = ask;
            view! {
                <ConfirmDialog
                    open=flow.open
                    title=title
                    consequence=consequence
                    confirm=Submit::new(
                        "Remove",
                        move || {
                            let set = set.clone();
                            async move {
                                flow.start(set);
                                Ok(())
                            }
                        },
                    )
                />
            }
        })
    }
}

/// The surface's contents: the rows, then the footer, which `PaneSection`
/// rules off.
fn body(flow: Flow) -> AnyView {
    view! {
        <div class="g-ori-body">
            <PaneSection>
                <div class="g-ori-rows">
                    {move || {
                        flow.remaining.get().into_iter().map(|i| row(i, flow)).collect_view()
                    }}
                </div>
            </PaneSection>
            {footer(flow)}
        </div>
    }
    .into_any()
}

/// The surface drawn in the page, for a cell to hold still. The same border,
/// padding and shadow the overlay's surface draws, so the stills read as the
/// popover they are copies of.
fn surface(flow: Flow) -> AnyView {
    view! {
        <div class="g-ori-surface">{body(flow)}</div>
        {dialog(flow)}
    }
    .into_any()
}

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

/// The appbar, with this flow's activity line in it.
fn appbar(flow: Flow) -> AnyView {
    view! {
        <div class="g-window g-window--bar">
            <Provider value=flow.activities>
                <PageLayout heading="Package" actions=appbar_actions()>
                    ""
                </PageLayout>
            </Provider>
        </div>
    }
    .into_any()
}

/// The notification, as `ToastStack` draws a `Success` card: the sentence and
/// a dismiss. A copy, because the stack reads the backend on mount.
fn toast(flow: Flow) -> impl IntoView {
    move || {
        flow.toast.get().map(|said| {
            view! {
                <div class="g-ori-toast" role="status">
                    <span class="g-ori-toast__body">{said}</span>
                    <button
                        class="g-ori-toast__close"
                        type="button"
                        aria-label="Dismiss"
                        on:click=move |_| flow.toast.set(None)
                    >
                        "\u{2715}"
                    </button>
                </div>
            }
        })
    }
}

/// The page around a still surface: the appbar on top, the notification where
/// the stack hangs, and the surface where the pane's popover opens.
fn staged(flow: Flow) -> AnyView {
    view! {
        <div class="g-ori-stage">
            {appbar(flow)}
            <div class="g-ori-toasts">{toast(flow)}</div>
            <div class="g-ori-page g-ori-page--still">
                <div class="g-ori-files">"the file list"</div>
                {surface(flow)}
            </div>
        </div>
    }
    .into_any()
}

/// The pane with the real trigger and the real popover, beside a stand-in for
/// the file list — the surface opens leftwards over it, as on the page. The
/// confirmation is the pane's, outside the popover.
fn pane(flow: Flow) -> AnyView {
    let open = RwSignal::new(false);
    let trigger = move |surface_id: String| {
        view! {
            <Button
                on_click=move |_| open.update(|o| *o = !*o)
                aria_expanded=open
                aria_controls=surface_id
            >
                {move || format!("Revisions you have ({})", flow.remaining.get().len())}
            </Button>
        }
        .into_any()
    };
    view! {
        <div class="g-ori-page">
            <div class="g-ori-files">"the file list"</div>
            <aside aria-label="About this package" class="g-ori-pane">
                <Card>
                    <PaneSection label="Revision">
                        <RevisionRow message=REVS[0].message at=ago(REVS[0].obtained) />
                        <AnchoredOverlay
                            trigger=trigger
                            open=open
                            aria_label="Revisions you have"
                            align=Align::End
                        >
                            {body(flow)}
                        </AnchoredOverlay>
                    </PaneSection>
                </Card>
            </aside>
            {dialog(flow)}
        </div>
    }
    .into_any()
}

/// The whole flow, live: the appbar, the stack's slot and the pane, with a
/// stub sync to start and a reset.
///
/// The slot keeps its height when empty. A notification arriving in the flow
/// would push the pane down under a popover that stays put — the very thing
/// the real stack floats to avoid.
fn live() -> AnyView {
    let flow = Flow::new(&ALL);
    view! {
        <div class="g-ori-stage">
            <div class="g-inline">
                // Refused while a removal runs as well: it owns the store, as
                // the sync would hold the package's lock.
                <Button
                    disabled=Signal::derive(move || flow.blocked())
                    on_click=move |_| flow.sync()
                >
                    "Start a sync (4 s)"
                </Button>
                <Button on_click=move |_| flow.reset()>"Reset"</Button>
            </div>
            {appbar(flow)}
            <div class="g-ori-toasts g-ori-toasts--held">{toast(flow)}</div>
            {pane(flow)}
        </div>
    }
    .into_any()
}

/// The confirmation inline, at the modal's width, beside a button that opens
/// the real one — a live modal blocks the gallery, so the copy is what a
/// screenshot reads. The same pieces `Dialog` and `ConfirmDialog` draw.
fn confirmation(ask: Ask) -> AnyView {
    let live = RwSignal::new(false);
    let Ask {
        title, consequence, ..
    } = ask;
    view! {
        <div class="g-bars">
            <div class="g-ori-dialog">
                <h2 class="g-ori-dialog__title">{title}</h2>
                <p class="g-consequence">{consequence.clone()}</p>
                // The footer, in the dialog's own arrangement: right-aligned, Cancel first.
                <div class="g-inline g-inline--end g-ori-dialog__footer">
                    <Button on_click=move |_| ()>"Cancel"</Button>
                    <Button variant=ButtonVariant::Danger on_click=move |_| ()>"Remove"</Button>
                </div>
            </div>
            <div class="g-inline">
                <Button on_click=move |_| live.set(true)>"Open the real one"</Button>
            </div>
        </div>
        <ConfirmDialog
            open=live
            title=title
            consequence=consequence
            confirm=Submit::new(
                "Remove",
                || async {
                    after_a_beat(700).await;
                    Ok(())
                },
            )
        />
    }
    .into_any()
}

const NOTE: &str = "Removal where the list already is: the context pane's \"Revisions you have\" \
    surface, at the pane's width. A removable row ends in a trash icon and says what removing \
    it frees alone — often less than 1 MB or nothing, which is the truth about old revisions. \
    Protected rows say why in a muted tag; hover it for the sentence. \
    \
    The footer and every trash open the kit's ConfirmDialog, which says what goes and what it \
    frees, Cancel first and the Danger Remove last. The dialog belongs to the pane, not the \
    popover, so the popover closing under it never takes the question away. Remove closes \
    the dialog; while the removal runs the \
    appbar says so and every remove button refuses, as it does while the package syncs. The \
    end is a notification from the stack, and the list drops to what is left. \
    \
    The live cell walks it all with stub delays: open the surface, remove a row or the lot, \
    start a sync to watch the buttons refuse, and reset.";

#[component]
pub fn OldRevisionsInlineScene() -> impl IntoView {
    let remove_one = Ask::one(4, &ALL);
    view! {
        <Scene title="Remove old revisions" note=NOTE>
            <Cell full=true label="live — the whole flow, with a stub sync and a reset">
                {live()}
            </Cell>
            <Cell wide=true label="idle — four removable, two protected">
                {surface(Flow::new(&ALL))}
            </Cell>
            <Cell wide=true label="confirm from the footer">
                {confirmation(Ask::older(&OLDER, &ALL))}
            </Cell>
            <Cell wide=true label="confirm from a row — Initial upload">
                {confirmation(remove_one)}
            </Cell>
            <Cell wide=true label="confirm from a row that frees nothing">
                {confirmation(Ask::one(5, &ALL))}
            </Cell>
            <Cell full=true label="removing — the appbar says so, every button refuses">
                {staged(Flow::removing(&ALL, &OLDER))}
            </Cell>
            <Cell full=true label="done — the notification, and the two kept rows">
                {
                    let flow = Flow::new(&KEPT);
                    flow.toast
                        .set(
                            Some(
                                format!("Removed 4 old revisions of {NAMESPACE} · freed 6.9 MB"),
                            ),
                        );
                    staged(flow)
                }
            </Cell>
            <Cell full=true label="busy syncing — autopull's line, every button refuses">
                {staged(Flow::syncing(&ALL))}
            </Cell>
            <Cell wide=true label="everything protected — nothing to remove">
                {surface(Flow::new(&KEPT))}
            </Cell>
            <Cell wide=true label="only the current revision — no footer, no icons">
                {surface(Flow::new(&[0]))}
            </Cell>
        </Scene>
    }
}
