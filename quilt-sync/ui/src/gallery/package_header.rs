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
///
/// `Create new revision` is here in **every** state, and also behind the caret
/// of the `Publish` split button in the states that publish. The duplication is
/// deliberate. The menu is its stable home — one place to learn, available even
/// when the package has nothing to publish and so nothing to hang a caret on.
/// The caret is proximity: at the moment somebody is about to publish, the other
/// way to do it should be next to their cursor rather than a menu away.
fn menu() -> Vec<MenuAction> {
    vec![
        MenuAction {
            label: "Create new revision".to_string(),
            tone: ActionTone::Default,
            disabled: None,
            on_select: Callback::new(|()| ()),
            separated: false,
        },
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
/// # The row is state-driven; the menu is not
///
/// Every control on the row answers *what does this package need* — so it is the
/// state's own action and nothing else. `Create new revision` answers *what may
/// I choose to do*, which does not vary with state, so it lives in `[⋯]`.
///
/// Three arrangements were rejected by looking at them. `[⋯]` alone buried a
/// button v1 shows as a peer of publishing. A literal `or` between two buttons
/// read correctly against `Publish` and produced `Create new revision or Get
/// latest` against everything else, claiming a relationship that is not there.
/// Keeping it as a standalone button left it the one thing on the row that was
/// not state-driven — conspicuous in `Latest`, where it stood alone as the only
/// control in the resting state, which is the state people see most.
///
/// So: the states that publish get a [`SplitButton`](crate::kit::SplitButton)
/// whose caret holds `Create new revision`, the states with another verb get
/// that verb plainly, and the states with nothing to do get an empty slot.
///
/// The command is in the menu in every state **and** behind the caret in the
/// publishing ones. Duplication on purpose: the menu is the stable home, the
/// caret is proximity at the moment it is wanted. See [`menu`].
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
                  Read down the action column first. Every control on the row answers \
                  `what does this package need`, so it is the state's own action and \
                  nothing else: a `Publish` split button where there is something to \
                  ship, a plain button for `Get latest`, `Resolve` and `Choose S3 \
                  bucket`, and an empty slot in the four states with nothing to do. \
                  \
                  `Create new revision` answers a different question — `what may I \
                  choose to do` — which does not vary with state, so it lives in `[⋯]`, \
                  present in all thirteen. It is also behind the split button's caret in \
                  the publishing states. That duplication is deliberate: the menu is the \
                  stable home, one place to learn and available even when there is no \
                  `Publish` to hang a caret on; the caret is proximity, at the moment \
                  somebody is about to publish and might want the other way to do it. \
                  \
                  Two groups, not four peers. The state's action sits apart from `Open \
                  folder` and `[⋯]`, which act on the copy on disk rather than on this \
                  package's relationship to the remote. At one uniform spacing they read \
                  as one undifferentiated row of controls. \
                  \
                  Measured, not guessed. Shrinking each row until it wraps: the widest \
                  is `Revision not published` at 584px, then `No S3 bucket yet` at 584 \
                  and `Newer revision available` at 580; the resting state needs 370. \
                  The page has 992 at a 1024 window, so the worst case clears it by 408. \
                  Taking the standalone button off the row bought back 162px against the \
                  arrangement before it. The package name truncates rather than pushing, \
                  so a long namespace does not move these numbers. The list toolbar \
                  below lost exactly this \
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
