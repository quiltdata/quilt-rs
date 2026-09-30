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
//! where `ZeroLine` collapses the whole region to a line. A third pairs the calm
//! page still checking with the page it settles into.
//!
//! [`LoadingScene`] is the pages before they have anything to show, and those are
//! the app's own components rather than compositions: `MainPageSkeleton` and
//! `PackagePageSkeleton` are what `main.rs` draws while a route decides between v1
//! and v2, so a scene there is the frame the reader sees.

use leptos::context::Provider;
use leptos::prelude::*;

use crate::Scene;
use crate::gallery::packages::PackagesRegion;
use crate::gallery::queue::QueueRegion;
use crate::gallery::state_strip::StateStripRegion;
use crate::kit::Activities;
use crate::kit::Activity;
use crate::kit::ActivityKind;
use crate::kit::Button;
use crate::kit::PageLayout;
use crate::kit::ZeroLine;
use crate::kit::ZeroLineSkeleton;
use crate::kit::icons;
use crate::pages;

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

/// The bar a loading page draws: Refresh spinning, because the page's first read
/// is out. The app's own is `components::appbar::appbar_actions` over the same
/// `refresh_button`; its Settings navigates, and this gallery has no router.
fn loading_actions() -> AnyView {
    view! {
        {quilt_sync_ui::components::appbar::refresh_button(|| (), Signal::stored(true))}
        <Button leading_visual=icons::gear() on_click=|_| ()>
            "Settings"
        </Button>
    }
    .into_any()
}

/// One autopull activity, for a bar of its own.
fn activities(label: &str) -> Activities {
    let activities = Activities::new();
    activities.set(vec![Activity {
        kind: ActivityKind::Autopull,
        label: label.to_owned(),
    }]);
    activities
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
                    // Bare, with no card, as `QueueRegion` draws it.
                    <ZeroLine text="Everything is Latest — 43 packages" />
                    <PackagesRegion view_name="normal-day-view" />
                </PageLayout>
            </div>
        </Scene>
        <CheckingScene />
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
            title="Scene · the appbar while autopull moves files"
            note="The activity line, centered on the bar between the logo and the controls, \
                  in the on-brand ink at reduced strength on a faint amber tint with the kit \
                  radius. It fades in after about 400ms, so reload to watch it arrive. Each \
                  bar has its own Activities; without one, as in the scene above, the bar \
                  draws no line at all. The second label is long enough to wrap: it grows \
                  the bar, and is never cut."
        >
            <div class="g-window g-window--bar">
                <Provider value=activities("Getting latest for team/pkg\u{2026}")>
                    <PageLayout heading="QuiltSync" actions=appbar_actions()>""</PageLayout>
                </Provider>
            </div>
            <div class="g-window g-window--bar">
                <Provider value=activities(
                    "Getting latest for a-team-with-a-long-name/an-analysis-package-whose-name-goes-on\u{2026}",
                )>
                    <PageLayout heading="QuiltSync" actions=appbar_actions()>""</PageLayout>
                </Provider>
            </div>
        </Scene>
    }
}

/// Both pages before they have anything to show — see the module doc.
#[component]
pub fn LoadingScene() -> impl IntoView {
    view! {
        <Scene
            title="Scene · the main page while it loads"
            note="What / draws before any read has answered, and what it draws even \
                  earlier, while it reads which main page the reader has switched on: the \
                  root marker predicts this one, so the frame is this page's first paint \
                  rather than a spinner of its own. The appbar with Refresh spinning, the \
                  strip's two cards on their toggle and host skeletons, the zero line held \
                  open, the live toolbar, and three package rows. MainPageSkeleton is drawn here and by main.rs's \
                  frame, and its regions are the page's own loading boundary, so neither \
                  handover — frame to page, page to rows — moves anything. A v1 reader \
                  gets v1's own spinner instead, whose stylesheet this gallery does not \
                  load."
        >
            <div class="g-window">
                <pages::MainPageSkeleton actions=loading_actions() />
            </div>
        </Scene>
        <Scene
            title="Scene · the package page while it loads"
            note="The same for /installed-package: the appbar, the banner's empty row, the \
                  header's two lines and both panes, select-all's spot in the file \
                  toolbar included, before the page's one read answers. \
                  PackagePageSkeleton, shared with main.rs's frame; its body is the page's \
                  own first paint. The banner's row is there because the page always \
                  fills that slot, as the package page's own scenes now draw it too."
        >
            <div class="g-window">
                <pages::PackagePageSkeleton actions=loading_actions() />
            </div>
        </Scene>
    }
}

/// The calm page still checking, beside the page it settles into.
#[component]
fn CheckingScene() -> impl IntoView {
    view! {
        <Scene
            title="Scene · the whole page, still checking"
            note="The calm day before its checks answer, beside the page it settles into: \
                  rows dimmed, the zero line's placeholder in the queue's place. The first \
                  package row must start at the same height in both windows."
        >
            <div class="g-window-pair">
                <div class="g-window">
                    <PageLayout heading="QuiltSync" actions=appbar_actions()>
                        <StateStripRegion />
                        <ZeroLineSkeleton />
                        <PackagesRegion view_name="checking-view" provisional=true />
                    </PageLayout>
                </div>
                <div class="g-window">
                    <PageLayout heading="QuiltSync" actions=appbar_actions()>
                        <StateStripRegion />
                        <ZeroLine text="Everything is Latest — 43 packages" />
                        <PackagesRegion view_name="checked-view" />
                    </PageLayout>
                </div>
            </div>
        </Scene>
    }
}
