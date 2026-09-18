//! `Banner` and `Spinner` stories, plus the banner in its real position.

use leptos::prelude::*;

use crate::Cell;
use crate::Scene;
use crate::Story;
use crate::kit::Banner;
use crate::kit::BannerVariant;
use crate::kit::Button;
use crate::kit::Card;
use crate::kit::IconButton;
use crate::kit::PageLayout;
use crate::kit::Spinner;
use crate::kit::SpinnerVariant;
use crate::kit::ToggleRow;
use crate::kit::icons;

#[component]
pub fn FeedbackStories() -> impl IntoView {
    view! { <Notices /> <Spinners /> }
}

#[component]
fn Notices() -> impl IntoView {
    view! {
        <Story
            title="Banner"
            note="Three kinds, and three variants rather than reusing StateTone's four — \
                  Neutral means nothing for an outcome. Error takes `role=alert` and the \
                  other two `role=status`. It does not dismiss itself: auto-dismiss is a \
                  policy about the operation, and only the caller knows whether a failure \
                  may go quietly. It animates in and not out, because once the signal is \
                  `None` there is nothing left to fade."
        >
            <Cell full=true label="success">
                <Banner variant=BannerVariant::Success on_dismiss=|_| ()>
                    "Published user/package-b."
                </Banner>
            </Cell>
            <Cell full=true label="warning — it worked, but something adjacent needs you">
                <Banner variant=BannerVariant::Warning on_dismiss=|_| ()>
                    "Bucket set to s3://team-bucket. Its default workflow could not be \
                     resolved, so publishing will use no workflow until you choose one."
                </Banner>
            </Cell>
            <Cell full=true label="error">
                <Banner variant=BannerVariant::Critical on_dismiss=|_| ()>
                    "Could not publish user/package-b: access denied for role analyst."
                </Banner>
            </Cell>
            <Cell full=true label="long message wraps — the glyph stays on the first line">
                <Banner variant=BannerVariant::Critical on_dismiss=|_| ()>
                    "Could not get the latest revision of \
                     team/rnaseq-batch-2026-07-31-reprocessed-v2: the manifest at \
                     s3://team-bucket/.quilt/named_packages/ refers to a top hash that is \
                     not present in the bucket, which usually means the package was \
                     rewritten while this machine was offline."
                </Banner>
            </Cell>
        </Story>
    }
}

#[component]
fn Spinners() -> impl IntoView {
    view! {
        <Story
            title="Spinner"
            note="Not the page's loading state — SkeletonBox is. This is for work whose \
                  shape is unknown, which leaves two jobs: inline beside a label that names \
                  the work, where it stays aria-hidden, and filling a region with no such \
                  text, where it names itself. Button keeps its own spinner, drawn as a \
                  `::before` on the leading slot, so a button with an icon does not change \
                  width when work starts."
        >
            <Cell label="inline, beside text that names the work">
                <span>
                    <Spinner />
                    " Checking for new revisions…"
                </span>
            </Cell>
            <Cell label="inline, on its own — so it names itself">
                <Spinner aria_label="Signing in" />
            </Cell>
            <Cell label="beside Button's own, which is not this component">
                <div class="g-inline">
                    <Button on_click=|_| () loading=true>
                        "Publishing…"
                    </Button>
                    <span>
                        <Spinner />
                        " same ring"
                    </span>
                </div>
            </Cell>
            <Cell label="IconButton's spinning glyph — a third spelling, same treatment">
                <IconButton icon=icons::gear() aria_label="Working" on_click=|_| () spinning=true />
            </Cell>
            <Cell wide=true label="region — for content that is not rows">
                <Card title="Account">
                    <Spinner variant=SpinnerVariant::Region aria_label="Loading your roles" />
                </Card>
            </Cell>
        </Story>
    }
}

#[component]
pub fn BannerScene() -> impl IntoView {
    let variant = RwSignal::new(Some(BannerVariant::Critical));
    let pull = RwSignal::new(true);

    view! {
        <Scene
            title="Scene · a banner in place"
            note="Under the appbar, in the flow, pushing the page down rather than floating \
                  over it. A bar cannot be missed by somebody reading the bottom of a long \
                  list, which is where a corner toast fails. Its width is capped like the \
                  appbar's contents, so it lines up with the regions rather than with the \
                  window. Dismiss it and it is gone — the caller owns that."
            >
            <PageLayout
                heading="QuiltSync"
                actions=view! {
                    <IconButton icon=icons::gear() aria_label="Settings" on_click=|_| () />
                }
                    .into_any()
                banner=view! {
                    <Show when=move || variant.get().is_some()>
                        <Banner
                            variant=variant.get().unwrap_or(BannerVariant::Critical)
                            on_dismiss=move |_| variant.set(None)
                        >
                            "Could not publish user/package-b: access denied for role analyst."
                        </Banner>
                    </Show>
                }
                    .into_any()
            >
                <Card title="Autosync">
                    <ToggleRow
                        label="Get new revisions"
                        sublabel="Every 30s, keeping any local changes"
                        checked=pull
                        trailing=view! { "0:23" }.into_any()
                    />
                </Card>
                <Card title="Needs your attention">
                    <div>
                        <Button on_click=move |_| variant.set(Some(BannerVariant::Success))>
                            "Show a success instead"
                        </Button>
                    </div>
                </Card>
            </PageLayout>
        </Scene>
    }
}
