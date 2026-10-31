//! The queue's components: `ZeroLine`, `QueueRow` and `CauseRow`.

use leptos::prelude::*;

use crate::Cell;
use crate::Story;
use crate::kit::Button;
use crate::kit::CauseRow;
use crate::kit::PackageAction;
use crate::kit::PackageState;
use crate::kit::QueueRow;
use crate::kit::Remedy;
use crate::kit::Site;
use crate::kit::StateTone;
use crate::kit::ZeroLine;
use crate::kit::ZeroLineSkeleton;
use crate::kit::render;

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
pub(crate) fn actionable() -> Vec<(&'static str, PackageState)> {
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
pub(crate) const REJECTION: &str = concat!(
    "package does not satisfy the workflow:\n",
    "  - a commit message is required by this workflow, but none was provided\n",
    "  - package name \"team/imaging-cohort-b\" does not match the required ",
    "handle_pattern \"^[a-z]+/[a-z]+-[0-9]{4}$\"",
);

/// The other shape: one line, from a malformed `.quilt/workflows/config.yml`.
const BAD_CONFIG: &str = "Invalid workflows config: missing required key 'version'";

/// A row as the page builds one: the vocabulary's words, and the remedy
/// pointing where `href` says. Nothing here is hand-written, so a gallery row
/// cannot say something the app would not.
pub(crate) fn queue_row(
    namespace: &'static str,
    state: &PackageState,
    detail: Option<String>,
    href: fn(PackageAction, &'static str) -> String,
) -> AnyView {
    let rendered = render(state, Site::QueueRow);
    let remedy = rendered.action.map(|action| Remedy {
        action,
        href: href(action, namespace),
    });
    view! {
        <QueueRow
            namespace=namespace
            state=rendered.words
            remedy=remedy
            detail=detail
        />
    }
    .into_any()
}

/// The routes are the app's, so a story's remedy points at its own section.
fn story_href(_: PackageAction, _: &'static str) -> String {
    "#/combined/queue-parts".to_string()
}

pub(crate) fn row(namespace: &'static str, state: &PackageState) -> AnyView {
    queue_row(namespace, state, None, story_href)
}

fn detailed_row(namespace: &'static str, state: &PackageState, detail: Option<String>) -> AnyView {
    queue_row(namespace, state, detail, story_href)
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
            note="The queue's status line. On a calm day it is the whole region, and it \
                  must stay one line (acceptance criterion 8) — a full-height empty state \
                  here would push the package list below the fold in order to say that \
                  nothing is wrong. With rows, the same line in the attention tone names \
                  them. ZeroLineSkeleton holds its place while checks are out, at the \
                  same height."
        >
            <Cell full=true label="the healthy queue">
                <ZeroLine text="All packages are up to date" />
            </Cell>
            <Cell full=true label="still checking — the placeholder directly above the line it becomes">
                <ZeroLineSkeleton />
                <ZeroLine text="All packages are up to date" />
            </Cell>
            <Cell full=true label="attention — the line above the queue's rows">
                <ZeroLine text="3 packages need your attention" tone=StateTone::Attention />
            </Cell>
            <Cell wide=true label="narrow · long text — truncates rather than wrapping">
                <ZeroLine text="All packages are up to date across 4 buckets and 2 hosts" />
            </Cell>
        </Story>
    }
}

#[component]
fn QueueRowStory() -> impl IntoView {
    view! {
        <Story
            title="QueueRow"
            note="The row is the link: every verb here opens a page, so a button would \
                  promise an operation that does not happen on press. The verb stays as text \
                  at the right edge, one tab stop per row. The state reads as a clause after \
                  the name rather than as a chip beside it, and the row draws no colour for \
                  it: the clause says the state in words."
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
            <Cell full=true label="sub-row — a bare name: no state, no remedy, indented, and the one row that keeps a bullet">
                <QueueRow namespace="team/rnaseq-batch-2026-07-31" sub=true />
            </Cell>
            <Cell full=true label="long namespace — name and clause share the line while both fit">
                {row(
                    "team/rnaseq-batch-2026-07-31-reprocessed-v2-with-a-very-long-suffix",
                    &PackageState::Diverged,
                )}
            </Cell>
            <Cell wide=true label="narrow · long namespace — the name ellipsises only past a whole line">
                {row(
                    "team/rnaseq-batch-2026-07-31-reprocessed-v2-with-a-very-long-suffix",
                    &PackageState::PendingChanges { files: 2 },
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
            note="One component with a trailing slot, not two components: both appearances \
                  name a cause, count its packages and let you see which, and only the \
                  slot's contents differ. Role-denied carries a pointer line and no control, \
                  because the fix is host-scoped and the control lives on the host row. \
                  These rows wrap rather than truncate, unlike every other row in the kit — \
                  a cause is a sentence, and the specifics are at the end. The expanders are \
                  live."
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
                    text="No access as analyst on data-catalog-eu-west-1.example.com, \
                          in s3://acme-research-eu-west-1-in-progress"
                    count=14
                    expanded=long
                    trailing=view! { "Change your role in Accounts, above." }.into_any()
                />
            </Cell>
        </Story>
    }
}
