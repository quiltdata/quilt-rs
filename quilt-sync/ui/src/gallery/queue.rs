//! The attention queue scene. Its components' stories are in `queue_parts.rs`.
//!
//! This is the region the whole redesign is for, so the scene carries the load: the
//! stories prove each row, and only the scene shows whether nineteen rows of things
//! needing decisions read as a queue or as a pile.

use leptos::prelude::*;

use crate::Scene;
use crate::gallery::queue_parts::{REJECTION, actionable, queue_row};
use crate::kit::AttentionQueue;
use crate::kit::Button;
use crate::kit::CauseRow;
use crate::kit::PackageAction;
use crate::kit::PackageState;
use crate::kit::QueueRow;
use crate::kit::ZeroLine;
use quilt_sync_ui::pages::action_href;

/// The remedy points wherever the page would point it.
fn page_href(action: PackageAction, namespace: &'static str) -> String {
    // The scenes name their packages as literals; the rows address them through
    // the type, exactly as the page does.
    let addressed = quilt_uri::Namespace::try_from(namespace).expect("a scene namespace");
    action_href(action, &addressed)
}

pub(crate) fn row(namespace: &'static str, state: &PackageState) -> AnyView {
    queue_row(namespace, state, None, page_href)
}

/// The same, with the second line only a pause carries.
fn detailed_row(namespace: &'static str, state: &PackageState, detail: Option<String>) -> AnyView {
    queue_row(namespace, state, detail, page_href)
}

/// The region itself, so the whole-page scene composes this code rather than a copy.
#[component]
pub fn QueueRegion() -> impl IntoView {
    let signed_out = RwSignal::new(false);
    let role = RwSignal::new(false);

    // Derived, never written — 11 + 3 + 5 + the paused row. The mock's hand-written
    // (17) is off against its own rows, which is what this closure exists to avoid.
    // Bound once rather than rebuilt on every read of the count: `actionable`
    // allocates.
    let actionable = actionable();
    let total = 11 + 3 + actionable.len() + 1;

    view! {
        <AttentionQueue text=format!("{total} packages need your attention")>
            <li>
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
            </li>
            <li>
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
            </li>
            {actionable
                .iter()
                .take(1)
                .map(|(namespace, state)| view! { <li>{row(namespace, state)}</li> })
                .collect_view()}
            // Precedence row 3, between the conflict above and everything below,
            // and the only row here that goes nowhere: nothing in the app can
            // restart a sync the remote refused.
            <li>
                {detailed_row(
                    "team/imaging-cohort-b",
                    &PackageState::Paused,
                    Some(REJECTION.to_string()),
                )}
            </li>
            {actionable
                .iter()
                .skip(1)
                .map(|(namespace, state)| view! { <li>{row(namespace, state)}</li> })
                .collect_view()}
        </AttentionQueue>
    }
}

#[component]
pub fn QueueScene() -> impl IntoView {
    view! {
        <Scene
            title="Scene · needs your attention"
            note="Shared causes first, then per-package rows in precedence order. Open both \
                  expanders and watch what nineteen items costs in vertical space, because \
                  that is what this design spends to buy the region. Read down the verbs: \
                  five, one per row, each true of the row it sits on. The paused row costs the most height and goes nowhere — it \
                  carries the engine's own rejection."
        >
            <QueueRegion />
        </Scene>
        <Scene
            title="Scene · nothing needs you"
            note="The same region on a working day, which is the common case with autosync \
                  on. The status line alone, with no rows and no count — counting to zero \
                  would be noise — and the package list starts immediately below rather \
                  than a screen down."
        >
            <ZeroLine text="All packages are up to date" />
        </Scene>
    }
}
