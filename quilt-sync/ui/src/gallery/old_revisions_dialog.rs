//! Option B · a Manage revisions dialog — an exploration for removing old revisions, not a shipped design.
//!
//! # One form per package, and the form is the confirmation
//!
//! The revisions this computer has are listed as a form: every removable one with a box,
//! the ones that must stay grouped last under `Kept` with the reason in place of a box
//! that could be ticked. The Danger button counts what it will remove and the line above
//! it says what that frees, so pressing it is the decision — a second "Are you sure?"
//! after a dialog that already named the count and the bytes would be asking twice.
//!
//! # One total, for the set
//!
//! No row carries a size. What a revision frees depends on what else goes with it —
//! objects shared only among the removed ones are freed too — so a per-row figure would
//! be true alone and wrong summed, and a reader summing a column is exactly what rows
//! invite. The one number is computed for the selection, and it goes quiet
//! (`measuring…`) while it is recomputed rather than showing the last set's figure.
//!
//! # Drawn inline, and the real one behind a button
//!
//! As `confirm_dialog.rs` argues: a modal in the top layer takes the page's pointer
//! events, so each state below is the dialog's body and footer drawn inline at the
//! modal's width. The live scene at the end opens the kit's `Dialog` with the same body.
//! It is `Dialog` and not `FormDialog` — see [`LiveScene`] for why.

use std::collections::BTreeSet;

use leptos::ev::MouseEvent;
use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::Cell;
use crate::Scene;
use crate::gallery::forms::after_a_beat;
use crate::kit::ActionMenu;
use crate::kit::Align;
use crate::kit::AnchoredOverlay;
use crate::kit::Banner;
use crate::kit::BannerVariant;
use crate::kit::Button;
use crate::kit::ButtonVariant;
use crate::kit::CheckState;
use crate::kit::Checkbox;
use crate::kit::Dialog;
use crate::kit::GroupHeading;
use crate::kit::MenuAction;
use crate::kit::PaneSection;
use crate::kit::RevisionRow;
use crate::kit::Spinner;
use crate::kit::ZeroLine;

const NAMESPACE: &str = "user/plate-07";
const TITLE: &str = "Manage revisions";

const MINUTE: f64 = 60_000.0;
const HOUR: f64 = 60.0 * MINUTE;
const DAY: f64 = 24.0 * HOUR;

/// The sentence above the buttons. It names where the loss is (this computer) and why it
/// is not a loss for published ones — every removable revision is published, because an
/// unpublished one exists nowhere else and is kept.
const CONSEQUENCE: &str = "Removes these revisions from this computer. Published ones can be \
                           installed again from the remote.";

/// Attention, not Danger: a package that is syncing is the ordinary case, nothing went
/// wrong, and the remedy is waiting. Names the package, because a Settings-wide caller
/// would draw the same banner for any of several.
const BUSY: &str = "user/plate-07 is syncing, so nothing was removed. Try again in a moment.";
/// Danger: the removal went wrong. Says nothing was removed, because the selection is
/// kept and a reader needs to know pressing Remove again is not removing twice.
const FAILED: &str = "Could not remove the revisions: a file in QuiltSync's cache could not \
                      be deleted. Nothing was removed.";

/// One revision this computer has. `kept` is the reason it can never be removed, drawn
/// where a removable row has its box.
#[derive(Clone, Copy)]
struct Rev {
    id: usize,
    message: &'static str,
    /// How long ago this computer *obtained* it — the only date on disk.
    ago: f64,
    published: bool,
    kept: Option<&'static str>,
}

/// The shared fixture, newest obtained first. #3 was made before #4 and sits above it
/// because it was fetched again yesterday — the order is acquisition, not history.
const PLATE_07: [Rev; 6] = [
    Rev {
        id: 1,
        message: "Normalize well IDs",
        ago: 20.0 * MINUTE,
        published: false,
        kept: Some("current · not pushed"),
    },
    Rev {
        id: 2,
        message: "Re-run plate 7 with the corrected layout",
        ago: 2.0 * HOUR,
        published: true,
        kept: Some("latest · base"),
    },
    Rev {
        id: 3,
        message: "Add plate 6 controls",
        ago: DAY,
        published: true,
        kept: None,
    },
    Rev {
        id: 4,
        message: "Add Caihong folder-upload note",
        ago: 3.0 * DAY,
        published: true,
        kept: None,
    },
    Rev {
        id: 5,
        message: "Initial upload",
        ago: 12.0 * DAY,
        published: true,
        kept: None,
    },
    Rev {
        id: 6,
        message: "",
        ago: 20.0 * DAY,
        published: true,
        kept: None,
    },
];

/// A package whose only revision is the one being worked on.
const ONLY_CURRENT: [Rev; 1] = [Rev {
    id: 1,
    message: "Normalize well IDs",
    ago: 20.0 * MINUTE,
    published: false,
    kept: Some("current · not pushed"),
}];

/// Every row protected: the two the fixture keeps, and nothing else.
const ALL_KEPT: [Rev; 2] = [PLATE_07[0], PLATE_07[1]];

fn removable(rows: &[Rev]) -> Vec<usize> {
    rows.iter()
        .filter(|r| r.kept.is_none())
        .map(|r| r.id)
        .collect()
}

/// What removing `set` frees, in tenths of a megabyte — the stub of the backend's
/// measurement. Each row's own share, plus 0.7 MB that only #4 and #5 use: those objects
/// go only when both do, which is why a set is measured and never added up.
fn freed_tenths(set: &BTreeSet<usize>) -> u32 {
    let alone: u32 = set
        .iter()
        .map(|id| match id {
            3 => 2,
            4 => 12,
            5 => 48,
            _ => 0,
        })
        .sum();
    let shared = if set.contains(&4) && set.contains(&5) {
        7
    } else {
        0
    };
    alone + shared
}

/// The size in words. `< 1 MB` rather than `0.2 MB`: most old revisions free that little,
/// and a decimal suggests a precision the reader has no use for. `nothing` is said
/// outright — a revision that frees nothing is still removable, it just tidies the list.
fn size(tenths: u32) -> String {
    match tenths {
        0 => "nothing".to_string(),
        1..=9 => "< 1 MB".to_string(),
        _ => format!("{}.{} MB", tenths / 10, tenths % 10),
    }
}

fn revisions(n: usize) -> String {
    if n == 1 {
        "1 revision".to_string()
    } else {
        format!("{n} revisions")
    }
}

/// The primary's words. It counts, so the button is the confirmation; with nothing
/// picked it keeps its verb and is disabled, rather than reading `Remove 0 revisions`.
fn verb(n: usize) -> String {
    if n == 0 {
        "Remove revisions".to_string()
    } else {
        format!("Remove {}", revisions(n))
    }
}

/// What the page says once the dialog has closed on success.
fn result_line(n: usize, tenths: u32) -> String {
    format!("Removed {} · freed {}", revisions(n), size(tenths))
}

/// Everything the dialog's body and footer read. Signals, so one definition serves the
/// frozen inline copies and the live dialog.
#[derive(Clone, Copy)]
struct Form {
    rows: Signal<Vec<Rev>>,
    selected: RwSignal<BTreeSet<usize>>,
    measuring: Signal<bool>,
    working: Signal<bool>,
    banner: Signal<Option<(BannerVariant, &'static str)>>,
}

impl Form {
    /// A frozen state for an inline cell. The boxes still toggle — the count, the total
    /// and the verb follow — but nothing measures or runs.
    fn frozen(
        rows: &'static [Rev],
        picked: &[usize],
        measuring: bool,
        working: bool,
        banner: Option<(BannerVariant, &'static str)>,
    ) -> Self {
        Self {
            rows: Signal::stored(rows.to_vec()),
            selected: RwSignal::new(picked.iter().copied().collect()),
            measuring: Signal::stored(measuring),
            working: Signal::stored(working),
            banner: Signal::stored(banner),
        }
    }

    fn removable(self) -> Signal<Vec<usize>> {
        Signal::derive(move || removable(&self.rows.get()))
    }

    fn picked(self) -> Signal<usize> {
        Signal::derive(move || self.selected.with(BTreeSet::len))
    }
}

/// The select-all line and the live total, on one row: the total sits beside the thing
/// that changes it.
///
/// Composed from `Checkbox` rather than `SelectAll`, whose resting text is `Select all 4`
/// or `Select all 4 shown`. Here the count must say `removable` — the list holds six rows
/// and a reader counting them would otherwise find two the control does not touch. The
/// real build wants a noun on `SelectAll`.
fn toolbar(form: Form) -> AnyView {
    let all = form.removable();
    let picked = form.picked();
    let state = Signal::derive(move || {
        let (p, a) = (picked.get(), all.with(Vec::len));
        if p == 0 || a == 0 {
            CheckState::Off
        } else if p >= a {
            CheckState::On
        } else {
            CheckState::Mixed
        }
    });
    let text = move || {
        let (p, a) = (picked.get(), all.with(Vec::len));
        if p == 0 {
            format!("Select all {a} removable")
        } else {
            format!("{p} of {a} selected")
        }
    };
    let on_toggle = move |on: bool| {
        form.selected.set(if on {
            all.get().into_iter().collect()
        } else {
            BTreeSet::new()
        });
    };
    let total = move || {
        if picked.get() == 0 {
            None
        } else if form.measuring.get() {
            Some(
                view! {
                    <span class="g-ord-total g-ord-total--measuring">
                        <Spinner aria_label="Measuring what this frees" />
                        "measuring…"
                    </span>
                }
                .into_any(),
            )
        } else {
            let freed = size(form.selected.with(freed_tenths));
            Some(view! { <span class="g-ord-total">{format!("Frees {freed}")}</span> }.into_any())
        }
    };
    view! {
        <div class="g-ord-toolbar">
            <label class="g-ord-selectall">
                <Checkbox state=state on_toggle=on_toggle />
                <span>{text}</span>
            </label>
            // `aria-live`, so a reader ticking boxes hears the total settle without
            // leaving the list to find it.
            <span class="g-ord-total-slot" aria-live="polite">{total}</span>
        </div>
    }
    .into_any()
}

/// A removable row: the box, then the revision. The whole row is the label, so the
/// message is the click target — a box 14px wide is not.
fn pick_row(form: Form, rev: Rev) -> AnyView {
    let id = rev.id;
    let state = Signal::derive(move || CheckState::from(form.selected.with(|s| s.contains(&id))));
    let name = if rev.message.is_empty() {
        "No message".to_string()
    } else {
        rev.message.to_string()
    };
    view! {
        <label class="g-ord-row">
            <Checkbox
                state=state
                aria_label=name
                on_toggle=move |on| {
                    form.selected
                        .update(|s| {
                            if on {
                                s.insert(id);
                            } else {
                                s.remove(&id);
                            }
                        });
                }
            />
            <RevisionRow
                message=rev.message
                at=js_sys::Date::now() - rev.ago
                published=rev.published
            />
        </label>
    }
    .into_any()
}

/// A kept row: the same box, disabled, so the column does not jog; the reason on the
/// right, muted, because it states a fact and asks nothing of the reader.
fn kept_row(rev: Rev, reason: &'static str) -> AnyView {
    view! {
        <div class="g-ord-row g-ord-row--kept">
            <Checkbox
                state=Signal::stored(CheckState::Off)
                aria_label=format!("{} — kept: {reason}", rev.message)
                disabled=true
                on_toggle=|_| ()
            />
            <RevisionRow
                message=rev.message
                at=js_sys::Date::now() - rev.ago
                published=rev.published
            />
            <span class="g-ord-reason">{reason}</span>
        </div>
    }
    .into_any()
}

/// The dialog's body: banner, list, sentence. Everything but the buttons.
fn body(form: Form) -> AnyView {
    let list = move || {
        let rows = form.rows.get();
        let kept: Vec<(Rev, &'static str)> = rows
            .iter()
            .filter_map(|r| r.kept.map(|why| (*r, why)))
            .collect();
        let open: Vec<Rev> = rows.iter().filter(|r| r.kept.is_none()).copied().collect();
        let nothing = open.is_empty();
        view! {
            {if nothing {
                // Said rather than drawn as an empty list with a disabled Select all: the
                // reader opened this to free space, and the answer is that there is none
                // to free here, and why is just below.
                view! { <ZeroLine text="Nothing to remove — every revision here is kept." /> }
                    .into_any()
            } else {
                view! {
                    {toolbar(form)}
                    <div class="g-ord-rows">
                        {open.into_iter().map(|r| pick_row(form, r)).collect_view()}
                    </div>
                }
                    .into_any()
            }}
            // Last, and grouped: these are the rows a reader is not choosing between, and
            // interleaved by date they would break up the run of boxes.
            {(!kept.is_empty())
                .then(|| {
                    view! {
                        <GroupHeading title="Kept" count=kept.len() />
                        <div class="g-ord-rows">
                            {kept.into_iter().map(|(r, why)| kept_row(r, why)).collect_view()}
                        </div>
                    }
                })}
            {(!nothing).then(|| view! { <p class="g-consequence">{CONSEQUENCE}</p> })}
        }
    };
    view! {
        {move || {
            form.banner.get().map(|(variant, words)| view! { <Banner variant=variant>{words}</Banner> })
        }}
        <p class="g-ord-caption">{format!("{NAMESPACE} · newest obtained first")}</p>
        // Sealed while the removal runs, as `FormDialog` seals its fields: the boxes in
        // flight are the ones that were submitted.
        <fieldset class="g-ord-fields" disabled=move || form.working.get()>
            {list}
        </fieldset>
    }
    .into_any()
}

/// Cancel, then the Danger verb. With nothing removable, one `Close`, as a read-only
/// `FormDialog` draws it.
fn footer(form: Form, on_cancel: Callback<()>, on_remove: Callback<()>) -> AnyView {
    let picked = form.picked();
    let nothing = Signal::derive(move || form.removable().with(Vec::is_empty));
    view! {
        {move || {
            if nothing.get() {
                view! {
                    <Button variant=ButtonVariant::Primary on_click=move |_| on_cancel.run(())>
                        "Close"
                    </Button>
                }
                    .into_any()
            } else {
                view! {
                    <Button disabled=form.working on_click=move |_| on_cancel.run(())>
                        "Cancel"
                    </Button>
                    // Disabled while measuring too: the dialog is the confirmation, and
                    // it should not be confirmed before it has said what it frees.
                    <Button
                        variant=ButtonVariant::Danger
                        disabled=Signal::derive(move || picked.get() == 0 || form.measuring.get())
                        loading=form.working
                        on_click=move |_| on_remove.run(())
                    >
                        {move || verb(picked.get())}
                    </Button>
                }
                    .into_any()
            }
        }}
    }
    .into_any()
}

/// The dialog drawn inline, with its title and edge, at the modal's width.
fn inline(form: Form) -> AnyView {
    let none = Callback::new(|()| ());
    view! {
        <div class="g-ord-dialog">
            <h2 class="g-ord-dialog__title">{TITLE}</h2>
            <div class="g-ord-dialog__body">{body(form)}</div>
            <div class="g-ord-dialog__footer">{footer(form, none, none)}</div>
        </div>
    }
    .into_any()
}

#[component]
pub fn OldRevisionsDialogScene() -> impl IntoView {
    view! {
        <Scene
            title="Option B · a Manage revisions dialog"
            note="One form per package. Removable revisions first, each with a box; the ones \
                  that must stay grouped last under Kept, with the reason where a box would \
                  be. No row carries a size — what a revision frees depends on what goes \
                  with it — so there is one total, measured for the selection. The Danger \
                  button counts what it removes, and that is the confirmation: there is no \
                  second one. Each state is drawn inline at the modal's width; the boxes \
                  in them still toggle."
        >
            <div class="g-ord-grid">
                <EntryPoints />
                <Cell wide=true label="ordinary — two selected">
                    {inline(Form::frozen(&PLATE_07, &[4, 5], false, false, None))}
                </Cell>
                <Cell wide=true label="all selected — the set frees more than its rows">
                    {inline(Form::frozen(&PLATE_07, &[3, 4, 5, 6], false, false, None))}
                </Cell>
                <Cell wide=true label="nothing selected — the verb stays, disabled">
                    {inline(Form::frozen(&PLATE_07, &[], false, false, None))}
                </Cell>
                <Cell wide=true label="measuring — the last total is not shown">
                    {inline(Form::frozen(&PLATE_07, &[3, 4, 5], true, false, None))}
                </Cell>
                <Cell wide=true label="working — sealed, Cancel refused">
                    {inline(Form::frozen(&PLATE_07, &[3, 4, 5, 6], false, true, None))}
                </Cell>
                <Cell wide=true label="refused — the package is syncing">
                    {inline(
                        Form::frozen(
                            &PLATE_07,
                            &[3, 4, 5, 6],
                            false,
                            false,
                            Some((BannerVariant::Warning, BUSY)),
                        ),
                    )}
                </Cell>
                <Cell wide=true label="failed — the selection is kept">
                    {inline(
                        Form::frozen(
                            &PLATE_07,
                            &[3, 4, 5, 6],
                            false,
                            false,
                            Some((BannerVariant::Critical, FAILED)),
                        ),
                    )}
                </Cell>
                <Cell wide=true label="only the current revision — nothing removable">
                    {inline(Form::frozen(&ONLY_CURRENT, &[], false, false, None))}
                </Cell>
                <Cell wide=true label="everything protected — Close only">
                    {inline(Form::frozen(&ALL_KEPT, &[], false, false, None))}
                </Cell>
                <Cell wide=true label="result — the dialog has closed">
                    <div class="g-ord-result">
                        <Banner variant=BannerVariant::Success on_dismiss=Callback::new(|_: MouseEvent| ())>
                            {result_line(4, 69)}
                        </Banner>
                    </div>
                </Cell>
            </div>
            <LiveScene />
        </Scene>
    }
}

/// Where the dialog is opened from, drawn small and static so both can be read at once.
///
/// The popover's footer is a composition: `AnchoredOverlay` has no footer slot and the kit
/// has no link-styled button, so the rule and the `Manage…` link are this file's classes.
/// The menu is a copy of `ActionMenu`'s open surface, because the kit's own surface is
/// private to it; the live one is in the scene at the end.
#[component]
fn EntryPoints() -> impl IntoView {
    view! {
        <Cell wide=true label="entry · the Revisions you have popover, with a footer">
            <div class="g-ord-popover">
                <PaneSection>
                    {PLATE_07[..4]
                        .iter()
                        .map(|r| {
                            view! {
                                <RevisionRow
                                    message=r.message
                                    at=js_sys::Date::now() - r.ago
                                    published=r.published
                                />
                            }
                        })
                        .collect_view()}
                </PaneSection>
                <div class="g-ord-popover__footer">
                    <button type="button" class="g-ord-link">"Manage…"</button>
                </div>
            </div>
        </Cell>
        <Cell label="entry · the package's action menu">
            <div class="g-ord-menu" aria-label="More package actions">
                <button type="button" class="g-ord-menu__item">"Open folder"</button>
                <button type="button" class="g-ord-menu__item">"Manage revisions…"</button>
                <div class="g-ord-menu__rule" role="separator"></div>
                <button type="button" class="g-ord-menu__item g-ord-menu__item--danger">
                    "Stop keeping"
                </button>
            </div>
        </Cell>
    }
}

/// How the live removal ends. Picked by which button opened the dialog, as the forms
/// scene opens one case per button.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Removed,
    Busy,
    Failed,
}

/// The real thing: both entry points, live, opening the kit's `Dialog` with the body the
/// cells draw.
///
/// `Dialog` rather than `FormDialog`, for two reasons the real build would have to fix in
/// the kit: `FormDialog`'s primary is always `Primary`, and this one is `Danger`; and its
/// label is fixed when `Submit` is built, and this one counts the selection as it changes.
/// Without those, the in-flight state, the sealed Escape and the banner are hand-held here
/// the way `FormDialog` holds them.
///
/// The measuring delay is faked: each change of selection shows `measuring…` for a beat,
/// and a later change supersedes an earlier one's answer.
#[component]
fn LiveScene() -> impl IntoView {
    let rows = RwSignal::new(PLATE_07.to_vec());
    let selected = RwSignal::new(BTreeSet::<usize>::new());
    let measuring = RwSignal::new(false);
    let working = RwSignal::new(false);
    let banner = RwSignal::new(None::<(BannerVariant, &'static str)>);
    let outcome = RwSignal::new(Outcome::Removed);
    let result = RwSignal::new(None::<String>);
    let open = RwSignal::new(false);
    let popover = RwSignal::new(false);
    let generation = StoredValue::new(0_u32);

    // Re-measure on every change of selection; only the newest answer lands.
    Effect::new(move |_| {
        let picked = selected.with(BTreeSet::len);
        if picked == 0 {
            measuring.set(false);
            return;
        }
        generation.update_value(|g| *g += 1);
        let mine = generation.get_value();
        measuring.set(true);
        spawn_local(async move {
            after_a_beat(450).await;
            if generation.get_value() == mine {
                measuring.set(false);
            }
        });
    });

    let form = Form {
        rows: rows.into(),
        selected,
        measuring: measuring.into(),
        working: working.into(),
        banner: banner.into(),
    };

    let show = move |how: Outcome| {
        outcome.set(how);
        selected.set(BTreeSet::new());
        banner.set(None);
        popover.set(false);
        open.set(true);
    };

    let on_cancel = Callback::new(move |()| {
        if !working.get_untracked() {
            open.set(false);
        }
    });
    let on_remove = Callback::new(move |()| {
        working.set(true);
        banner.set(None);
        spawn_local(async move {
            after_a_beat(900).await;
            working.set(false);
            match outcome.get_untracked() {
                Outcome::Removed => {
                    let gone = selected.get_untracked();
                    result.set(Some(result_line(gone.len(), freed_tenths(&gone))));
                    rows.update(|rs| rs.retain(|r| !gone.contains(&r.id)));
                    selected.set(BTreeSet::new());
                    open.set(false);
                }
                // The selection stays in both: the reader tries again, or waits and does.
                Outcome::Busy => banner.set(Some((BannerVariant::Warning, BUSY))),
                Outcome::Failed => banner.set(Some((BannerVariant::Critical, FAILED))),
            }
        });
    });

    let trigger = move |surface_id: String| {
        view! {
            <Button
                on_click=move |_| popover.update(|o| *o = !*o)
                aria_expanded=popover
                aria_controls=surface_id
            >
                {move || format!("Revisions you have ({})", rows.with(Vec::len))}
            </Button>
        }
        .into_any()
    };

    let menu = vec![
        MenuAction::new("Open folder", Callback::new(|()| ())),
        MenuAction::new(
            "Manage revisions…",
            Callback::new(move |()| show(Outcome::Removed)),
        ),
        MenuAction::new("Stop keeping", Callback::new(|()| ())).danger(),
    ];

    view! {
        <div class="g-ord-live">
            <p class="g-note">
                "Live. Either entry point opens the real dialog; the three buttons open it \
                 with the removal ending each way. A successful removal takes the rows out, \
                 so a second opening shows what is left — and after removing all four, the \
                 nothing-removable state."
            </p>
            <div class="g-inline">
                <AnchoredOverlay
                    trigger=trigger
                    open=popover
                    aria_label="Revisions you have"
                    align=Align::Start
                >
                    <PaneSection>
                        {move || {
                            rows.get()
                                .into_iter()
                                .map(|r| {
                                    view! {
                                        <RevisionRow
                                            message=r.message
                                            at=js_sys::Date::now() - r.ago
                                            published=r.published
                                        />
                                    }
                                })
                                .collect_view()
                        }}
                    </PaneSection>
                    <div class="g-ord-popover__footer">
                        <button
                            type="button"
                            class="g-ord-link"
                            on:click=move |_| show(Outcome::Removed)
                        >
                            "Manage…"
                        </button>
                    </div>
                </AnchoredOverlay>
                <ActionMenu aria_label="More package actions" actions=menu />
                <Button on_click=move |_| show(Outcome::Removed)>"Open the real one"</Button>
                <Button on_click=move |_| show(Outcome::Busy)>"…one that is refused"</Button>
                <Button on_click=move |_| show(Outcome::Failed)>"…one that fails"</Button>
                <Button on_click=move |_| {
                    rows.set(PLATE_07.to_vec());
                    result.set(None);
                }>"Put the revisions back"</Button>
            </div>
            {move || {
                result
                    .get()
                    .map(|line| {
                        view! {
                            <div class="g-ord-result">
                                <Banner
                                    variant=BannerVariant::Success
                                    on_dismiss=Callback::new(move |_: MouseEvent| result.set(None))
                                >
                                    {line}
                                </Banner>
                            </div>
                        }
                    })
            }}
            <Dialog open=open title=TITLE held=working footer=footer(form, on_cancel, on_remove)>
                {body(form)}
            </Dialog>
        </div>
    }
}
