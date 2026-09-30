//! Option A · remove from the popover — an exploration for removing old revisions, not a shipped design.
//!
//! # Where it lives
//!
//! In the "Revisions you have" surface the context pane already opens, at the
//! pane's 280px. The list is the thing a reader is looking at when they wonder
//! why there are six of them, so the removal sits on the rows rather than in a
//! settings page that has to name the package back to them.
//!
//! # The confirmation is an arm, not a dialog
//!
//! The surface is an `auto` popover, and a modal opened from it takes focus into
//! the top layer and light-dismisses the list it was asked about. So the footer
//! button confirms in place: the first press turns it into the Danger verb with
//! the count and the size in it, the second removes. Escape and focus leaving
//! disarm it. Escape while armed is swallowed, so the first one disarms and only
//! a second closes the surface.
//!
//! A row's own trash icon removes on one press. Every removable row is a
//! published revision — an unpublished one is protected — so what it undoes can
//! be obtained again; the bulk action is the one that earns the arm.
//!
//! # Sizes are what removing frees
//!
//! Never a revision's full size: only objects no other revision or package uses.
//! A row's figure is what it frees *alone, now* — so removing "Initial upload"
//! makes "Add Caihong folder-upload note" free more, because objects the two
//! shared are now its alone. The footer's figure is computed for the set, which
//! is why it is larger than the rows added up.
//!
//! # Kit changes the real build needs
//!
//! - `icons::trash()` — drawn here from Octicons' `trash-16` until then.
//! - `RevisionRow` wants a trailing-action slot and a detail after the time
//!   (`3 days ago · frees 1.2 MB`); composed here as a wrapper and a third line.
//! - `Button` wants a `node_ref` or `on_keydown`/`on_blur` so the arm can focus
//!   and disarm itself; composed here with a wrapping `span` and delegated
//!   events.
//! - `ButtonVariant::Danger`'s rule ("only `ConfirmDialog` draws one") would
//!   have to admit an armed button as a confirmation.

use leptos::ev::FocusEvent;
use leptos::ev::KeyboardEvent;
use leptos::ev::MouseEvent;
use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::Cell;
use crate::Scene;
use crate::gallery::forms::after_a_beat;
use crate::kit::Align;
use crate::kit::AnchoredOverlay;
use crate::kit::Banner;
use crate::kit::BannerVariant;
use crate::kit::Button;
use crate::kit::ButtonVariant;
use crate::kit::Card;
use crate::kit::CatalogLink;
use crate::kit::IconButton;
use crate::kit::IconButtonVariant;
use crate::kit::PaneSection;
use crate::kit::RevisionRow;
use crate::kit::StateTone;

const NAMESPACE: &str = "user/plate-07";

const HOUR: f64 = 3_600_000.0;
const DAY: f64 = 24.0 * HOUR;
const MINUTE: f64 = HOUR / 60.0;

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

/// A size, honestly: most old revisions free little or nothing, and a figure
/// rounded to `0.2 MB` would claim a precision the reader has no use for.
fn size(tenths: u32) -> String {
    match tenths {
        0 => "nothing".to_string(),
        1..=9 => "< 1 MB".to_string(),
        _ => format!("{}.{} MB", tenths / 10, tenths % 10),
    }
}

fn revisions(count: usize) -> String {
    if count == 1 {
        "1 revision".to_string()
    } else {
        format!("{count} revisions")
    }
}

fn catalog(hash: &str) -> CatalogLink {
    CatalogLink::new(
        format!("https://quilt-lab.example/b/quilt-lab-plates/packages/{NAMESPACE}/tree/{hash}/"),
        Callback::new(|_url: String| ()),
    )
}

/// Octicons' `trash-16`, drawn here only until the kit has it (see the module
/// comment). MIT, © GitHub Inc., as `kit/icons.rs` reproduces.
fn trash() -> AnyView {
    view! {
        <svg viewBox="0 0 16 16" aria-hidden="true" fill="currentColor">
            <path d="M11 1.75V3h2.25a.75.75 0 0 1 0 1.5H2.75a.75.75 0 0 1 0-1.5H5V1.75C5 .784 5.784 0 6.75 0h2.5C10.216 0 11 .784 11 1.75ZM4.496 6.675l.66 6.6a.25.25 0 0 0 .249.225h5.19a.25.25 0 0 0 .249-.225l.66-6.6a.75.75 0 0 1 1.492.149l-.66 6.6A1.748 1.748 0 0 1 10.595 15h-5.19a1.75 1.75 0 0 1-1.741-1.575l-.66-6.6a.75.75 0 1 1 1.492-.15ZM6.5 1.75V3h3V1.75a.25.25 0 0 0-.25-.25h-2.5a.25.25 0 0 0-.25.25Z" />
        </svg>
    }
    .into_any()
}

/// Where the footer stands. `Removed` and `Partial` are the result line; the
/// others are what the button is doing.
#[derive(Clone, Debug, PartialEq)]
enum Phase {
    Idle,
    Armed,
    Working,
    Removed {
        count: usize,
        freed: u32,
    },
    /// The package is locked by a sync. Nothing runs, and the words say which
    /// package — the surface belongs to one, but the refusal is read later.
    Busy,
    Failed,
    Partial {
        removed: usize,
        of: usize,
        freed: u32,
        left: usize,
    },
}

/// One surface's state. Every cell owns one, so every cell is live from the
/// point it starts at.
#[derive(Clone, Copy)]
struct Model {
    remaining: RwSignal<Vec<usize>>,
    phase: RwSignal<Phase>,
    /// Set while the arm swaps one button for another. The idle button is
    /// removed while it has focus, and a `focusout` from it then must not read
    /// as the reader leaving.
    settling: StoredValue<bool>,
}

impl Model {
    fn new(remaining: &[usize], phase: Phase) -> Self {
        Self {
            remaining: RwSignal::new(remaining.to_vec()),
            phase: RwSignal::new(phase),
            settling: StoredValue::new(false),
        }
    }

    fn removable(self) -> Vec<usize> {
        self.remaining
            .get()
            .into_iter()
            .filter(|&i| REVS[i].kept.is_none())
            .collect()
    }

    fn blocked(self) -> bool {
        matches!(self.phase.get(), Phase::Working | Phase::Busy)
    }

    /// Removes `set` after a beat, as the command would. The gallery's always
    /// succeeds; the failure and the partial are starting points, not outcomes.
    fn remove(self, set: Vec<usize>) {
        self.phase.set(Phase::Working);
        leptos::task::spawn_local(async move {
            after_a_beat(1200).await;
            let freed = freed(&set, &self.remaining.get_untracked());
            self.remaining
                .update(|rows| rows.retain(|i| !set.contains(i)));
            self.phase.set(Phase::Removed {
                count: set.len(),
                freed,
            });
        });
    }

    /// One row, at once. Its figure was what it frees alone, and that is what
    /// the result line reports.
    fn remove_one(self, i: usize) {
        let freed = freed(&[i], &self.remaining.get_untracked());
        self.remaining.update(|rows| rows.retain(|&r| r != i));
        self.phase.set(Phase::Removed { count: 1, freed });
    }
}

/// One row: the kit's revision row, the trash after its catalog icon, and a
/// third line saying what removing it frees — or why it cannot be removed.
fn row(i: usize, model: Model) -> AnyView {
    let rev = &REVS[i];
    let note = if let Some((tag, why)) = rev.kept {
        view! { <span class="g-ori-note" title=why>{tag}</span> }.into_any()
    } else {
        let words = move || format!("frees {}", size(freed(&[i], &model.remaining.get())));
        view! { <span class="g-ori-note">{words}</span> }.into_any()
    };
    // A kept row holds the trash's width empty, so every catalog icon in the
    // column lines up — except when the whole list is kept and no row has one.
    let action = if rev.kept.is_some() {
        let holds = move || model.removable().is_empty().then_some("display:none");
        view! { <span class="g-ori-gap" aria-hidden="true" style=holds></span> }.into_any()
    } else {
        view! {
            <IconButton
                icon=trash()
                aria_label="Remove this revision"
                variant=IconButtonVariant::Invisible
                disabled=Signal::derive(move || model.blocked())
                on_click=move |_| model.remove_one(i)
            />
        }
        .into_any()
    };
    view! {
        <div class="g-ori-row">
            <div class="g-ori-line">
                <RevisionRow
                    message=rev.message
                    at=ago(rev.obtained)
                    published=rev.hash.is_some()
                    catalog=rev.hash.map(catalog)
                />
                {action}
            </div>
            {note}
        </div>
    }
    .into_any()
}

/// Focuses the button inside `holder` once the swap has drawn it, then lets
/// `focusout` count again.
fn focus_after_swap(holder: NodeRef<leptos::html::Span>, settling: StoredValue<bool>) {
    request_animation_frame(move || {
        if let Some(button) = holder
            .get_untracked()
            .and_then(|el| el.query_selector("button").ok().flatten())
            .and_then(|el| el.dyn_into::<web_sys::HtmlElement>().ok())
        {
            drop(button.focus());
        }
        settling.set_value(false);
    });
}

/// The footer's button, in whichever of its four faces the phase gives it.
fn bulk(model: Model, removable: &[usize]) -> AnyView {
    let holder = NodeRef::<leptos::html::Span>::new();
    let count = removable.len();
    let total = size(freed(removable, &model.remaining.get_untracked()));
    let arm = move |_: MouseEvent| {
        model.settling.set_value(true);
        model.phase.set(Phase::Armed);
        focus_after_swap(holder, model.settling);
    };
    let set = removable.to_vec();
    let confirm = move |_: MouseEvent| model.remove(set.clone());

    // Escape disarms and is spent doing it, so the surface stays open; a second
    // one reaches the popover and closes it.
    let on_keydown = move |ev: KeyboardEvent| {
        if ev.key() == "Escape" && model.phase.get_untracked() == Phase::Armed {
            ev.prevent_default();
            ev.stop_propagation();
            model.settling.set_value(true);
            model.phase.set(Phase::Idle);
            focus_after_swap(holder, model.settling);
        }
    };
    let on_focusout = move |_: FocusEvent| {
        if !model.settling.get_value() && model.phase.get_untracked() == Phase::Armed {
            model.phase.set(Phase::Idle);
        }
    };
    // WebKit does not focus a button it is clicking, so pressing the armed
    // one would first blur it and disarm it under the pointer. Keeping the
    // focus where it is lets the click land on the button it was aimed at.
    let on_mousedown = move |ev: MouseEvent| {
        if model.phase.get_untracked() == Phase::Armed {
            ev.prevent_default();
        }
    };

    let face = move || match model.phase.get() {
        Phase::Armed => view! {
            <Button variant=ButtonVariant::Danger on_click=confirm.clone()>
                {format!("Remove {count}? frees {total}")}
            </Button>
        }
        .into_any(),
        Phase::Working => view! {
            <Button loading=true on_click=|_| ()>
                {format!("Removing {count}\u{2026}")}
            </Button>
        }
        .into_any(),
        phase => view! {
            <Button disabled=phase == Phase::Busy on_click=arm>
                {format!("Remove {count} older · frees {total}")}
            </Button>
        }
        .into_any(),
    };
    view! {
        <span
            class="g-ori-arm"
            node_ref=holder
            on:keydown=on_keydown
            on:focusout=on_focusout
            on:mousedown=on_mousedown
        >
            {face}
        </span>
    }
    .into_any()
}

/// What the footer says above its button, if anything.
fn status(model: Model, removable: &[usize]) -> Option<AnyView> {
    Some(match model.phase.get() {
        Phase::Removed { count, freed } => view! {
            <p class="g-ori-result">
                {StateTone::Success.glyph()}
                <span>{format!("Removed {} · freed {}", revisions(count), size(freed))}</span>
            </p>
        }
        .into_any(),
        Phase::Busy => view! {
            <p class="g-ori-muted">
                {format!("{NAMESPACE} is busy syncing — try again in a moment")}
            </p>
        }
        .into_any(),
        Phase::Failed => {
            let set = removable.to_vec();
            view! {
                <Banner variant=BannerVariant::Critical>
                    "Could not remove old revisions: a file in one is open in another program."
                </Banner>
                <Button on_click=move |_| model.remove(set.clone())>"Try again"</Button>
            }
            .into_any()
        }
        Phase::Partial {
            removed,
            of,
            freed,
            left,
        } => {
            let message = REVS[left].message;
            view! {
                <p class="g-ori-result">
                    <span>{format!("Removed {removed} of {of} · freed {}", size(freed))}</span>
                </p>
                <Banner variant=BannerVariant::Critical>
                    {format!("\u{201c}{message}\u{201d} could not be removed: a file in it is open in another program.")}
                </Banner>
                <Button on_click=move |_| model.remove(vec![left])>"Try again"</Button>
            }
            .into_any()
        }
        Phase::Idle | Phase::Armed | Phase::Working if removable.is_empty() => view! {
            <p class="g-ori-muted">"Nothing to remove — every revision here is in use"</p>
        }
        .into_any(),
        _ => return None,
    })
}

/// The footer. Absent when the list is the current revision alone: there is
/// nothing to say about removing, not even that nothing can be.
///
/// The button is rebuilt only when what it would remove changes, or when a
/// retry takes its place — never on the arm, which has to keep the element it
/// just focused.
fn footer(model: Model) -> impl IntoView {
    // Failure and partial own the footer's one action, `Try again`.
    let retrying =
        Memo::new(move |_| matches!(model.phase.get(), Phase::Failed | Phase::Partial { .. }));
    move || {
        let remaining = model.remaining.get();
        if remaining == [0] {
            return None;
        }
        let removable = model.removable();
        let offers = !removable.is_empty() && !retrying.get();
        Some(view! {
            <PaneSection>
                // Kept mounted with the footer, so a result line arriving in it
                // is announced rather than inserted with its region.
                <div class="g-ori-footer" role="status">
                    {move || status(model, &model.removable())}
                    {offers.then(|| bulk(model, &removable))}
                </div>
            </PaneSection>
        })
    }
}

/// The surface's contents: the rows, then the footer, which `PaneSection`
/// rules off.
fn body(model: Model) -> AnyView {
    view! {
        <div class="g-ori-body">
            <PaneSection>
                <div class="g-ori-rows">
                    {move || {
                        model.remaining.get().into_iter().map(|i| row(i, model)).collect_view()
                    }}
                </div>
            </PaneSection>
            {footer(model)}
        </div>
    }
    .into_any()
}

/// The surface drawn in the page, for a cell to hold still. The same border,
/// padding and shadow the overlay's surface draws, so the static cells read as
/// the popover they are copies of.
fn surface(remaining: &[usize], phase: Phase) -> AnyView {
    view! { <div class="g-ori-surface">{body(Model::new(remaining, phase))}</div> }.into_any()
}

/// The pane with the real trigger and the real popover, beside a stand-in for
/// the file list — the surface opens leftwards over it, as on the page.
fn live() -> AnyView {
    let open = RwSignal::new(false);
    let model = Model::new(&[0, 1, 2, 3, 4, 5], Phase::Idle);
    let trigger = move |surface_id: String| {
        view! {
            <Button
                on_click=move |_| open.update(|o| *o = !*o)
                aria_expanded=open
                aria_controls=surface_id
            >
                {move || format!("Revisions you have ({})", model.remaining.get().len())}
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
                            {body(model)}
                        </AnchoredOverlay>
                    </PaneSection>
                </Card>
            </aside>
        </div>
    }
    .into_any()
}

const ALL: [usize; 6] = [0, 1, 2, 3, 4, 5];

const NOTE: &str = "Removal where the list already is: the context pane's \"Revisions you have\" \
    surface, at the pane's width. A removable row ends in a trash icon after its catalog icon \
    and says what removing it frees alone — often less than 1 MB or nothing, which is the \
    truth about old revisions. Protected rows say why in a muted tag, and hovering the tag \
    gives the sentence. \
    \
    The footer removes every removable row. It confirms in place, because a modal would \
    light-dismiss the popover: press it once and it turns into the Danger verb with the \
    count and the size; press again to remove; Escape or tabbing away disarms. Every cell \
    is live from where it starts. Remove one row and watch the others' figures move: \
    objects it shared with another are now that one's alone.";

#[component]
pub fn OldRevisionsInlineScene() -> impl IntoView {
    view! {
        <Scene title="Option A · remove from the popover" note=NOTE>
            <Cell full=true label="live — open the surface; remove a row, or arm the footer">
                {live()}
            </Cell>
            <Cell wide=true label="the ordinary case — four removable, two kept">
                {surface(&ALL, Phase::Idle)}
            </Cell>
            <Cell wide=true label="armed — the second press removes; Escape or blur disarms">
                {surface(&ALL, Phase::Armed)}
            </Cell>
            <Cell wide=true label="working — rows hold still, nothing can be pressed">
                {surface(&ALL, Phase::Working)}
            </Cell>
            <Cell wide=true label="the result line, in the footer">
                {surface(&[0, 1], Phase::Removed { count: 4, freed: 69 })}
            </Cell>
            <Cell wide=true label="only the current revision — no footer, no icons">
                {surface(&[0], Phase::Idle)}
            </Cell>
            <Cell wide=true label="everything protected">
                {surface(&[0, 1], Phase::Idle)}
            </Cell>
            <Cell wide=true label="busy — the package is syncing, every button refuses">
                {surface(&ALL, Phase::Busy)}
            </Cell>
            <Cell wide=true label="failure — nothing removed, Try again runs the set">
                {surface(&ALL, Phase::Failed)}
            </Cell>
            <Cell wide=true label="partial — three went, one is still here">
                {surface(
                    &[0, 1, 3],
                    Phase::Partial { removed: 3, of: 4, freed: 51, left: 3 },
                )}
            </Cell>
        </Scene>
    }
}
