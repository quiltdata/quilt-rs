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

#[component]
pub fn FeedbackStories() -> impl IntoView {
    view! { <Notices /> <Spinners /> }
}

#[component]
fn Notices() -> impl IntoView {
    view! {
        <Story
            title="Banner"
            note="Three kinds, and the type has exactly three variants rather than reusing \
                  StateTone's four — Neutral has no meaning for an outcome, and a type that \
                  cannot express it beats a note asking people not to. They map onto the \
                  page's tones internally, so a warning here and a warning on a row cannot \
                  disagree about what amber means. \
                  \
                  Error takes role=alert and the other two role=status: an error has to cut \
                  across what is being read because the thing the user asked for did not \
                  happen, and a success does not, because it did. \
                  \
                  It does not dismiss itself. Auto-dismiss is a policy about the OPERATION \
                  — a publish confirmation can go quietly, a failure must not — and only \
                  the caller knows which. It animates in and not out: an exit needs the \
                  node to outlive the state that produced it, and once the signal is None \
                  there is nothing left to fade."
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
            note="No longer the page's loading state — SkeletonBox is, because a skeleton holds \
                  the space the content will take and a spinner says only that you are \
                  waiting. This is for work whose SHAPE is unknown, which leaves two jobs: \
                  inline beside a label that names the work, and filling a region whose \
                  contents are not a list of rows. \
                  \
                  Inline usually passes no label and is aria-hidden, because the text \
                  beside it already says what is happening and announcing it twice is \
                  worse. A region spinner has no such text, so it always names itself. \
                  \
                  Button keeps its OWN spinner rather than using this one: it draws it as a \
                  ::before on the leading slot so a button with an icon does not change \
                  width when work starts, and swapping in an element would put that width \
                  at the mercy of a child. Ten duplicated lines, against coupling a \
                  button's geometry to another component."
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
                <IconButton icon=gear_icon() aria_label="Working" on_click=|_| () spinning=true />
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
            note="Under the appbar, in the flow, pushing the page down — not floating over \
                  it. Anchored positioning is banned by the design, and that ban is what \
                  keeps the whole tooltip/popover/dropdown class out of the codebase; a bar \
                  also cannot be missed by someone looking at the bottom of a long list, \
                  which is exactly where a corner toast fails. \
                  \
                  Its width is capped like the appbar's contents and the page column, so it \
                  lines up with the regions rather than with the window. Dismiss it and it \
                  is gone — the caller owns that, which is why PageLayout takes a slot and not \
                  the signal."
            >
            <PageLayout
                actions=view! {
                    <IconButton icon=gear_icon() aria_label="Settings" on_click=|_| () />
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
                        sublabel="Every 30s, when nothing is changed here"
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
