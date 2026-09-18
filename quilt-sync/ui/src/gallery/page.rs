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
use crate::kit::Button;
use crate::kit::Card;
use crate::kit::PageLayout;
use crate::kit::Spinner;
use crate::kit::SpinnerVariant;
use crate::kit::ZeroLine;
use crate::kit::icons;

fn appbar_actions() -> AnyView {
    view! {
        <Button leading_visual=icons::sync() on_click=|_| ()>
            "Refresh"
        </Button>
        <Button leading_visual=icons::gear() on_click=|_| ()>
            "Settings"
        </Button>
    }
    .into_any()
}

#[component]
pub fn PageScene() -> impl IntoView {
    view! {
        <Scene
            title="Scene · the whole page, a busy day"
            note="The worst case: nineteen things needing a decision, and both expanders \
                  live. The frame is 560px, the window's minimum, so the fold is the real \
                  one — count how many package rows survive above it, then expand a cause \
                  and count again. This is the scene that tests the central bet, that a \
                  queue above the list earns the height it takes."
        >
            <div class="g-window">
                <PageLayout heading="QuiltSync" actions=appbar_actions()>
                    <StateStripRegion />
                    <QueueRegion />
                    <PackagesRegion view_name="busy-day-view" />
                </PageLayout>
            </div>
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
            <div class="g-window">
                <PageLayout heading="QuiltSync" actions=appbar_actions()>
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
                    <PackagesRegion view_name="normal-day-view" />
                </PageLayout>
            </div>
        </Scene>
        <Scene
            title="Scene · the appbar alone"
            note="The two chrome buttons on the brand ground, with nothing under them to \
                  borrow attention from. Buttons with an icon and a label, frame taken off \
                  by the bar (see PageLayout), as v1's link buttons: hover and focus them \
                  here — the ring is the bar's own ink. The glyphs are drawn twice, here and \
                  in main_page.rs; a shared kit/icons.rs is qhq-8mgw.77's."
        >
            <div class="g-window g-window--bar">
                <PageLayout heading="QuiltSync" actions=appbar_actions()>""</PageLayout>
            </div>
        </Scene>
        <Scene
            title="Scene · the frame while / decides"
            note="What the window shows between launch and knowing which main page to draw: \
                  the page ground and a region spinner, no appbar. Drawing one page's chrome \
                  and then swapping it for the other's is the flicker this frame exists to \
                  avoid. The markup is main.rs's, repeated here because a bin cannot lend it."
        >
            <div class="g-window">
                <div data-home-frame>
                    <Spinner variant=SpinnerVariant::Region aria_label="Loading QuiltSync" />
                </div>
            </div>
        </Scene>
    }
}
