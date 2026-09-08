//! A heavy-phase check that failed: the packages the page could not account for.
//!
//! # What the state is
//!
//! `refresh_main_page_package` rejected, so the row keeps the light phase's cached
//! state and nothing has confirmed it. The state is not *wrong* — it is
//! **unwitnessed**, and it stays that way until something asks again. The row already
//! says so in two channels, dashed and dimmed (`kit/package_row.module.scss`); what
//! it cannot say is that the check failed rather than that it is still running, and it
//! cannot offer to run it again.
//!
//! # Why the queue carries it and not the row
//!
//! `kit/package_row.module.scss` and `queue.rs`'s own comment both argued the row
//! should carry it. Drawn in the browser, that position loses on three counts, and
//! the third is measured rather than asserted:
//!
//! 1. **The row cannot hold the retry.** It is one real `<a>` with no controls, which
//!    is what buys middle-click, open-in-new-tab and one tab stop per row. An anchor
//!    may hold no interactive descendant — the same rule that makes `FileRow` pay for
//!    its children with `role="button"`. So the row could carry words, never a button.
//! 2. **Words on the row cost the state.** Spending the state label on the failure
//!    drops the cached state, which is the informative half — the light phase is right
//!    most of the time. And on a page where every check failed it becomes forty-three
//!    identical Attention chips, which is the wall-of-green problem the ragged state
//!    column exists to avoid. The time column cannot take the words instead: it
//!    already owns a different absence, `not recorded`, meaning the package has no
//!    recorded change time.
//! 3. **The dim is a weaker signal than anyone had checked, and weakest in dark.**
//!    `package_row.module.scss` measured *legibility* — dimmed text against its
//!    ground — and is right: 7.05:1 in light and 9.40:1 in dark, both above AA at
//!    opacity 0.75. What nobody measured is **distinguishability**, dimmed text
//!    against a settled row's text, which is the channel a reader actually uses to
//!    notice a row is unconfirmed. That is 2.24:1 in light and only **1.73:1 in
//!    dark** — computed from the rendered colours (light `#1E1F24` on `#FAFAFA`,
//!    dark `#EEEEF0` on `#111111`, composited at 0.75 in sRGB, WCAG relative
//!    luminance). A reader in dark cannot reliably tell a dimmed row from a settled
//!    one without a settled one beside it.
//!
//! The offline scene below is what decides it. Today, when every check fails, the
//! queue returns nothing twice over — once on an empty settled list and once because
//! nothing is accounted for — so the region is *absent*: no card, no heading, no zero
//! line. Words on a row do not bring it back. A cause row does.
//!
//! # Why these words
//!
//! `Couldn't check for new revisions on {host}` — host-grouped, so the sentence and
//! its remedy share a scope, exactly as the signed-out cause does.
//!
//! It claims only what is known. `Couldn't reach {host}` reads better and is shorter,
//! and was rejected: the call can fail in credential vending, in the role query or in
//! the hash walk, none of which establish a network fact, and this epic has already
//! paid twice for a state it manufactured. `Couldn't check for changes on {host}` was
//! rejected for pointing at the wrong half — on this page "changes" means *local* file
//! changes (`2 files changed`, `Publish your changes`). `Couldn't check these
//! packages` was rejected on sight: `CauseRow` appends `— N packages`, so it stutters.
//!
//! `new revisions` is the phrase the page already uses for this operation, on the
//! autosync toggle. The host clause is composed from a name the vocabulary never
//! carries — the one exception `role_denied_text` already documents.
//!
//! None of this belongs in `render(&state, Site::…)`: it is a property of the
//! **check**, not of a package's state, and the row goes on showing its cached state
//! label underneath.

use leptos::prelude::*;

use crate::Cell;
use crate::Scene;
use crate::kit::Button;
use crate::kit::ButtonVariant;
use crate::kit::Card;
use crate::kit::CauseRow;
use crate::kit::PackageRow;
use crate::kit::QueueRow;
use crate::kit::StateTone;

/// Both scenes sit under one index entry, so this is the only anchor either of them
/// has. A row is a real link and needs a real destination even here.
const ANCHOR: &str = "#scene--a-check-that-failed";

const MINUTE: f64 = 60_000.0;
const HOUR: f64 = 60.0 * MINUTE;

fn ago(ms: f64) -> f64 {
    js_sys::Date::now() - ms
}

/// The packages whose check failed. Their cached states differ on purpose: the
/// failure is not a state, so a fixture where every row said the same thing would
/// hide that the label underneath is still each package's own.
const UNCHECKED: &[(&str, &str, StateTone, f64)] = &[
    ("user/package-a", "Latest", StateTone::Success, 2.0 * HOUR),
    (
        "user/package-b",
        "Not the latest",
        StateTone::Attention,
        20.0 * MINUTE,
    ),
    (
        "team/dataset-f",
        "2 files changed",
        StateTone::Neutral,
        5.0 * HOUR,
    ),
];

fn try_again() -> AnyView {
    view! {
        <Button variant=ButtonVariant::Default on_click=|_| ()>
            "Try again"
        </Button>
    }
    .into_any()
}

fn unchecked_rows() -> AnyView {
    UNCHECKED
        .iter()
        .map(|&(namespace, ..)| view! { <QueueRow namespace=namespace sub=true /> })
        .collect_view()
        .into_any()
}

/// The list as it stands beneath the cause — cached states, dashed and dimmed.
///
/// Present in both scenes because on the real page both are on screen at once: the
/// cause explains, the rows stay honest about what is unconfirmed. They are two
/// channels, not rivals.
fn dimmed_list(repeats: usize) -> AnyView {
    view! {
        <Card>
            {(0..repeats)
                .flat_map(|_| UNCHECKED.iter())
                .map(|&(namespace, state, tone, elapsed)| {
                    view! {
                        <PackageRow
                            namespace=namespace
                            href=ANCHOR
                            changed_at=ago(elapsed)
                            state=state
                            tone=tone
                            provisional=true
                        />
                    }
                })
                .collect_view()}
        </Card>
    }
    .into_any()
}

#[component]
pub fn UncheckedScene() -> impl IntoView {
    view! { <SomeFailed /><AllFailed /> }
}

#[component]
fn SomeFailed() -> impl IntoView {
    let unchecked = RwSignal::new(false);
    let expanded = RwSignal::new(true);
    let signed_out = RwSignal::new(false);
    let single = RwSignal::new(false);

    view! {
        <Scene
            title="Scene · a check that failed"
            note="One cause, host-grouped, in the region that already exists to name a \
                  shared cause once and carry its remedy. Read the first cell against the \
                  signed-out row directly under it: same grammar, same slot, same kind of \
                  sentence — which is the argument for putting it here rather than in a bar \
                  above a region that does this job. \
                  \
                  Expand it and the packages are named, so the sentence and the list it \
                  speaks for cannot disagree. The count is CauseRow's, derived from those \
                  members; nothing here writes a number. \
                  \
                  [Try again] re-checks only these packages. That is what the appbar's \
                  Refresh cannot do — it reloads the page — and it is the affordance v1 had \
                  in a per-row menu this design deliberately removed. \
                  \
                  Below the region, the rows themselves stay dashed and dimmed. Both \
                  channels are on screen at once on the real page: the cause explains, the \
                  rows stay honest about which states are unconfirmed. Switch to dark and \
                  watch how much the dim alone is carrying — 1.73:1 against a settled row, \
                  which is why it is not carrying this by itself."
        >
            <Cell full=true label="the cause, beside the one it is modelled on">
                <Card title="Needs your attention" count=14>
                    <div>
                        <CauseRow
                            text="Couldn't check for new revisions on open.quiltdata.com"
                            count=3
                            expanded=unchecked
                            trailing=try_again()
                        />
                        <Show when=move || unchecked.get()>{unchecked_rows()}</Show>
                        <CauseRow
                            text="Signed out from custom.registry.io"
                            count=11
                            expanded=signed_out
                            trailing=view! {
                                <Button on_click=|_| ()>
                                    "Sign in"
                                </Button>
                            }
                                .into_any()
                        />
                    </div>
                </Card>
            </Cell>
            <Cell full=true label="expanded — the sentence and the packages it speaks for">
                <Card title="Needs your attention" count=3>
                    <div>
                        <CauseRow
                            text="Couldn't check for new revisions on open.quiltdata.com"
                            count=3
                            expanded=expanded
                            trailing=try_again()
                        />
                        <Show when=move || expanded.get()>{unchecked_rows()}</Show>
                    </div>
                </Card>
            </Cell>
            <Cell full=true label="one package — CauseRow's own singular">
                <Card title="Needs your attention" count=1>
                    <div>
                        <CauseRow
                            text="Couldn't check for new revisions on open.quiltdata.com"
                            count=1
                            expanded=single
                            trailing=try_again()
                        />
                    </div>
                </Card>
            </Cell>
            <Cell full=true label="and the rows underneath, unconfirmed in two channels">
                {dimmed_list(1)}
            </Cell>
        </Scene>
    }
}

#[component]
fn AllFailed() -> impl IntoView {
    let expanded = RwSignal::new(false);

    view! {
        <Scene
            title="Scene · offline — nothing could be checked"
            note="The case this scene exists for, and the likely one: no network, so every \
                  call rejects and not one row is confirmed. \
                  \
                  Without a cause row the region is not merely quiet, it is ABSENT — the \
                  queue returns nothing twice over, once on an empty settled list and once \
                  because no package is accounted for, so there is no card, no heading and \
                  no zero line. The reader is left with a page of grey rows and no \
                  statement. Words on a row would not bring the region back, which is what \
                  settled this between the two candidate homes. \
                  \
                  Here one sentence accounts for the whole page, standing exactly where \
                  'Everything is Latest' stands on a good day — and saying the opposite of \
                  it, which is the point: an all-clear over packages the app could not read \
                  is the failure this region has already been fixed for once."
        >
            <Cell full=true label="one sentence for the whole page">
                <Card title="Needs your attention" count=6>
                    <div>
                        <CauseRow
                            text="Couldn't check for new revisions on open.quiltdata.com"
                            count=6
                            expanded=expanded
                            trailing=try_again()
                        />
                        <Show when=move || expanded.get()>
                            {(0..2).flat_map(|_| UNCHECKED.iter())
                                .map(|&(namespace, ..)| {
                                    view! { <QueueRow namespace=namespace sub=true /> }
                                })
                                .collect_view()}
                        </Show>
                    </div>
                </Card>
            </Cell>
            // A second cell rather than a second card in the same one: `.g-cell__body`
            // is a flex ROW, so two cards inside one cell sit shoulder to shoulder and
            // claim a composition the page does not have — the region is ABOVE the
            // list. Consecutive full-width cells stack, with the grid's gap standing in
            // for the one `PageLayout` owns between regions.
            <Cell full=true label="and the whole list beneath it, every row unconfirmed">
                {dimmed_list(2)}
            </Cell>
        </Scene>
    }
}
