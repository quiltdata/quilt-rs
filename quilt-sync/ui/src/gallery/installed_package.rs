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
//! # One primary per screen
//!
//! The header's action is the way *into* a mode, so while the mode is on screen
//! it hides. Resolve is the case: entering leaves `Share mine` as
//! the page's one primary, and the pane's `BackLink` closes the mode and brings
//! the header's `Resolve` back with it. The two read as one control in two
//! states rather than as two controls, and the header keeps `Open folder`, the
//! `[⋯]` and the Danger state label that says what is wrong. Settled 2026-09-19;
//! it is also what settles what the resolve pane's exit is.
//!
//! **Both ends are drawn, two cells apart rather than one click apart.** The
//! pane is the app's own `ResolvePane`, and its exit is a `BackLink` because
//! leaving the mode is a navigation — the page drops `?resolve=1` and the
//! router redraws it — so in a gallery, which has no router, it is an anchor
//! that does nothing — pointed at this cell's own
//! window, because an anchor aimed anywhere further away scrolls, and a link
//! that says it does nothing must not move the page. The same way the revisions
//! surface's catalog links have no browser to open. The last cell is the mode
//! open and its header without a primary; the other four are the same page with
//! `Resolve` back on the header.
//!
//! The same rule has a second instance this page does **not** resolve: with
//! files ticked, the header's `Get latest` and the footer's `[Download N]` are
//! both primary. They are different verbs on different objects — the package and
//! the rows you picked — and each is the primary of its own region, so they are
//! left alone and named here rather than quietly changed.
//!
//! # The narrow arrangement is the shell's own decision
//!
//! §1 stacks the panes below ~800px, context above files, because the region
//! with no bound on its length goes last. Drawn here as a **container query** on
//! the shell rather than a viewport media query: what decides the arrangement is
//! the room the shell has, not the window's width, and in a gallery cell those
//! are different numbers. The real page can keep the same rule.
//!
//! Stacked, the pane takes the whole column and its two blocks sit **side by
//! side** rather than one under the other, with `PaneSection`'s divider turned
//! from a `border-top` into a `border-left`. It is the same trade the page makes
//! one level up — horizontal room is what this arrangement has and vertical room
//! is what it lacks — and it is worth 130px: the pane measures 170px against the
//! 300 it took stacked in a column, and every pixel of that is a row of the list
//! under it.
//!
//! The context pane comes first in the DOM, so reading and focus order is
//! context, then files, at every width. Wide, `order` draws the files on the
//! leading side; stacked, the order resets and what you see is DOM order.

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
            id=name
            class="g-window"
            style=format!("width:{width}px; --q-frame-height:{height}px; max-width:100%")
        >
            <PageLayout heading="QuiltSync" actions=appbar_actions()>
                <div class="g-ip-page">
                    <PackageHeaderRegion
                        state=state
                        publish_choice=publish_choice
                        action_open=resolving
                    />
                    <div class="g-ip-shell">
                        <ContextPaneRegion
                            resolving=resolving
                            scope=scope
                            pending=2
                            exit=format!("#{name}")
                        />
                        <FilePaneRegion name=name ticked=ticked marked=resolving />
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
    stacks itself, and the last is resolve mode, the app's own resolve pane beside the \
    marked rows, so the rows and the sentence naming them are finally on screen together.";

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
            <Cell full=true label="1024×560 — resolve mode: the pane swapped, the rows marked, no primary in the header">
                {page(1024, 560, PackageState::Diverged, "page-resolve", 0, true)}
            </Cell>
        </Scene>
    }
}
