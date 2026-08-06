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
use crate::kit::Card;
use crate::kit::Countdown;
use crate::kit::HostRow;
use crate::kit::ToggleRow;

fn in_secs(secs: f64) -> Signal<Option<f64>> {
    let at = js_sys::Date::now() + secs * 1000.0;
    Signal::derive(move || Some(at))
}

#[component]
pub fn StateStripScene() -> impl IntoView {
    let pull = RwSignal::new(true);
    let publish = RwSignal::new(true);
    let role = RwSignal::new("analyst".to_string());
    let out_role = RwSignal::new("analyst".to_string());

    view! {
        <Scene
            title="Scene · state strip"
            note="Both cards at page width, wrapping with no breakpoint. Pull always has a \
                  next tick so it always counts down; publish only counts when changes are \
                  pending, and otherwise says so — a blank would leave the user guessing \
                  between broken, working, and nothing to do. The countdown is live."
        >
            <div style="display:flex; gap:var(--q-space-3); flex-wrap:wrap;">
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
        </Scene>
    }
}
