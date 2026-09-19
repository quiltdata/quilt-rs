//! The whole installed-package page, composed from the three regions.
//!
//! Not a fourth copy of anything: `PackageHeaderRegion`, `FilePaneRegion` and
//! `ContextPaneRegion` are the same code their own scenes draw, so this page and
//! those scenes cannot disagree about what a region is.
//!
//! # What this answers that the scenes cannot
//!
//! **How the page spends its height.** Every region scene was framed at a width
//! and given as much height as it wanted. The design's central bet is that two
//! panes buy enough horizontal room to be worth a header and two control rows
//! above them, and §9 records that bet as *unsolved*: measured at the floor when
//! the layout was first drawn, four rows of list survived, which is what v1
//! manages. Nothing has re-measured it since the regions were built.
//!
//! The first two cells are that measurement, at `tauri.conf.json`'s own
//! `minWidth`/`minHeight` of 1024×560, with and without the footer — because the
//! footer is the difference between the resting page and the page you are
//! actually picking files on.
//!
//! # What it measured, at 1024x560
//!
//! **The vertical bet is won, and by more than the doc expected.**
//!
//! | | design §1, first drawn | measured here |
//! |---|---:|---:|
//! | list, nothing ticked | 193px · 4 rows | **269px · 8 rows** |
//! | list, three ticked | — | **220px · 6 rows** |
//! | at the 900px default | — | 609px · 19 rows |
//!
//! v1 manages about four at the same window. The difference is not layout
//! cleverness: **the header came in at 60px against the 108 §1 budgeted**,
//! because the built header is two lines — a `BackLink` over one 32px row — and
//! the bucket it used to carry moved into the context pane. §9's own number,
//! 264px and five rows, turns out to describe the resting state rather than the
//! selecting one, and to be about right for it.
//!
//! What is still spent for nothing: `PageLayout`'s `main` ends with
//! `padding-block-end: space-8`, 32px of empty page under the list at a window
//! that has none to spare. That is a row.
//!
//! # The list is the only region that takes what is left
//!
//! Everything else on this page has a height of its own: the appbar is 48, the
//! header is its three lines, the search row and the toolbar are what they are.
//! The list is as long as the package, so it gets the remainder and scrolls
//! inside it — `Card`'s `fill`, and `FilePaneRegion`'s own `fill`, exist for
//! exactly this and for nothing else.
//!
//! # The narrow arrangement is the shell's own decision
//!
//! §1 stacks the panes below ~800px, context above files, because the region
//! with no bound on its length goes last. Drawn here as a **container query** on
//! the shell rather than a viewport media query: what decides the arrangement is
//! the room the shell has, not the window's width, and in a gallery cell those
//! are different numbers. The real page can keep the same rule.
//!
//! One thing that costs: `column-reverse` swaps what you see and leaves the DOM
//! alone, so at narrow the focus order reaches the file pane before the context
//! pane above it. A page written in Leptos can reorder the two for real behind a
//! media query, which is the fix; a stylesheet on its own cannot.

use leptos::prelude::*;

use crate::Cell;
use crate::Scene;
use crate::gallery::context_pane::ContextPaneRegion;
use crate::gallery::file_pane::FilePaneRegion;
use crate::gallery::package_header::PackageHeaderRegion;
use crate::kit::Button;
use crate::kit::PackageState;
use crate::kit::PageLayout;
use crate::kit::icons;

/// The appbar is untouched by this page — §1, no omnibar and no breadcrumb.
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

/// One page, at one window size.
fn page(
    width: u32,
    height: u32,
    state: PackageState,
    name: &'static str,
    ticked: usize,
    resolving: bool,
) -> AnyView {
    let publish_choice = RwSignal::new(0_usize);
    let scope = RwSignal::new("pick".to_string());

    view! {
        <div
            class="g-window"
            style=format!("width:{width}px; --q-frame-height:{height}px; max-width:100%")
        >
            <PageLayout heading="QuiltSync" actions=appbar_actions()>
                <div class="g-ip-page">
                    <PackageHeaderRegion state=state publish_choice=publish_choice />
                    <div class="g-ip-shell">
                        <FilePaneRegion name=name ticked=ticked marked=resolving />
                        <ContextPaneRegion resolving=resolving scope=scope pending=2 />
                    </div>
                </div>
            </PageLayout>
        </div>
    }
    .into_any()
}

const NOTE: &str = "The page at its own floor, 1024×560, which is where the vertical bet \
    is settled. It is won: 269px of list resting and 220 with the footer up — eight rows \
    and six, against the four §1 measured when the layout was first drawn and the four v1 \
    manages. The header is what paid for it, at 60px against the 108 budgeted. The third \
    cell is the 900px default for comparison, the fourth is 760px wide where the shell \
    stacks itself, and the last is resolve mode, where the marked rows and the sentence \
    naming them are finally on screen together.";

#[component]
pub fn InstalledPackageScene() -> impl IntoView {
    view! {
        <Scene title="The installed package page" note=NOTE>
            <Cell full=true label="1024×560 — the floor, nothing ticked">
                {page(1024, 560, PackageState::Behind, "page-floor", 0, false)}
            </Cell>
            <Cell full=true label="1024×560 — three ticked, and the footer has taken its 49px">
                {page(1024, 560, PackageState::Behind, "page-selecting", 3, false)}
            </Cell>
            <Cell full=true label="1024×900 — the default window, for what the floor costs">
                {page(1024, 900, PackageState::Behind, "page-default", 0, false)}
            </Cell>
            <Cell full=true label="760×560 — the shell stacks itself, context above files">
                {page(760, 560, PackageState::Behind, "page-narrow", 0, false)}
            </Cell>
            <Cell full=true label="1024×560 — resolve mode, the pane swapped and the rows marked">
                {page(1024, 560, PackageState::Diverged, "page-resolve", 0, true)}
            </Cell>
        </Scene>
    }
}
