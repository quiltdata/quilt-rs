//! Queue components and the "Needs your attention" scene.
//!
//! This is the region the whole redesign is for, so the scene carries the load: the
//! stories prove each row, and only the scene shows whether nineteen rows of things
//! needing decisions read as a queue or as a pile.

use leptos::prelude::*;

use crate::Cell;
use crate::Scene;
use crate::Story;
use crate::kit::Button;
use crate::kit::ButtonVariant;
use crate::kit::CauseRow;
use crate::kit::QueueRow;
use crate::kit::SectionLabel;
use crate::kit::StateTone;
use crate::kit::ZeroLine;

/// The six actions, each paired with the state that produces it. This table *is* the
/// precedence lattice's visible half, and two of the pairings are corrections worth
/// re-reading rather than trusting from memory:
///
/// - `conflicts in N files` offers **`Publish`**, not `Resolve`. A pull conflict is
///   not resolvable on the merge page until the changes are committed, so publishing
///   commits the local version and lands the package in `Diverged` — at which point
///   the row becomes `Changed in both places` and *does* offer `Resolve`. Two steps,
///   each one click, with the state explicit at both.
/// - `Changed in both places` offers **`Resolve`**, not `Merge`. No merge operation
///   exists: resolving is a binary package-level choice, and a button labelled Merge
///   promises git semantics the product deliberately does not have.
const ACTIONS: &[(&str, &str, StateTone, &str)] = &[
    ("org/dataset-c", "conflicts in 2 files", StateTone::Danger, "Publish"),
    ("team/dataset-f", "Changed in both places", StateTone::Danger, "Resolve"),
    ("user/package-e", "Newer revision available", StateTone::Attention, "Get latest"),
    ("user/package-b", "2 files changed", StateTone::Neutral, "Publish"),
    ("local/my-data", "No S3 bucket yet", StateTone::Attention, "Choose S3 bucket"),
];

fn action(label: &'static str, variant: ButtonVariant) -> AnyView {
    view! {
        <Button variant=variant on_click=|_| ()>
            {label}
        </Button>
    }
    .into_any()
}

#[component]
pub fn QueueStories() -> impl IntoView {
    view! { <LabelsStory /><QueueRowStory /><CauseRowStory /> }
}

#[component]
fn LabelsStory() -> impl IntoView {
    view! {
        <Story
            title="SectionLabel · ZeroLine"
            note="The label is sentence case in the source and upper-cased in CSS — typing \
                  it in capitals would make some screen readers spell it as an initialism. \
                  Its count must be DERIVED: the design mock reads (17) above 19 rows, \
                  which is what a hand-written count does the first time the rows change. \
                  \
                  ZeroLine is the state most users see most days, and it must stay one \
                  line (acceptance criterion 8) — a full-height empty state here would \
                  push the package list below the fold in order to say nothing is wrong."
        >
            <Cell label="with a count">
                <SectionLabel text="Needs your attention" count=19 />
            </Cell>
            <Cell label="count of one">
                <SectionLabel text="Needs your attention" count=1 />
            </Cell>
            <Cell label="no count">
                <SectionLabel text="Needs your attention" />
            </Cell>
            <Cell full=true label="the healthy queue">
                <ZeroLine text="Everything is Latest — 43 packages" />
            </Cell>
            <Cell full=true label="singular">
                <ZeroLine text="Everything is Latest — 1 package" />
            </Cell>
            <Cell wide=true label="narrow · long text — truncates rather than wrapping">
                <ZeroLine text="Everything is Latest — 43 packages across 4 buckets and 2 hosts" />
            </Cell>
        </Story>
    }
}

#[component]
fn QueueRowStory() -> impl IntoView {
    view! {
        <Story
            title="QueueRow"
            note="The only row in the design that carries a text button, and it is the \
                  payoff for the rule that stripped buttons off the list rows rather than \
                  an exception to it: a queue row exists BECAUSE the package needs the \
                  action, so the button and the row are the same fact. On today's list, \
                  Publish renders on 43 rows and applies to two. \
                  \
                  The row does not navigate — the list below is where you go to a \
                  package, the queue is where you decide about one — so there is no hover \
                  tint promising otherwise, and one tab stop per row, which is the button. \
                  Actions sit in a fixed 152px column, wide enough for Choose S3 bucket, \
                  so the buttons have a straight left edge to read down."
        >
            {ACTIONS
                .iter()
                .map(|&(namespace, state, tone, label)| {
                    view! {
                        <Cell full=true label=label>
                            <QueueRow
                                namespace=namespace
                                state=state
                                tone=tone
                                action=action(label, ButtonVariant::Default)
                            />
                        </Cell>
                    }
                })
                .collect_view()}
            <Cell full=true label="Sign in — the sixth action, when a host is a whole cause">
                <QueueRow
                    namespace="custom.registry.io"
                    state="No access"
                    tone=StateTone::Danger
                    action=action("Sign in", ButtonVariant::Default)
                />
            </Cell>
            <Cell full=true label="primary action — the caller decides, the row does not">
                <QueueRow
                    namespace="user/package-b"
                    state="2 files changed"
                    tone=StateTone::Neutral
                    action=action("Publish", ButtonVariant::Primary)
                />
            </Cell>
            <Cell full=true label="action disabled — a pull check in flight">
                <QueueRow
                    namespace="user/package-e"
                    state="Newer revision available"
                    tone=StateTone::Attention
                    action=view! {
                        <Button on_click=|_| () disabled=true>
                            "Get latest"
                        </Button>
                    }
                        .into_any()
                />
            </Cell>
            <Cell full=true label="sub-row — no state, no action, indented">
                <QueueRow namespace="team/rnaseq-batch-2026-07-31" sub=true />
            </Cell>
            <Cell full=true label="long namespace truncates, the action keeps its column">
                <QueueRow
                    namespace="team/rnaseq-batch-2026-07-31-reprocessed-v2-with-a-very-long-suffix"
                    state="Changed in both places"
                    tone=StateTone::Danger
                    action=action("Resolve", ButtonVariant::Default)
                />
            </Cell>
            <Cell wide=true label="narrow — two columns">
                <QueueRow
                    namespace="local/my-data"
                    state="No S3 bucket yet"
                    tone=StateTone::Attention
                    action=action("Choose S3 bucket", ButtonVariant::Default)
                />
            </Cell>
        </Story>
    }
}

#[component]
fn CauseRowStory() -> impl IntoView {
    let signed_out = RwSignal::new(false);
    let role = RwSignal::new(true);
    let single = RwSignal::new(false);
    let long = RwSignal::new(false);

    view! {
        <Story
            title="CauseRow"
            note="One component with a trailing slot, not two components. Both appearances \
                  do the same job — name a cause, count its packages, let you see which — \
                  and only the slot's contents differ, which is data. The kit bans the \
                  other kind of flag: one that would change the row's job, as \
                  \"is the row clickable\" did for the two list rows. \
                  \
                  Role-denied carries a pointer line and no control on purpose. It is \
                  fixed by switching role, which is host-scoped, so the control lives on \
                  the host row in the Accounts card. A link may be duplicated across \
                  scopes; a control may not, because the same control at two \
                  granularities makes one of them a lie. \
                  \
                  These rows WRAP rather than truncate, unlike every other row in the \
                  kit — a cause is a sentence and the end of it is where the specifics \
                  are. The expanders are live; the packages they reveal belong to the \
                  caller, which is why `expanded` is passed in."
        >
            <Cell full=true label="collapsed with an action">
                <CauseRow
                    text="Signed out from custom.registry.io"
                    count=11
                    expanded=signed_out
                    tone=StateTone::Danger
                    trailing=view! {
                        <Button on_click=|_| ()>
                            "Sign in"
                        </Button>
                    }
                        .into_any()
                />
            </Cell>
            <Cell full=true label="expanded — the caller renders what it reveals">
                <CauseRow
                    text="No access as analyst on custom.registry.io, 3 packages in s3://team-bucket"
                    count=3
                    expanded=role
                    tone=StateTone::Attention
                    trailing=view! { "Change your role in Accounts, above." }.into_any()
                />
                <Show when=move || role.get()>
                    <QueueRow namespace="team/rnaseq-batch-2026-07-31" sub=true />
                    <QueueRow namespace="team/imaging-cohort-b" sub=true />
                    <QueueRow namespace="team/spatial-pilot" sub=true />
                </Show>
            </Cell>
            <Cell full=true label="count of one — singular">
                <CauseRow
                    text="Signed out from open.quiltdata.com"
                    count=1
                    expanded=single
                    tone=StateTone::Danger
                    trailing=view! {
                        <Button on_click=|_| ()>
                            "Sign in"
                        </Button>
                    }
                        .into_any()
                />
            </Cell>
            <Cell wide=true label="narrow · long cause — wraps, keeps the glyph at the top">
                <CauseRow
                    text="No access as analyst on quilt-enterprise-eu-west-1.example.com, \
                          in s3://quilt-enterprise-eu-west-1-in-progress"
                    count=14
                    expanded=long
                    tone=StateTone::Attention
                    trailing=view! { "Change your role in Accounts, above." }.into_any()
                />
            </Cell>
        </Story>
    }
}

#[component]
pub fn QueueScene() -> impl IntoView {
    let signed_out = RwSignal::new(false);
    let role = RwSignal::new(false);

    // Derived, never written — 11 + 3 + 5. The mock's hand-written (17) is off by two
    // against its own rows, which is the failure mode this closure exists to avoid.
    let total = move || 11 + 3 + ACTIONS.len();

    view! {
        <Scene
            title="Scene · needs your attention"
            note="Shared causes first, then per-package rows in precedence order. Both \
                  expanders work — open them and watch what nineteen items actually costs \
                  in vertical space, because the region above the package list is the \
                  thing this design spends to buy. \
                  \
                  Read down the buttons: five different verbs, one per row, each true of \
                  the row it sits on. That column is what replaces 43 rows of Publish."
        >
            <SectionLabel text="Needs your attention" count=total() />
            <CauseRow
                text="Signed out from custom.registry.io"
                count=11
                expanded=signed_out
                tone=StateTone::Danger
                trailing=view! {
                    <Button on_click=|_| ()>
                        "Sign in"
                    </Button>
                }
                    .into_any()
            />
            <Show when=move || signed_out.get()>
                {["user/package-x", "user/package-y", "org/shared-set"]
                    .into_iter()
                    .map(|namespace| view! { <QueueRow namespace=namespace sub=true /> })
                    .collect_view()}
                <QueueRow namespace="…and 8 more" sub=true />
            </Show>
            <CauseRow
                text="No access as analyst on custom.registry.io, 3 packages in s3://team-bucket"
                count=3
                expanded=role
                tone=StateTone::Attention
                trailing=view! { "Change your role in Accounts, above." }.into_any()
            />
            <Show when=move || role.get()>
                {["team/rnaseq-batch-2026-07-31", "team/imaging-cohort-b", "team/spatial-pilot"]
                    .into_iter()
                    .map(|namespace| view! { <QueueRow namespace=namespace sub=true /> })
                    .collect_view()}
            </Show>
            {ACTIONS
                .iter()
                .map(|&(namespace, state, tone, label)| {
                    view! {
                        <QueueRow
                            namespace=namespace
                            state=state
                            tone=tone
                            action=action(label, ButtonVariant::Default)
                        />
                    }
                })
                .collect_view()}
        </Scene>
        <Scene
            title="Scene · nothing needs you"
            note="The same region on a working day, which is the common case with autosync \
                  on. One line, no count — counting to zero would be noise — and the \
                  package list starts immediately below rather than a screen down."
        >
            <SectionLabel text="Needs your attention" />
            <ZeroLine text="Everything is Latest — 43 packages" />
        </Scene>
    }
}
