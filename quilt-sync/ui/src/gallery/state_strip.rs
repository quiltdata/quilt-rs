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
use crate::kit::Card;
use crate::kit::CauseRow;
use crate::kit::Countdown;
use crate::kit::HostRow;
use crate::kit::QueueRow;
use crate::kit::StateLabel;
use crate::kit::StateTone;
use crate::kit::ToggleRow;

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

/// The paused case, and the two regions together — because the pairing *is* the
/// design, and neither half is right alone.
#[component]
pub fn PausedScene() -> impl IntoView {
    let pull = RwSignal::new(true);
    let publish = RwSignal::new(true);
    let expanded = RwSignal::new(false);

    view! {
        <Scene
            title="Scene · autosync paused"
            note="The 2026-07-11 report in one screen: autosync stopped on a transient \
                  error, never re-armed, and said nothing — so every package read \
                  un-published while the switch read on, and the only recovery was \
                  restarting the app. \
                  \
                  The fix is split across two regions on purpose, and the split follows a \
                  rule the page already has. The CARD REPORTS: it says the setting is on \
                  and not operating, and carries no control, because the state strip's job \
                  is to say what is running. The QUEUE ACTS: a paused autosync affecting 13 \
                  packages is a shared cause with a count and an expander, which is exactly \
                  what a CauseRow is — and the queue is where the user already looks for \
                  things that need them. \
                  \
                  One control, one scope. Putting [Resume] in both places would be the \
                  duplication the vocabulary spec bans: a link may appear at two \
                  granularities because it asserts nothing about what it governs, but a \
                  control asserts \"the fix is here, at this scope\", so the same control \
                  twice makes one of them a lie."
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
                    // three ways to clear the pause today, so disabling it would take away
                    // the user's only lever.
                    <ToggleRow
                        label="Publish your changes"
                        sublabel="5 min after your last edit"
                        checked=publish
                        trailing=view! {
                            <StateLabel tone=StateTone::Attention>"Paused"</StateLabel>
                        }
                            .into_any()
                    />
                </Card>
            </div>
            <Card title="Needs your attention" count=13>
                <div>
                    <CauseRow
                        text="Publishing paused after a network error"
                        count=13
                        expanded=expanded
                        trailing=view! {
                            <Button on_click=|_| ()>
                                "Resume"
                            </Button>
                        }
                            .into_any()
                    />
                    <Show when=move || expanded.get()>
                        {["user/package-a", "user/package-b", "org/dataset-c"]
                            .into_iter()
                            .map(|namespace| view! { <QueueRow namespace=namespace sub=true /> })
                            .collect_view()}
                        <QueueRow namespace="…and 10 more" sub=true />
                    </Show>
                </div>
            </Card>
        </Scene>
    }
}
