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
use crate::kit::Card;
use crate::kit::CauseRow;
use crate::kit::PackageState;
use crate::kit::QueueRow;
use crate::kit::Remedy;
use crate::kit::Site;
use crate::kit::ZeroLine;
use crate::kit::render;
use quilt_sync_ui::pages::action_href;

/// The five states that name an operation, in precedence order. Real states rather
/// than hand-written words: the gallery draws them through `render`, so a row here
/// cannot say something the app would not.
///
/// Two of the pairings are corrections worth re-reading rather than trusting from
/// memory, and both live in `kit/package_state.rs` where the verbs are chosen:
///
/// - a pull conflict offers **`Publish`**, not `Resolve`. It is not resolvable on
///   the merge page until the changes are committed, so publishing commits the local
///   version and lands the package in `Diverged` — at which point the row says
///   `changed in both places` and *does* offer `Resolve`.
/// - `Diverged` offers **`Resolve`**, not `Merge`. No merge operation exists:
///   resolving is a binary package-level choice, and a button labelled Merge promises
///   git semantics the product deliberately does not have.
fn actionable() -> Vec<(&'static str, PackageState)> {
    vec![
        (
            "org/dataset-c",
            PackageState::PullConflict {
                files: vec!["a.csv".to_string(), "b.csv".to_string()],
            },
        ),
        ("team/dataset-f", PackageState::Diverged),
        ("user/package-e", PackageState::Behind),
        ("user/package-b", PackageState::PendingChanges { files: 2 }),
        ("local/my-data", PackageState::NoRemote),
    ]
}

/// A workflow rejection, verbatim. `WorkflowValidationError::Rejected` renders its
/// violations one per line, so the row has to keep the newlines: flattened, the reader
/// cannot tell one broken rule from the next.
const REJECTION: &str = concat!(
    "package does not satisfy the workflow:\n",
    "  - a commit message is required by this workflow, but none was provided\n",
    "  - package name \"team/imaging-cohort-b\" does not match the required ",
    "handle_pattern \"^[a-z]+/[a-z]+-[0-9]{4}$\"",
);

/// The other shape: one line, from a malformed `.quilt/workflows/config.yml`.
const BAD_CONFIG: &str = "Invalid workflows config: missing required key 'version'";

/// A row as the page builds one — the vocabulary's words and tone, and the remedy
/// pointing wherever the page would point it. Nothing here is hand-written, so a
/// gallery row cannot say something the app would not.
pub(crate) fn row(namespace: &'static str, state: &PackageState) -> AnyView {
    detailed_row(namespace, state, None)
}

/// The same, with the second line only a pause carries.
pub(crate) fn detailed_row(
    namespace: &'static str,
    state: &PackageState,
    detail: Option<String>,
) -> AnyView {
    let rendered = render(state, Site::QueueRow);
    let remedy = rendered.action.map(|action| Remedy {
        action,
        href: action_href(action, namespace),
    });
    view! {
        <QueueRow
            namespace=namespace
            state=rendered.words
            tone=rendered.tone
            remedy=remedy
            detail=detail
        />
    }
    .into_any()
}

#[component]
pub fn QueueStories() -> impl IntoView {
    view! { <ZeroLineStory /><QueueRowStory /><CauseRowStory /> }
}

#[component]
fn ZeroLineStory() -> impl IntoView {
    view! {
        <Story
            title="ZeroLine"
            note="The state most users see most days, and it must stay one line \
                  (acceptance criterion 8) — a full-height empty state here would push the \
                  package list below the fold in order to say that nothing is wrong. \
                  \
                  The region's heading and count are `Card`'s, not a component of their \
                  own: a `SectionLabel` existed here briefly and was deleted once the \
                  regions became cards, because Card's title already had exactly that \
                  treatment. See the Card story for the count states."
        >
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
            note="The row IS the link. Every verb here opens a page — Publish the commit \
                  page, Resolve the merge page, the other two the package's own — so a \
                  button would promise an operation that does not happen on press. The \
                  verb stays as text at the right edge, one tab stop per row, and the \
                  accent this page never spends stays unspent. \
                  \
                  The state reads as a clause after the name rather than as a chip beside \
                  it: name at weight 600 in default ink, clause muted, which is the \
                  treatment HostRow already uses. The tone that was the chip's job now \
                  draws twice — a rule on the row's edge, inset so a column of rows shows \
                  separate marks rather than one band, and the tone's glyph in the leading \
                  column. Two channels, because the four tone hues are lightness-matched \
                  and the rule alone is four identical greys in greyscale. \
                  \
                  That leading column is CauseRow's expander column, which is what keeps a \
                  cause and a package aligned on their text. A row with no state of its \
                  own gets a plain bullet there instead."
        >
            {actionable()
                .into_iter()
                .map(|(namespace, state)| {
                    let label = render(&state, Site::QueueRow)
                        .action
                        .expect("every state in this table names an operation")
                        .label();
                    view! { <Cell full=true label=label>{row(namespace, &state)}</Cell> }
                })
                .collect_view()}
            <Cell full=true label="no operation — the row is inert, and does not pretend to link">
                {row("org/dataset-x", &PackageState::Unknown)}
            </Cell>
            <Cell full=true label="Sync paused — the engine's own words, kept verbatim">
                {detailed_row(
                    "team/imaging-cohort-b",
                    &PackageState::Paused,
                    Some(REJECTION.to_string()),
                )}
            </Cell>
            <Cell full=true label="Sync paused — a one-line reason">
                {detailed_row("org/dataset-c", &PackageState::Paused, Some(BAD_CONFIG.to_string()))}
            </Cell>
            <Cell wide=true label="narrow · the rejection wraps, and keeps its own line breaks">
                {detailed_row(
                    "team/imaging-cohort-b",
                    &PackageState::Paused,
                    Some(REJECTION.to_string()),
                )}
            </Cell>
            <Cell full=true label="sub-row — no state, no remedy, indented, and a bullet rather than a glyph">
                <QueueRow namespace="team/rnaseq-batch-2026-07-31" sub=true />
            </Cell>
            <Cell full=true label="long namespace truncates before the clause does">
                {row(
                    "team/rnaseq-batch-2026-07-31-reprocessed-v2-with-a-very-long-suffix",
                    &PackageState::Diverged,
                )}
            </Cell>
            <Cell wide=true label="narrow — two columns">
                {row("local/my-data", &PackageState::NoRemote)}
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
                    trailing=view! {
                        <Button on_click=|_| ()>
                            "Sign in"
                        </Button>
                    }
                        .into_any()
                />
            </Cell>
            <Cell full=true label="expanded — the caller renders what it reveals">
                <div class="g-rows">
                    <CauseRow
                        text="No access as analyst on custom.registry.io, 3 packages in s3://team-bucket"
                        count=3
                        expanded=role
                        trailing=view! { "Change your role in Accounts, above." }.into_any()
                    />
                    <Show when=move || role.get()>
                        <QueueRow namespace="team/rnaseq-batch-2026-07-31" sub=true />
                        <QueueRow namespace="team/imaging-cohort-b" sub=true />
                        <QueueRow namespace="team/spatial-pilot" sub=true />
                    </Show>
                </div>
            </Cell>
            <Cell full=true label="count of one — singular">
                <CauseRow
                    text="Signed out from open.quiltdata.com"
                    count=1
                    expanded=single
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
                    trailing=view! { "Change your role in Accounts, above." }.into_any()
                />
            </Cell>
        </Story>
    }
}

/// The region itself, so the whole-page scene composes this code rather than a copy.
#[component]
pub fn QueueRegion() -> impl IntoView {
    let signed_out = RwSignal::new(false);
    let role = RwSignal::new(false);

    // Derived, never written — 11 + 3 + 5 + the paused row. The mock's hand-written
    // (17) is off against its own rows, which is what this closure exists to avoid.
    // Bound once rather than rebuilt on every read of the card's count: `actionable`
    // allocates.
    let actionable = actionable();
    let total = 11 + 3 + actionable.len() + 1;

    view! {
        // One wrapper child, so `Card`'s between-children hairline does not fire: a
        // queue is a list of decisions, and dividing every row would make it read as a
        // table of data.
        <Card title="Needs your attention" count=total>
            <div>
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
                    trailing=view! { "Change your role in Accounts, above." }.into_any()
                />
                <Show when=move || role.get()>
                    {["team/rnaseq-batch-2026-07-31", "team/imaging-cohort-b", "team/spatial-pilot"]
                        .into_iter()
                        .map(|namespace| view! { <QueueRow namespace=namespace sub=true /> })
                        .collect_view()}
                </Show>
                {actionable
                    .iter()
                    .take(1)
                    .map(|(namespace, state)| row(namespace, state))
                    .collect_view()}
                // Precedence row 3, between the conflict above and everything below,
                // and the only row here that goes nowhere: nothing in the app can
                // restart a sync the remote refused.
                {detailed_row(
                    "team/imaging-cohort-b",
                    &PackageState::Paused,
                    Some(REJECTION.to_string()),
                )}
                {actionable
                    .iter()
                    .skip(1)
                    .map(|(namespace, state)| row(namespace, state))
                    .collect_view()}
            </div>
        </Card>
    }
}

#[component]
pub fn QueueScene() -> impl IntoView {
    view! {
        <Scene
            title="Scene · needs your attention"
            note="Shared causes first, then per-package rows in precedence order. Both \
                  expanders work — open them and watch what nineteen items actually costs \
                  in vertical space, because the region above the package list is the \
                  thing this design spends to buy. \
                  \
                  Read down the verbs: five of them, one per row, each true of the row it \
                  sits on. That column is what replaces 43 rows of Publish. Each row is a \
                  link to the page that performs its verb, so the verb is text and not a \
                  button — nothing here acts on press. \
                  \
                  Read down the left edge too: the tone marks band rather than scatter, \
                  because the severe states sort first and Latest never enters the queue. \
                  \
                  The paused row is the exception and costs the most height: it carries \
                  the engine's own rejection and goes nowhere, because no operation here \
                  can restart a sync the remote refused. It is allowed to take the room — \
                  that text is the only account of why the package stopped."
        >
            <QueueRegion />
        </Scene>
        <Scene
            title="Scene · nothing needs you"
            note="The same region on a working day, which is the common case with autosync \
                  on. One line, no count — counting to zero would be noise — and the \
                  package list starts immediately below rather than a screen down."
        >
            <Card title="Needs your attention">
                <ZeroLine text="Everything is Latest — 43 packages" />
            </Card>
        </Scene>
    }
}
