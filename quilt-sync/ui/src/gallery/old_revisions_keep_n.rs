//! Option D · keep the last N automatically — an exploration for removing old revisions, not a shipped design.
//!
//! # What this option bets on
//!
//! Nobody picks rows. A per-package count — `Keep the last 3` — is a standing
//! policy, and after each pull the app removes the revisions this computer
//! obtained beyond it. There is no per-row removal UI anywhere; the one control
//! is the count, in Settings as a default and in the "Revisions you have"
//! popover as a per-package override.
//!
//! # The policy is visible before it acts
//!
//! A policy that deletes on its own owes the reader a preview. So the popover
//! marks the rows the next run will take — muted, `will be removed` — and
//! turning the setting on confirms a dry run first: how many revisions, across
//! how many packages, and about how much it frees *now*. Afterwards the result
//! arrives as a notification and Settings keeps one line about the last run.
//!
//! # N counts rows by when they were obtained, protected ones included
//!
//! Current, base, latest, the not-pushed chain and anything unpublished are
//! never removed, and when they fall inside the last N they use up places in
//! it. The count is over *obtained* order, not commit order, which has a
//! surprise in it: re-fetching an old revision moves it to the top and pushes
//! the next-oldest out. One cell draws exactly that.
//!
//! # What the kit lacks here
//!
//! `RevisionRow` has no trailing slot and no muted variant, so a marked row is
//! composed around it (`.g-ork-row`). `ActivityKind` has only `Autopull`; the
//! prune line borrows it. `ToggleRow` has no body slot for a dependent field,
//! so the `Keep the last` select sits under it in the card, indented to the
//! label.

use leptos::prelude::*;

use crate::Cell;
use crate::Scene;
use crate::gallery::forms::after_a_beat;
use crate::kit::Activities;
use crate::kit::Activity;
use crate::kit::ActivityKind;
use crate::kit::ActivityLine;
use crate::kit::Align;
use crate::kit::AnchoredOverlay;
use crate::kit::Banner;
use crate::kit::BannerVariant;
use crate::kit::Button;
use crate::kit::ButtonVariant;
use crate::kit::Card;
use crate::kit::ConfirmDialog;
use crate::kit::FormControl;
use crate::kit::Naming;
use crate::kit::PaneSection;
use crate::kit::RevisionRow;
use crate::kit::Select;
use crate::kit::Spinner;
use crate::kit::StateLabel;
use crate::kit::StateTone;
use crate::kit::Submit;
use crate::kit::ToggleRow;

/// The pane's fixed width, as `context_pane.rs` has it.
const PANE: &str = "width:280px";

const MINUTE: f64 = 60_000.0;
const HOUR: f64 = 60.0 * MINUTE;
const DAY: f64 = 24.0 * HOUR;

fn ago(ms: f64) -> f64 {
    js_sys::Date::now() - ms
}

/// The domain-wide default. Settings shows it and the override's first option
/// quotes it, so the two cannot disagree.
const DEFAULT_KEEP: usize = 3;

/// The dry run turning the setting on shows, with keep 3: plate-07 loses rows
/// 4–6 (3), plate-06 its 3, lab/assays its 1. Seven, not the nine the prompt
/// suggested — nine does not fit the shared fixture (see the report).
/// lab/imaging is busy, so the dry run leaves it out and says nothing it
/// cannot promise.
const DRY_RUN: &str =
    "With keep 3, removes 7 revisions across 3 packages now and frees about 29 MB.";

const HINT: &str = "Counts revisions this computer obtained, newest first. The current, base, \
    latest, not-pushed and unpublished revisions are always kept, and count toward N when they \
    fall inside it.";

const SUBLABEL: &str = "After each pull. Only published revisions go, so each can be obtained \
    again from the platform.";

/// One revision of the shared fixture, newest obtained first.
#[derive(Clone, Copy)]
struct Rev {
    message: &'static str,
    obtained: f64,
    published: bool,
    /// Why it is never removed, in the words the row shows. `None` for a row
    /// the count alone decides.
    guard: Option<&'static str>,
}

const fn rev(
    message: &'static str,
    obtained: f64,
    published: bool,
    guard: Option<&'static str>,
) -> Rev {
    Rev {
        message,
        obtained,
        published,
        guard,
    }
}

/// `user/plate-07`, as the shared brief draws it.
fn plate07() -> Vec<Rev> {
    vec![
        rev(
            "Normalize well IDs",
            20.0 * MINUTE,
            false,
            Some("current · not pushed"),
        ),
        rev(
            "Re-run plate 7 with the corrected layout",
            2.0 * HOUR,
            true,
            Some("latest · base"),
        ),
        rev("Add plate 6 controls", DAY, true, None),
        rev("Add Caihong folder-upload note", 3.0 * DAY, true, None),
        rev("Initial upload", 12.0 * DAY, true, None),
        rev("", 20.0 * DAY, true, None),
    ]
}

/// The same six after re-fetching "Initial upload": obtained just now, so it
/// is the newest — and "Add plate 6 controls" slides to fourth.
fn plate07_refetched() -> Vec<Rev> {
    let mut rows = plate07();
    let mut initial = rows.remove(4);
    initial.obtained = MINUTE;
    rows.insert(0, initial);
    rows
}

/// Only the current revision: nothing for a count to act on.
fn only_current() -> Vec<Rev> {
    vec![rev(
        "Normalize well IDs",
        20.0 * MINUTE,
        true,
        Some("current · latest · base"),
    )]
}

/// `team/notes`: two revisions, both protected.
fn team_notes() -> Vec<Rev> {
    vec![
        rev(
            "Draft the week's notes",
            HOUR,
            false,
            Some("current · not pushed"),
        ),
        rev("Week 38 notes", 4.0 * DAY, true, Some("latest · base")),
    ]
}

/// The override's options, in the order the select lists them.
fn keep_options() -> Vec<String> {
    vec![
        format!("Use default ({DEFAULT_KEEP})"),
        "Keep all".to_string(),
        "3".to_string(),
        "5".to_string(),
        "10".to_string(),
    ]
}

/// What an override option means: `None` keeps everything.
fn keep_of(choice: &str) -> Option<usize> {
    match choice {
        "Keep all" => None,
        other => other.parse().ok().or(Some(DEFAULT_KEEP)),
    }
}

/// What one row says about the policy. Position is by obtained order and
/// counts protected rows, so row `i` is inside N when `i < n`.
fn mark(i: usize, rev: Rev, keep: Option<usize>) -> AnyView {
    if let Some(guard) = rev.guard {
        return view! { <StateLabel tone=StateTone::Neutral>{guard}</StateLabel> }.into_any();
    }
    match keep {
        Some(n) if i >= n => {
            view! { <span class="g-ork-mark g-ork-mark--going">"will be removed"</span> }.into_any()
        }
        Some(n) => view! { <span class="g-ork-mark">{format!("kept · {} of {n}", i + 1)}</span> }
            .into_any(),
        None => view! { <span class="g-ork-mark">"kept"</span> }.into_any(),
    }
}

/// One row, with its mark at the end. A row that will go is muted as a
/// whole, so the column reads as "these stay, these go" at a glance.
fn row(i: usize, rev: Rev, keep: Option<usize>) -> AnyView {
    let going = rev.guard.is_none() && keep.is_some_and(|n| i >= n);
    view! {
        <div class=if going { "g-ork-row g-ork-row--going" } else { "g-ork-row" }>
            <div class="g-ork-row__rev">
                <RevisionRow
                    message=rev.message
                    at=ago(rev.obtained)
                    published=rev.published
                />
            </div>
            {mark(i, rev, keep)}
        </div>
    }
    .into_any()
}

/// The footer's one sentence: what the count will do next, derived from the
/// same rows so it cannot claim a number the column does not show.
fn footer_line(rows: &[Rev], keep: Option<usize>) -> String {
    let going = rows
        .iter()
        .enumerate()
        .filter(|(i, r)| r.guard.is_none() && keep.is_some_and(|n| *i >= n))
        .count();
    match (keep, going) {
        (None, _) => "Every revision stays until you choose a count.".to_string(),
        (Some(_), 0) => "Nothing to remove: every revision is kept.".to_string(),
        (Some(_), 1) => "1 revision goes after the next pull.".to_string(),
        (Some(_), n) => format!("{n} revisions go after the next pull."),
    }
}

/// The surface's body: the rows and, below them, the override.
///
/// The select drives the marks: pick `5` and only the oldest row is left
/// marked, `Keep all` and none are.
fn surface(rows: Vec<Rev>, choice: RwSignal<String>) -> AnyView {
    let keep = Signal::derive(move || keep_of(&choice.get()));
    let listed = rows.clone();
    view! {
        <div class="g-ork-surface">
            <PaneSection>
                {move || {
                    let keep = keep.get();
                    listed
                        .iter()
                        .enumerate()
                        .map(|(i, r)| row(i, *r, keep))
                        .collect_view()
                }}
            </PaneSection>
            <div class="g-ork-footer">
                <Select
                    naming=Naming::Prefix("Keep".to_string())
                    options=keep_options()
                    selected=choice
                />
                <p class="g-ork-footnote">{move || footer_line(&rows, keep.get())}</p>
            </div>
        </div>
    }
    .into_any()
}

/// The pane with the popover trigger, staged as `context_pane.rs` stages it:
/// last in a row, with the file list to its left, so the surface opens over
/// the list and not into empty gallery.
fn in_page(open: RwSignal<bool>, rows: Vec<Rev>, choice: RwSignal<String>) -> AnyView {
    let count = rows.len();
    let current = rows
        .iter()
        .copied()
        .find(|r| r.guard.is_some_and(|g| g.starts_with("current")))
        .unwrap_or(rows[0]);
    let body = surface(rows, choice);
    let trigger = move |surface_id: String| {
        view! {
            <Button
                on_click=move |_| open.update(|o| *o = !*o)
                aria_expanded=open
                aria_controls=surface_id
            >
                {format!("Revisions you have ({count})")}
            </Button>
        }
        .into_any()
    };
    view! {
        <div class="g-ork-page">
            <div class="g-ork-filelist">"the file list"</div>
            <aside aria-label="About this package" style=PANE>
                <Card>
                    <PaneSection label="Revision">
                        <RevisionRow message=current.message at=ago(current.obtained) />
                        <AnchoredOverlay
                            trigger=trigger
                            open=open
                            aria_label="Revisions you have"
                            align=Align::End
                        >
                            {body}
                        </AnchoredOverlay>
                    </PaneSection>
                </Card>
            </aside>
        </div>
    }
    .into_any()
}

/// The surface drawn flat, at the popover's width, so a state can be read and
/// screenshotted without a click.
fn flat(rows: Vec<Rev>, choice: &str) -> AnyView {
    view! { <div class="g-ork-flat">{surface(rows, RwSignal::new(choice.to_string()))}</div> }
        .into_any()
}

/// What Settings says under the toggle. One line about the last run, never a
/// history: the notification stack already said each result when it happened.
#[derive(Clone, Copy)]
enum LastRun {
    Ran(&'static str),
    Nothing,
    Working(&'static str),
    Skipped,
    Failed,
}

fn last_run(run: LastRun) -> AnyView {
    match run {
        LastRun::Ran(words) => view! { <p class="g-ork-note">{words}</p> }.into_any(),
        LastRun::Nothing => view! {
            <p class="g-ork-note">
                "Last run 10 min ago: nothing to remove — every package is within its count."
            </p>
        }
        .into_any(),
        LastRun::Working(package) => view! {
            <p class="g-ork-note">
                <Spinner />
                {format!("Removing old revisions of {package}…")}
            </p>
        }
        .into_any(),
        LastRun::Skipped => view! {
            <p class="g-ork-note">"Last run 5 min ago: removed 7 revisions, freed 29 MB."</p>
            <Banner variant=BannerVariant::Warning>
                "lab/imaging was busy syncing and was skipped; it will be tried on the next run."
            </Banner>
        }
        .into_any(),
        LastRun::Failed => view! {
            <p class="g-ork-note">"Last run 2 h ago: removed 4 revisions, freed 22 MB."</p>
            <Banner variant=BannerVariant::Critical>
                "Could not remove old revisions of lab/assays: its folder could not be \
                 written. Nothing was removed from it; it will be tried on the next run."
            </Banner>
        }
        .into_any(),
    }
}

/// The Settings "Storage" card: the toggle, the count it governs, the hint
/// and the last-run line. The select is disabled while the toggle is off —
/// a count for a policy that is not running would be a promise with nothing
/// behind it — but stays visible, so turning it on is not a surprise.
fn storage(on: RwSignal<bool>, keep: RwSignal<String>, below: AnyView) -> AnyView {
    view! {
        <div class="g-ork-settings">
            <Card title="Storage">
                <ToggleRow
                    label="Remove old revisions automatically"
                    sublabel=SUBLABEL
                    checked=on
                />
                <div class="g-ork-field">
                    <FormControl
                        label="Keep the last"
                        caption=HINT
                        control=move |id| {
                            view! {
                                <Select
                                    naming=Naming::FormControl(id)
                                    options=vec![
                                        "3 revisions per package".to_string(),
                                        "5 revisions per package".to_string(),
                                        "10 revisions per package".to_string(),
                                        "25 revisions per package".to_string(),
                                    ]
                                    selected=keep
                                    disabled=Signal::derive(move || !on.get())
                                />
                            }
                                .into_any()
                        }
                    />
                    {below}
                </div>
            </Card>
        </div>
    }
    .into_any()
}

fn three() -> RwSignal<String> {
    RwSignal::new("3 revisions per package".to_string())
}

/// The confirmation's resting copy, at the modal's width: the dry run is its
/// one sentence, Cancel first, the verb last on a Danger button.
fn dry_run(on_cancel: Callback<()>, on_confirm: Callback<()>) -> AnyView {
    view! {
        <div class="g-bars g-dialog-inline">
            <p class="g-ork-dialog-title">"Remove old revisions automatically"</p>
            <p class="g-consequence">{DRY_RUN}</p>
            <div class="g-inline g-inline--end">
                <Button on_click=move |_| on_cancel.run(())>"Cancel"</Button>
                <Button variant=ButtonVariant::Danger on_click=move |_| on_confirm.run(())>
                    "Turn on and remove"
                </Button>
            </div>
        </div>
    }
    .into_any()
}

/// The storage card that answers: turn it on and the dry run appears under
/// it; Cancel turns it back off, the verb leaves it on with a result line.
#[component]
fn TryIt() -> impl IntoView {
    let wanted = RwSignal::new(false);
    let confirmed = RwSignal::new(false);
    Effect::new(move |_| {
        if !wanted.get() {
            confirmed.set(false);
        }
    });
    let below = view! {
        <Show when=move || wanted.get() && !confirmed.get()>
            {dry_run(
                Callback::new(move |()| wanted.set(false)),
                Callback::new(move |()| confirmed.set(true)),
            )}
        </Show>
        <Show when=move || confirmed.get()>
            <p class="g-ork-note">"Last run just now: removed 7 revisions, freed 29 MB."</p>
        </Show>
    }
    .into_any();
    storage(wanted, three(), below)
}

/// The prune after a pull, as the appbar says it while it runs. The kit's
/// line reads its words from context, so the cell provides one.
#[component]
fn Pruning() -> impl IntoView {
    let activities = Activities::new();
    activities.set(vec![Activity {
        kind: ActivityKind::Autopull,
        label: "Removing 3 old revisions of user/plate-07…".to_string(),
    }]);
    provide_context(activities);
    view! { <ActivityLine /> }
}

/// The resting copy, and a button that opens the real `ConfirmDialog`, whose
/// verb runs for a beat so both buttons can be watched refusing meanwhile.
#[component]
fn RealDryRun() -> impl IntoView {
    let live = RwSignal::new(false);
    view! {
        <div class="g-bars">
            {dry_run(Callback::new(|()| ()), Callback::new(|()| ()))}
            <div class="g-inline">
                <Button on_click=move |_| live.set(true)>"Open the real one"</Button>
            </div>
        </div>
        <ConfirmDialog
            open=live
            title="Remove old revisions automatically"
            consequence=DRY_RUN
            confirm=Submit::new(
                "Turn on and remove",
                || async {
                    after_a_beat(900).await;
                    Ok(())
                },
            )
        />
    }
}

const NOTE: &str = "No per-row removal anywhere: a count per package, a default in Settings \
    and an override in the popover's footer. The popover marks the rows the next run takes, \
    so the policy is visible before it acts; turning it on confirms a dry run first; afterwards \
    a notification says what went and Settings keeps one line about the last run. Change the \
    popover's Keep select and the marks follow it.";

#[component]
pub fn OldRevisionsKeepNScene() -> impl IntoView {
    let off = RwSignal::new(false);
    let on = RwSignal::new(true);
    let nothing = RwSignal::new(true);
    let working = RwSignal::new(true);
    let skipped = RwSignal::new(true);
    let failed = RwSignal::new(true);

    // One open signal per pane: an `auto` popover closes any other.
    let listed = RwSignal::new(false);

    view! {
        <Scene title="Option D · keep the last N automatically" note=NOTE>
            <Cell wide=true label="Settings — off, the default; the count waits, disabled">
                {storage(off, three(), ().into_any())}
            </Cell>
            <Cell wide=true label="Settings — on with keep 3, the ordinary case">
                {storage(
                    on,
                    three(),
                    last_run(LastRun::Ran("Last run 2 h ago: removed 7 revisions, freed 29 MB.")),
                )}
            </Cell>
            <Cell wide=true label="turn it on — the dry run confirms before anything goes">
                <TryIt />
            </Cell>
            <Cell wide=true label="the dry run, at the modal's width — and the real one">
                <RealDryRun />
            </Cell>
            <Cell wide=true label="Settings — ran, nothing to remove">
                {storage(nothing, three(), last_run(LastRun::Nothing))}
            </Cell>
            <Cell wide=true label="Settings — working (spinner)">
                {storage(working, three(), last_run(LastRun::Working("user/plate-06")))}
            </Cell>
            <Cell wide=true label="Settings — a package was busy and skipped">
                {storage(skipped, three(), last_run(LastRun::Skipped))}
            </Cell>
            <Cell wide=true label="Settings — failure on one package">
                {storage(failed, three(), last_run(LastRun::Failed))}
            </Cell>
            <Cell
                full=true
                label="click the trigger — plate-07 with keep 3: rows 1, 2 protected, row 3 kept as \
                       third, rows 4–6 will be removed; the Keep select re-marks them"
            >
                {in_page(listed, plate07(), RwSignal::new(keep_options()[0].clone()))}
            </Cell>
            <Cell wide=true label="the popover flat — keep 3 (default)">
                {flat(plate07(), &format!("Use default ({DEFAULT_KEEP})"))}
            </Cell>
            <Cell wide=true label="override — keep all: nothing marked">
                {flat(plate07(), "Keep all")}
            </Cell>
            <Cell
                wide=true
                label="the surprise: re-fetching row 5 (\"Initial upload\") moves it to the top \
                       and pushes row 3 out — N counts obtained order, not commit order"
            >
                {flat(plate07_refetched(), "3")}
            </Cell>
            <Cell wide=true label="only the current revision">
                {flat(only_current(), "3")}
            </Cell>
            <Cell wide=true label="everything protected — team/notes, keep 3">
                {flat(team_notes(), "3")}
            </Cell>
            <Cell full=true label="after a pull — the activity line while it runs">
                <Pruning />
            </Cell>
            <Cell full=true label="…then the result, in the notification stack">
                <Banner variant=BannerVariant::Success on_dismiss=|_| ()>
                    "Removed 3 old revisions of user/plate-07 · freed 6.0 MB"
                </Banner>
            </Cell>
            <Cell full=true label="a run that removes nothing says nothing">
                <p class="g-note">
                    "No notification: an automatic run that found nothing to remove is not news. \
                     Settings' last-run line is where that is visible."
                </p>
            </Cell>
            <Cell full=true label="failure, in the notification stack">
                <Banner variant=BannerVariant::Critical on_dismiss=|_| ()>
                    "Could not remove old revisions of lab/assays: its folder could not be \
                     written. It will be tried on the next run."
                </Banner>
            </Cell>
        </Scene>
    }
}
