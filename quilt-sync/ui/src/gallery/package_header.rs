//! The installed-package page's header, in every state the vocabulary can reach.
//!
//! # Why a scene and not a component
//!
//! This settled the header's *shape* — trail, identity, one state label, one
//! primary action, `Open folder`, `[⋯]` — from the kit pieces that exist, so the
//! states could be read against each other before the region was written. The
//! same reasoning the component record applies to `DegradedBand`: the degraded
//! states are not a component, they are the header with different props, and the
//! place to see that is here.
//!
//! **`pages::installed_package_v2::header::PageHeader` is built now, and this is
//! still a parallel drawing of it.** Two differences keep it that way for the
//! moment: the page's header takes a payload where this takes a bare state, and
//! this has an `action_open` the page has no use for until resolve mode exists —
//! the whole-page scene's one cell that needs it. Worth collapsing once resolve
//! mode lands, because a scene that hand-draws a region it could render is a
//! scene that stops being true the first time somebody edits the region.
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
//! # One state is still missing
//!
//! Signed out, sign-in expired and unreachable were three separate rows in the
//! design's header table and one `error` string in the backend — the "three
//! failures the page cannot currently tell apart". Two of them are states now:
//! `NoSession` and `SignInExpired`, told apart because `is_invalid_credentials`
//! and `LoginError::NoSession` are distinguishable at the point the read fails.
//!
//! `Unreachable` is not, and cannot be drawn: `S3ErrorKind` has no transport
//! variant, so a connection failure is indistinguishable from a failed list or
//! get, and nothing could construct the state. It draws as `Unknown` until the
//! engine can tell them apart.

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
use crate::kit::SplitOption;
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
///
/// `NoSession` appears twice for the opposite reason: it DOES name its host, so
/// the two shapes draw different headers, and the hostless one is the widest
/// case the row has to survive in reverse — the shortest label beside the same
/// controls.
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
        (
            "no session for the deployment",
            PackageState::NoSession {
                host: Some("demo.quiltdata.com".to_string()),
            },
        ),
        (
            "a bare bucket on ambient credentials, so no deployment to name",
            PackageState::NoSession { host: None },
        ),
        (
            "a session that existed and was refused",
            PackageState::SignInExpired {
                host: Some("demo.quiltdata.com".to_string()),
            },
        ),
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
/// # One gap, because there is nothing left to group
///
/// An earlier arrangement split the row into an essential group and an auxiliary
/// one with a wider space between them. That earned itself while the row held
/// four controls. It does not now: moving `Create new revision` into the menu
/// leaves at most the state's action, `Open folder` and `[⋯]`, so the wider gap
/// would separate a group of one from a group of two and invite the reader to
/// look for a distinction that is not doing any work. One uniform `space-2`.
/// `action_open`: the state's own action is already on screen somewhere else, so
/// the header does not offer it a second time.
///
/// One primary per screen is the rule this serves. Resolve mode is the case: the
/// header's `Resolve` is what opens the pane, and while the pane is open it
/// would be a second primary button offering what is already being offered —
/// beside the pane's own `Make mine the shared one`, which is the real one. The
/// pane's `BackLink` closes the mode and the header's action comes back with it,
/// so the pair reads as one control in two states rather than as two controls.
fn header(state: &PackageState, publish_choice: RwSignal<usize>, action_open: bool) -> AnyView {
    let rendered = render(state, Site::PageHeader);
    let action = rendered.action;
    let publishes = !action_open && matches!(action, Some(PackageAction::Publish));

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

                <div style="margin-left:auto; display:flex; align-items:center; \
                            gap:var(--q-space-2)">
                    {publishes
                        .then(|| {
                            view! {
                                <SplitButton
                                    options=vec![
                                        SplitOption::new("Publish", Callback::new(|()| ())),
                                        SplitOption::new(
                                            "Create new revision",
                                            Callback::new(|()| ()),
                                        ),
                                    ]
                                    selected=publish_choice
                                    menu_label="Change what this button does"
                                    variant=ButtonVariant::Primary
                                />
                            }
                        })}
                    {action
                        .filter(|_| !action_open)
                        .filter(|a| !matches!(a, PackageAction::Publish))
                        .map(|action| {
                            view! {
                                <Button variant=ButtonVariant::Primary on_click=|_| ()>
                                    {action.label()}
                                </Button>
                            }
                        })}
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

/// The region itself, so the whole-page scene composes this code rather than a
/// copy of it. A mockup that hand-writes a region is a mockup that stops being
/// true the first time somebody edits the region.
#[component]
#[allow(
    clippy::needless_pass_by_value,
    reason = "a component's props are owned; `header` borrows it from there"
)]
pub fn PackageHeaderRegion(
    /// Which state the package is in. The header is state-driven and nothing
    /// else: the tone, the words and the primary action all come from `render`.
    state: PackageState,
    /// Shared with the page's other cells, so the split button's choice is the
    /// page's preference rather than one cell's.
    publish_choice: RwSignal<usize>,
    /// The state's action is already open elsewhere on the page — resolve mode's
    /// pane — so the header drops it rather than drawing a second primary.
    #[prop(optional)]
    action_open: bool,
) -> impl IntoView {
    header(&state, publish_choice, action_open)
}

#[component]
pub fn PackageHeaderScene() -> impl IntoView {
    // One signal across every cell, so picking in any of them moves them all —
    // which is what a preference persisted by the page would do. Per-cell signals
    // would draw a control that cannot exist.
    let publish_choice = RwSignal::new(0_usize);

    view! {
        <Scene
            title="The package header, state by state"
            note="Sixteen cells for thirteen states; `PendingChanges`, `PullConflict` and \
                  `NoSession` appear twice, the first two because the singular is written by \
                  hand and the third because only it names a host. Read down the action \
                  column: every control answers what this package needs, so it is the \
                  state's own action and nothing else, while `Create new revision` sits in \
                  the overflow menu in all sixteen. Measured: the widest row is 609px — \
                  `Signed out of demo.quiltdata.com`, which took the title from `Revision \
                  not published` at 568 — against the 992 the page has at 1024, and every \
                  row is 32px tall, so the page does not jump between states. Then read the \
                  tone column — one Success, two Neutral, four Attention, nine Danger. \
                  Whether one package's page carries that much red is what this scene is \
                  for. Missing: unreachable, which the backend still cannot tell from any \
                  other failed read."
        >
            {states()
                .into_iter()
                .map(|(label, state)| {
                    view! { <Cell full=true label=label>{header(&state, publish_choice, false)}</Cell> }
                })
                .collect_view()}
        </Scene>
    }
}
