//! The whole main page, composed from the three region components.
//!
//! Not a fourth copy of anything. `StateStripRegion`, `QueueRegion` and
//! `PackagesRegion` are the same components their own scenes render, so this page and
//! those scenes cannot disagree — which is the only way a whole-page mockup stays
//! true once someone edits a region.
//!
//! # What this is for
//!
//! Every earlier scene answered a question about one region. This one answers the
//! question the regions cannot: **how the page spends its height.** The design's
//! central bet is that a queue above the list is worth the vertical space it takes,
//! and until the three regions sit in one column at one width, that bet is untested.
//!
//! Two versions, because they are the two days a user has. The busy page is the
//! worst case — 19 things needing decisions — and the calm page is the common one,
//! where `ZeroLine` collapses the whole region to a line.

use leptos::prelude::*;

use crate::Scene;
use crate::gallery::packages::PackagesRegion;
use crate::gallery::queue::QueueRegion;
use crate::gallery::state_strip::StateStripRegion;
use crate::kit::Card;
use crate::kit::IconButton;
use crate::kit::Layout;
use crate::kit::ZeroLine;

fn refresh_icon() -> AnyView {
    view! {
        <svg viewBox="0 0 16 16" aria-hidden="true" fill="none" stroke="currentColor"
            stroke-width="1.4" stroke-linecap="round">
            <path d="M13.5 8a5.5 5.5 0 1 1-1.9-4.15" />
            <path d="M13.6 1.9v2.4h-2.4" />
        </svg>
    }
    .into_any()
}

fn gear_icon() -> AnyView {
    view! {
        <svg viewBox="0 0 16 16" aria-hidden="true" fill="none" stroke="currentColor"
            stroke-width="1.4">
            <circle cx="8" cy="8" r="2.1" />
            <path d="M8 1.6v1.7M8 12.7v1.7M2.5 8H4.2M11.8 8h1.7M4.1 4.1l1.2 1.2M10.7 10.7l1.2 1.2M11.9 4.1l-1.2 1.2M5.3 10.7l-1.2 1.2" />
        </svg>
    }
    .into_any()
}

fn appbar_actions() -> AnyView {
    view! {
        <IconButton icon=refresh_icon() aria_label="Refresh" on_click=|_| () />
        <IconButton icon=gear_icon() aria_label="Settings" on_click=|_| () />
    }
    .into_any()
}

#[component]
pub fn PageScene() -> impl IntoView {
    view! {
        <Scene
            title="Scene · the whole page, a busy day"
            note="The worst case: nineteen things needing a decision, and both expanders \
                  still work. This is the scene that tests the design's central bet — that \
                  a queue above the list earns the height it takes. Scroll it and count how \
                  many package rows survive above the fold, then expand a cause and count \
                  again. \
                  \
                  Read it top to bottom as a user would: what is running (Autosync), who \
                  you are (Accounts), what wants you (the queue), then everything else \
                  (the list). Nothing below the queue needs reading on a bad day, and \
                  nothing in the queue exists on a good one."
        >
            <Layout actions=appbar_actions()>
                <StateStripRegion />
                <QueueRegion />
                <PackagesRegion />
            </Layout>
        </Scene>
        <Scene
            title="Scene · the whole page, a normal day"
            note="The same page with autosync working, which is what most users see most \
                  days. The queue is one line and the package list starts near the top — \
                  compare the first visible row here against the busy page above, because \
                  that difference is the whole argument for ZeroLine not being a \
                  full-height empty state. \
                  \
                  The state strip is unchanged, and that is deliberate: it reports what is \
                  running, which is as true on a calm day as on a bad one."
        >
            <Layout actions=appbar_actions()>
                <StateStripRegion />
                // The queue region collapsed. Composed here rather than hidden inside
                // `QueueRegion` behind a flag — "is anything wrong" is the caller's
                // question, and a region that answered it for itself would need the
                // data this gallery does not have.
                //
                // No count: counting to zero is noise, and `Card`'s count is optional
                // for exactly this row of the design.
                <Card title="Needs your attention">
                    <ZeroLine text="Everything is Latest — 43 packages" />
                </Card>
                <PackagesRegion />
            </Layout>
        </Scene>
    }
}
