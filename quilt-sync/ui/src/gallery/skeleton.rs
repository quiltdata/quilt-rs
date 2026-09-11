//! `SkeletonBox` stories.
//!
//! The composed cells are the ones that matter. A bar on its own proves nothing; a
//! skeleton row sitting directly above the real row it stands in for proves the only
//! thing worth proving, which is that they are the same height.

use leptos::prelude::*;

use crate::Cell;
use crate::Story;
use crate::kit::Button;
use crate::kit::ButtonVariant;
use crate::kit::Card;
use crate::kit::GroupHeading;
use crate::kit::PackageRow;
use crate::kit::PackageRowSkeleton;
use crate::kit::QueueRow;
use crate::kit::QueueRowSkeleton;
use crate::kit::SkeletonBox;
use crate::kit::StateTone;

const HOUR: f64 = 3_600_000.0;

fn ago(ms: f64) -> f64 {
    js_sys::Date::now() - ms
}

#[component]
pub fn SkeletonStories() -> impl IntoView {
    view! { <Bars /> <Rows /> }
}

#[component]
fn Bars() -> impl IntoView {
    view! {
        <Story
            title="SkeletonBox"
            note="For content that is genuinely UNKNOWN — the window before the light phase \
                  resolves, where the row count is not known yet. Not for provisional: the \
                  light phase already returns a status and the heavy walk merely corrects \
                  it, so skeletonising a state label would hide information we already have \
                  and then reveal the same value. A provisional row renders dimmed and \
                  settles instead. \
                  \
                  It pulses rather than sweeping a gradient across itself, because a \
                  shimmer band has to be lighter than its base and 'lighter' is a lower \
                  grey step in light and a higher one in dark — a sweep needs a per-theme \
                  highlight, and fading opacity is correct in both by construction. Under \
                  prefers-reduced-motion the pulse stops and the bar stays. \
                  \
                  Each bar is aria-hidden; the REGION sets aria-busy. Getting that backwards \
                  makes a screen reader read out a dozen nameless boxes."
        >
            <Cell label="a text bar — the default height">
                <SkeletonBox width="140px" />
            </Cell>
            <Cell label="percentage width, for content of unknown length">
                <SkeletonBox width="60%" />
            </Cell>
            <Cell label="a state label's shape">
                <SkeletonBox width="88px" height="22px" />
            </Cell>
            <Cell label="a button's shape">
                <SkeletonBox width="76px" height="26px" />
            </Cell>
            <Cell label="a block">
                <SkeletonBox width="100%" height="64px" />
            </Cell>
            <Cell wide=true label="three bars — a paragraph's worth, ragged like real text">
                <div class="g-bars">
                    <SkeletonBox width="72%" />
                    <SkeletonBox width="88%" />
                    <SkeletonBox width="46%" />
                </div>
            </Cell>
        </Story>
    }
}

#[component]
fn Rows() -> impl IntoView {
    view! {
        <Story
            title="SkeletonBox — composed as rows"
            note="THE HEIGHT IS THE WHOLE JOB. Each cell puts skeleton rows directly above \
                  the real rows they stand in for — if the boundary between them is visible \
                  as a step, the list will jump when it settles, which is worse than no \
                  skeleton because the reflow lands exactly when the user starts reading. \
                  \
                  The two skeleton rows reuse their real row's own `.root` class rather \
                  than restating its padding, so equal height holds by construction and \
                  not by two numbers agreeing. They switch off the pointer cursor and the \
                  hover tint, because nothing in them responds to a click. \
                  \
                  Chrome is never skeletonised — the appbar, both cards, the section \
                  headings and the toolbar all render immediately. Only the queue and the \
                  two lists have an unknown state, which is why the card titles and counts \
                  below are real while their contents are not."
        >
            <Cell wide=true label="package list — four unknown rows above two real ones">
                <Card>
                    <div>
                        <GroupHeading title="s3://my-bucket" count=6 />
                        <div class="g-rows" aria-busy="true">
                            <PackageRowSkeleton />
                            <PackageRowSkeleton />
                            <PackageRowSkeleton />
                            <PackageRowSkeleton />
                        </div>
                        <PackageRow
                            namespace="user/package-a"
                            href="#skeleton"
                            changed_at=ago(2.0 * HOUR)
                            state="Latest"
                            tone=StateTone::Success
                        />
                        <PackageRow
                            namespace="user/package-b"
                            href="#skeleton"
                            changed_at=ago(5.0 * HOUR)
                            state="2 files changed"
                            tone=StateTone::Neutral
                        />
                    </div>
                </Card>
            </Cell>
            <Cell wide=true label="queue — three unknown rows above one real one">
                <Card title="Needs your attention">
                    <div>
                        <div class="g-rows" aria-busy="true">
                            <QueueRowSkeleton />
                            <QueueRowSkeleton />
                            <QueueRowSkeleton />
                        </div>
                        <QueueRow
                            namespace="org/dataset-c"
                            state="conflicts in 2 files"
                            tone=StateTone::Danger
                            action=view! {
                                <Button variant=ButtonVariant::Default on_click=|_| ()>
                                    "Publish"
                                </Button>
                            }
                                .into_any()
                        />
                    </div>
                </Card>
            </Cell>
            <Cell wide=true label="the whole region unknown — what the first paint shows">
                <Card title="Needs your attention">
                    <div class="g-rows" aria-busy="true">
                        <QueueRowSkeleton />
                        <QueueRowSkeleton />
                    </div>
                </Card>
            </Cell>
        </Story>
    }
}
