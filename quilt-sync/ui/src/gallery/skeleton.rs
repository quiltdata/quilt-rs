//! `SkeletonBox` stories.
//!
//! The composed cells are the ones that matter. A bar on its own proves nothing; a
//! skeleton row sitting directly above the real row it stands in for proves the only
//! thing worth proving, which is that they are the same height.

use leptos::prelude::*;

use crate::Cell;
use crate::Story;
use crate::gallery::queue::row;
use crate::kit::Card;
use crate::kit::GroupHeading;
use crate::kit::PackageRow;
use crate::kit::PackageRowSkeleton;
use crate::kit::PackageState;
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
            note="For content that is genuinely unknown — the window before the light phase \
                  resolves, where the row count is not known yet. Not for provisional: a row \
                  whose status will merely be corrected renders dimmed and settles instead. \
                  It pulses rather than sweeping a gradient, because a shimmer band needs a \
                  per-theme highlight while fading opacity is correct in both; under \
                  `prefers-reduced-motion` the bar stays still. Each bar is `aria-hidden` \
                  and the region sets `aria-busy` — backwards, a reader gets a dozen \
                  nameless boxes."
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
                <SkeletonBox width="76px" height="32px" />
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
            note="The height is the whole job. Each cell puts skeleton rows directly above \
                  the real rows they stand in for: if the boundary reads as a step, the list \
                  will jump when it settles, exactly when somebody starts reading. The \
                  skeleton rows reuse the real row's own `.root` class rather than restating \
                  its padding, so equal height holds by construction. Chrome is never \
                  skeletonised — only the queue and the two lists have an unknown state."
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
                        {row(
                            "org/dataset-c",
                            &PackageState::PullConflict {
                                files: vec!["a.csv".to_string(), "b.csv".to_string()],
                            },
                        )}
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
