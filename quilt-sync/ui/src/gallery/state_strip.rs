//! The state strip — the two cards at the top of the new main page, composed as
//! the page composes them.
//!
//! This is the first time the design is visible as a *region* rather than as a
//! list of controls, so it is where region-level mistakes show up: whether the
//! two cards balance, whether the sub-labels crowd, and how tall the strip is —
//! which matters because every pixel here is a pixel the attention queue and the
//! package list do not get.

use leptos::prelude::*;

use crate::Scene;
use crate::kit::Button;
use crate::kit::ButtonVariant;
use crate::kit::Card;
use crate::kit::Countdown;
use crate::kit::HostRow;
use crate::kit::QueueRow;
use crate::kit::StateLabel;
use crate::kit::StateTone;
use crate::kit::ToggleRow;
use crate::kit::ZeroLine;

fn in_secs(secs: f64) -> Signal<Option<f64>> {
    let at = js_sys::Date::now() + secs * 1000.0;
    Signal::derive(move || Some(at))
}

#[component]
pub fn StateStripScene() -> impl IntoView {
    view! {
        <Scene
            title="Scene · state strip"
            note="Both cards at page width, wrapping with no breakpoint. Pull always has a \
                  next tick so it always counts down; publish only counts when changes are \
                  pending, and otherwise says so — a blank would leave the user guessing \
                  between broken, working, and nothing to do. The countdown is live."
        >
            <StateStripRegion />
        </Scene>
    }
}

/// The region itself, so the whole-page scene composes this code rather than a copy of
/// it. Two mockups of one region drift the first time either is edited.
#[component]
pub fn StateStripRegion() -> impl IntoView {
    let pull = RwSignal::new(true);
    let publish = RwSignal::new(true);
    let role = RwSignal::new("analyst".to_string());
    let out_role = RwSignal::new("analyst".to_string());

    view! {
        // No breakpoint and no media query: `flex-wrap` alone drops the second card
        // under the first when the window cannot hold both.
        <div class="g-strip">
                <Card title="Autosync">
                    <ToggleRow
                        label="Get new revisions"
                        sublabel="Every 30s, when nothing is changed here"
                        checked=pull
                        trailing=view! {
                            <Countdown
                                deadline=in_secs(23.0)
                                interval=30_000.0
                                aria_label="Checks for new revisions every 30 seconds"
                                repeat=true
                            />
                        }
                            .into_any()
                    />
                    <ToggleRow
                        label="Publish your changes"
                        sublabel="5 min after your last edit"
                        checked=publish
                        trailing=view! { "nothing to publish" }.into_any()
                    />
                </Card>

                <Card title="Accounts">
                    <HostRow
                        host="open.quiltdata.com"
                        role=role
                        roles=vec![
                            "analyst".to_string(),
                            "bench-scientist".to_string(),
                            "admin".to_string(),
                        ]
                        on_sign_in=|_| ()
                    />
                    <HostRow
                        host="custom.registry.io"
                        role=out_role
                        signed_out=true
                        on_sign_in=|_| ()
                    />
                </Card>
            </div>
    }
}

/// The paused case. Two scenes, because the interesting thing is the difference
/// between them — and neither one has a `[Resume]` button, which is the correction.
///
/// # Every pause reason is the user's to fix
///
/// All six `PausedReason` variants require an action, and every one already has a
/// queue row that carries it:
///
/// | Reason | Queue row | Action |
/// |---|---|---|
/// | `PendingChanges` | `2 files changed` | `[Publish]` |
/// | `PendingCommit` | `Revision not published` | `[Publish]` |
/// | `Diverged` | `Changed in both places` | `[Resolve]` |
/// | `PullConflict(files)` | `conflicts in N files` | `[Publish]` |
/// | `RoleDenied { role }` | `No access as analyst` | points at Accounts |
/// | `Other(msg)` | the message | whatever it names |
///
/// `RoleDenied` says outright that *"retrying cannot help — the role has to change
/// first"*, and `Other` is documented as non-transient. So there is no reason for which
/// "resume" is the fix, and a `[Resume]` button would offer to retry something that
/// will pause again on the next tick.
///
/// It follows that **a pause is never its own queue row.** It is a consequence of a
/// state the queue already lists, and listing consequences beside causes would count
/// the same packages twice — including in the region's own header count.
#[component]
pub fn PausedScene() -> impl IntoView {
    view! { <PausedWithReason /> <PausedStale /> }
}

/// The paused label used in both scenes, so the two cannot drift apart on the one
/// thing they are being compared on.
fn paused() -> AnyView {
    view! { <StateLabel tone=StateTone::Attention>"Paused"</StateLabel> }.into_any()
}

#[component]
fn PausedWithReason() -> impl IntoView {
    let pull = RwSignal::new(true);
    let publish = RwSignal::new(true);

    view! {
        <Scene
            title="Scene · paused, with a reason"
            note="The ordinary case, and there is no [Resume] anywhere in it. The card \
                  REPORTS that publishing is on and not operating; the queue already \
                  EXPLAINS why and carries the fix. Publish those two files and the pause \
                  clears as a side effect, because committing is one of the seven sites \
                  that calls clear_paused. \
                  \
                  So the paused label is a pointer, not a problem of its own: it tells you \
                  the switch is not lying, and the row below tells you what to do. Adding a \
                  paused row to the queue would count the same packages twice — once as the \
                  cause and once as its consequence — including in the header count."
        >
            <div class="g-strip">
                <Card title="Autosync">
                    <ToggleRow
                        label="Get new revisions"
                        sublabel="Every 30s, when nothing is changed here"
                        checked=pull
                        trailing=view! {
                            <Countdown
                                deadline=in_secs(23.0)
                                interval=30_000.0
                                aria_label="Checks for new revisions every 30 seconds"
                                repeat=true
                            />
                        }
                            .into_any()
                    />
                    // Checked and enabled, and both matter. The setting is on — what
                    // stopped is the machinery — and flipping it off and on is one of only
                    // three ways to clear the pause today, so disabling it would remove the
                    // user's only lever.
                    <ToggleRow
                        label="Publish your changes"
                        sublabel="5 min after your last edit"
                        checked=publish
                        trailing=paused()
                    />
                </Card>
            </div>
            <Card title="Needs your attention" count=1>
                <div>
                    <QueueRow
                        namespace="user/package-b"
                        state="2 files changed"
                        tone=StateTone::Neutral
                        action=view! {
                            <Button variant=ButtonVariant::Primary on_click=|_| ()>
                                "Publish"
                            </Button>
                        }
                            .into_any()
                    />
                </div>
            </Card>
        </Scene>
    }
}

#[component]
fn PausedStale() -> impl IntoView {
    let stale_pull = RwSignal::new(true);
    let stale_publish = RwSignal::new(true);

    view! {
        <Scene
            title="Scene · paused with nothing to fix — the 2026-07-11 bug"
            note="THIS IS THE FAILURE STATE, rendered rather than described. The card says \
                  paused and the queue says everything is Latest, which is a flat \
                  contradiction — and it is exactly what the 2026-07-11 diagnostic bundle \
                  showed: data.json had all 13 packages fully synced, commit == null and \
                  base_hash == latest_hash, so the reason had gone away while the pause \
                  persisted in memory. The user's only recovery was restarting the app. \
                  \
                  It cannot self-heal because autopull's tick skips any namespace already \
                  in the paused set, so the tick that would prove the condition cleared is \
                  the one that never runs. \
                  \
                  The fix is NOT a button. This state should not be reachable, and \
                  qhq-usw0 is the bug. The escape hatch is deliberately left out of this \
                  scene, because drawing one would make a backend defect look like a \
                  feature — and because the UI cannot tell this case from the one above \
                  unless the queue and the paused set are derived from ONE resolved state. \
                  Today they are assembled from two sources, which is what lets them \
                  disagree."
        >
            <div class="g-strip">
                <Card title="Autosync">
                    <ToggleRow
                        label="Get new revisions"
                        sublabel="Every 30s, when nothing is changed here"
                        checked=stale_pull
                        trailing=paused()
                    />
                    <ToggleRow
                        label="Publish your changes"
                        sublabel="5 min after your last edit"
                        checked=stale_publish
                        trailing=paused()
                    />
                </Card>
            </div>
            <Card title="Needs your attention">
                <ZeroLine text="Everything is Latest — 13 packages" />
            </Card>
        </Scene>
    }
}
