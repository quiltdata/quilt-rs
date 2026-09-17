//! The installed-package page's header, in every state the vocabulary can reach.
//!
//! # Why a scene and not a component
//!
//! `PageHeader` is page-level and not built. This draws the header's *shape* —
//! trail, identity, one state label, one primary action, `Open folder`, `[⋯]` —
//! from the kit pieces that do exist, so the states can be read against each
//! other before the region is written. The same reasoning the component record
//! applies to `DegradedBand`: the degraded states are not a component, they are
//! the header with different props, and the place to see that is here.
//!
//! # The words are not chosen here
//!
//! Every label, tone and action comes from `render(state, site)` — the one place
//! the vocabulary lives. A scene that hand-wrote its own strings would be a
//! drawing of the header rather than a rendering of it, and would keep agreeing
//! with itself after the vocabulary moved. `queue.rs` set that precedent.
//!
//! **The site is `PageHeader`**, which this scene is what settled. The header
//! borrows the list's chip words for every state but one: `Behind` reads
//! `Newer revision available` here and `Not the latest` in the list. A list row is
//! read while scanning many packages, where the useful fact is how this one sorts
//! against its neighbours; a header is read having already chosen the package,
//! where the useful fact is that there is something to fetch. That single
//! divergence is the whole reason the site exists, and a test in
//! `package_state.rs` holds it to exactly one.
//!
//! # Three states are missing, and cannot be added yet
//!
//! Signed out, sign-in expired and unreachable are three separate rows in the
//! design's header table and one `error` string in today's backend — the
//! "three failures the page cannot currently tell apart". None is a
//! `PackageState`, so none can be drawn until the `Blocked` value lands on the
//! DTO. The cells below are what exists, not what the page will finally show.

use leptos::prelude::*;

use crate::Cell;
use crate::Scene;
use crate::kit::ActionMenu;
use crate::kit::ActionTone;
use crate::kit::BackLink;
use crate::kit::Button;
use crate::kit::ButtonVariant;
use crate::kit::MenuAction;
use crate::kit::PackageAction;
use crate::kit::PackageState;
use crate::kit::Site;
use crate::kit::SplitButton;
use crate::kit::StateLabel;
use crate::kit::render;

const NAMESPACE: &str = "user/plate-07";

/// Every state the vocabulary can reach, with the condition spelled out rather
/// than the variant named — the label is what makes a cell reviewable by
/// somebody who does not already know the enum.
///
/// `PendingChanges` and `PullConflict` appear twice because they interpolate a
/// count and the singular is written by hand; a plural-only fixture would not
/// exercise the arm that says `1 file`.
///
/// `RoleDenied` appears once. It carries an optional role, but the list site
/// never names it — `No access` either way — so a second cell would draw the
/// same header twice.
fn states() -> Vec<(&'static str, PackageState)> {
    vec![
        ("nothing to do", PackageState::Latest),
        ("a newer revision exists upstream", PackageState::Behind),
        (
            "one local file differs",
            PackageState::PendingChanges { files: 1 },
        ),
        (
            "several local files differ",
            PackageState::PendingChanges { files: 4 },
        ),
        ("committed locally, never sent", PackageState::PendingCommit),
        ("changed here and upstream, both", PackageState::Diverged),
        (
            "one file came back conflicted",
            PackageState::PullConflict {
                files: vec!["data/plate.csv".to_string()],
            },
        ),
        (
            "several files came back conflicted",
            PackageState::PullConflict {
                files: vec![
                    "data/plate.csv".to_string(),
                    "data/wells.csv".to_string(),
                    "notes.md".to_string(),
                ],
            },
        ),
        (
            "the active role cannot read the bucket",
            PackageState::RoleDenied {
                role: Some("analyst".to_string()),
            },
        ),
        ("no bucket chosen yet", PackageState::NoRemote),
        ("has a bucket, never published", PackageState::Unpublished),
        ("syncing stopped for this package", PackageState::Paused),
        (
            "a state this build does not recognise",
            PackageState::Unknown,
        ),
    ]
}

/// The package-level commands. Fixed across states on purpose: the header's
/// menu is where everything that is *not* the one primary action lives, so it
/// does not change shape as the state does.
fn menu() -> Vec<MenuAction> {
    vec![
        MenuAction {
            label: "Open in catalog".to_string(),
            tone: ActionTone::Default,
            disabled: None,
            on_select: Callback::new(|()| ()),
            separated: false,
        },
        MenuAction {
            label: "Change bucket".to_string(),
            tone: ActionTone::Default,
            disabled: None,
            on_select: Callback::new(|()| ()),
            separated: false,
        },
        MenuAction {
            label: "Undo last revision".to_string(),
            tone: ActionTone::Danger,
            disabled: None,
            on_select: Callback::new(|()| ()),
            separated: true,
        },
        MenuAction {
            label: "Remove".to_string(),
            tone: ActionTone::Danger,
            disabled: None,
            on_select: Callback::new(|()| ()),
            separated: false,
        },
    ]
}

/// The header at one state.
///
/// # `/commit` is one click away, not buried
///
/// `Create new revision` is the way to the commit page. v1 has it as a visible
/// peer of publishing — `[Create new revision] or [Commit and Push]`, `primary`
/// swapping by context — so filing it under `[⋯]` would take away a button
/// people already use.
///
/// Two earlier arrangements were rejected by looking at them. `[⋯]` buried it.
/// A literal `or` between two buttons worked against `Publish` and produced
/// `Create new revision or Get latest` against everything else, which claims a
/// relationship that is not there. A [`SplitButton`](crate::kit::SplitButton)
/// says the same thing structurally and only where it is true: the caret holds
/// the other way to do *this* command, so it appears only when the command has
/// another way.
///
/// So `Publish` is a split button and the other three verbs are plain. A state
/// the page cannot act on keeps `Create new revision` on its own, which is v1's
/// rule too: committing is local, and only the push half needs access.
///
/// # Two groups, not four peers
///
/// `Publish` / `Get latest` / `Resolve` / `Choose S3 bucket` act on this
/// package's relationship to the remote. `Open folder` and `[⋯]` act on the copy
/// on disk. At one uniform gap they read as four peers, so the essential group
/// and the auxiliary one are separated by a wider space — `space-4` against
/// `space-2` — rather than by a rule, which would be a second border in a header
/// that already has the chip's.
fn header(state: &PackageState) -> AnyView {
    let rendered = render(state, Site::PageHeader);
    let action = rendered.action;
    let publishes = matches!(action, Some(PackageAction::Publish));

    view! {
        <div class="g-stack" style="gap:var(--q-space-2)">
            <BackLink href="#packageheader" label="Packages" />
            <div style="display:flex; align-items:center; gap:var(--q-space-2); \
                        flex-wrap:wrap">
                <h3 style="margin:0; font-size:var(--q-text-title); \
                           min-width:0; overflow:hidden; text-overflow:ellipsis; \
                           white-space:nowrap">
                    {NAMESPACE}
                </h3>
                <StateLabel tone=rendered.tone>{rendered.words}</StateLabel>

                // Essential: what this package can do about the remote.
                <div style="margin-left:auto; display:flex; align-items:center; \
                            gap:var(--q-space-2)">
                    {publishes
                        .then(|| {
                            view! {
                                <SplitButton
                                    label="Publish"
                                    menu_label="Other ways to publish"
                                    actions=vec![
                                        MenuAction::new(
                                            "Create new revision",
                                            Callback::new(|()| ()),
                                        ),
                                    ]
                                    variant=ButtonVariant::Primary
                                    on_click=|_| ()
                                />
                            }
                        })}
                    {(!publishes)
                        .then(|| {
                            view! { <Button on_click=|_| ()>"Create new revision"</Button> }
                        })}
                    {action
                        .filter(|a| !matches!(a, PackageAction::Publish))
                        .map(|action| {
                            view! {
                                <Button variant=ButtonVariant::Primary on_click=|_| ()>
                                    {action.label()}
                                </Button>
                            }
                        })}
                </div>

                // Auxiliary: what you can do with the copy on disk.
                <div style="display:flex; align-items:center; gap:var(--q-space-2); \
                            margin-left:var(--q-space-4)">
                    <Button on_click=|_| ()>"Open folder"</Button>
                    <ActionMenu
                        aria_label="More actions for this package"
                        actions=menu()
                    />
                </div>
            </div>
        </div>
    }
    .into_any()
}

#[component]
pub fn PackageHeaderScene() -> impl IntoView {
    view! {
        <Scene
            title="The package header, state by state"
            note="Thirteen cells for eleven states — `PendingChanges` and `PullConflict` \
                  each appear twice, because the singular is written by hand and a \
                  plural-only fixture never exercises it. \
                  \
                  Read down the action column first. The way to the commit page stays \
                  visible rather than going in `[⋯]` — v1 shows it as a peer of \
                  publishing, sometimes as the primary one, and demoting it would take \
                  away a button people use. Where the command is `Publish` it is the \
                  caret of a `SplitButton`, because those two are one job done two ways. \
                  Where it is `Get latest`, `Resolve` or `Choose S3 bucket` there is no \
                  such relationship, so `Create new revision` stands beside them as its \
                  own button. Four states offer nothing at all and leave it alone on the \
                  row, which is v1's rule too — committing is local, only the push half \
                  needs access. \
                  \
                  Two groups, not four peers. The commands that act on this package's \
                  relationship to the remote sit together; `Open folder` and `[⋯]` act \
                  on the copy on disk and are pushed out by a wider gap. At one uniform \
                  spacing they read as one undifferentiated row of controls. \
                  \
                  Measured, not guessed. Shrinking each row until it wraps: the widest \
                  is `No S3 bucket yet` at 746px, then `Newer revision available` at \
                  742 and `Changed in both places` at 724; the resting state needs 526. \
                  The page has 992 at a 1024 window, so the worst case clears it by 246. \
                  The package name truncates rather than pushing, so a long namespace \
                  does not move these numbers. The list toolbar below lost exactly this \
                  argument — it needs 708 and gets 700 — which is why the header's was \
                  measured rather than argued. \
                  \
                  Then read the tone column. The rubric the words were chosen against: \
                  Success is the resting state and appears once; Neutral is a fact you \
                  may act on; Attention is waiting on you; Danger is something the page \
                  cannot fix by itself. Counted here: one Success, two Neutral, four \
                  Attention and six Danger. Six of thirteen is what this scene is really \
                  for — whether a page about one package can carry that much red without \
                  it stopping meaning anything. \
                  \
                  `conflicts in 3 files` is the one label that is not capitalised. That \
                  is a defect, not a choice, and it is filed. \
                  \
                  Missing: signed out, sign-in expired and unreachable. They are three \
                  rows in the design's table and one `error` string in the backend, so \
                  nothing can draw them apart yet."
        >
            {states()
                .into_iter()
                .map(|(label, state)| {
                    view! { <Cell full=true label=label>{header(&state)}</Cell> }
                })
                .collect_view()}
        </Scene>
    }
}
