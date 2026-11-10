//! Every surface that tells the reader something, on one page, so they can be
//! compared: the activity line, the bell and its inbox, the update band, the
//! outcome band, and the acknowledgment that stands in place.
//!
//! Each tier has one surface. What runs now is the activity line. The result
//! of what the reader just did, while its page shows, is the outcome band.
//! What is true until it changes, such as a newer release, is a standing band.
//! What happened while the reader was away is the inbox. A copy or a save is
//! acknowledged where it happened, for one shared duration.

use leptos::context::Provider;
use leptos::prelude::*;

use crate::Cell;
use crate::Scene;
use crate::Story;
use crate::gallery::inbox::action_needed_in_queue;
use crate::gallery::inbox::every_kind;
use crate::gallery::packages::PackagesRegion;
use crate::pages;
use quilt_ui_kit::ACKNOWLEDGE_FOR;
use quilt_ui_kit::Activities;
use quilt_ui_kit::Activity;
use quilt_ui_kit::ActivityKind;
use quilt_ui_kit::Appbar;
use quilt_ui_kit::Banner;
use quilt_ui_kit::BannerVariant;
use quilt_ui_kit::Button;
use quilt_ui_kit::Inbox;
use quilt_ui_kit::PageLayout;
use quilt_ui_kit::StateTone;
use quilt_ui_kit::UpdateBand;
use quilt_ui_kit::UpdateState;
use quilt_ui_kit::ZeroLine;
use quilt_ui_kit::icons;

fn activity(kind: ActivityKind, label: &str) -> Activity {
    Activity {
        kind,
        label: label.to_string(),
        package: None,
    }
}

fn activities(autopull: Option<&str>, removing: Option<&str>) -> Activities {
    let all = Activities::new();
    if let Some(label) = autopull {
        all.set(
            ActivityKind::Autopull,
            vec![activity(ActivityKind::Autopull, label)],
        );
    }
    if let Some(label) = removing {
        all.set(
            ActivityKind::RemoveRevisions,
            vec![activity(ActivityKind::RemoveRevisions, label)],
        );
    }
    all
}

/// The bar's controls, the bell first.
fn controls() -> AnyView {
    let open = RwSignal::new(false);
    view! {
        <Inbox entries=every_kind() action_needed=action_needed_in_queue(1) open=open />
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
pub fn NotificationsScene() -> impl IntoView {
    view! {
        <WholeSystem />
        <ActivityLines />
        <OutcomeBands />
        <Acknowledge />
    }
}

#[component]
fn WholeSystem() -> impl IntoView {
    let outcome = RwSignal::new(true);
    let update = RwSignal::new(true);
    // Siblings, not wrapped: a row of hidden bands then takes no room.
    let bands = view! {
            <Show when=move || update.get()>
                <UpdateBand
                    state=UpdateState::Available {
                        version: "0.42.0".to_string(),
                    }
                    on_install=|()| ()
                    on_snooze=move |()| update.set(false)
                />
            </Show>
            <Show when=move || outcome.get()>
                <Banner variant=BannerVariant::Success on_dismiss=move |_| outcome.set(false)>
                    "Published user/plate-07."
                </Banner>
            </Show>
    }
    .into_any();

    view! {
        <Scene
            title="Scene · every notification at once"
            note="The calm main page with each surface showing. On the bar, the activity line \
                  says what runs now, and the bell leads the controls: amber because one \
                  thing needs you, with unread news behind it. Under the bar, the update \
                  band, which stands until the release is installed or snoozed, and the \
                  outcome band, which reports what you just did on this page. Open the bell \
                  for what happened while you were away. Nothing here floats but the \
                  inbox, and only because its bell opened it."
        >
            <div class="g-window">
                <Provider value=activities(Some("Getting latest for lab/rna-seq\u{2026}"), None)>
                    <PageLayout
                        heading="QuiltSync"
                        actions=controls()
                        banner=bands
                        page_scroll=true
                    >
                        <pages::QueueBand>
                            <ZeroLine text="1 package needs your attention" tone=StateTone::Attention />
                        </pages::QueueBand>
                        <PackagesRegion view_name="notifications-view" />
                    </PageLayout>
                </Provider>
            </div>
        </Scene>
    }
}

#[component]
fn ActivityLines() -> impl IntoView {
    let bar = |all: Activities| {
        view! {
            <div class="g-stack">
                <Provider value=all>
                    <Appbar />
                </Provider>
            </div>
        }
    };
    view! {
        <Story
            title="Activity line"
            note="What runs now, centred on the bar, and nothing once it is done: never kept, \
                  never dismissed, never an error. One slot per producer; autopull ranks \
                  first, since nobody started it. A pull shows here while its files change \
                  and lands in the inbox once they have. It fades in after a moment, so \
                  reload to watch it arrive."
        >
            <Cell full=true label="idle — the region is there, and empty">
                {bar(activities(None, None))}
            </Cell>
            <Cell full=true label="autopull">
                {bar(activities(Some("Getting latest for user/plate-07\u{2026}"), None))}
            </Cell>
            <Cell full=true label="removing old revisions, started on the package page">
                {bar(activities(None, Some("Removing old revisions of lab/rna-seq\u{2026}")))}
            </Cell>
            <Cell full=true label="both: autopull's ranks first">
                {bar(
                    activities(
                        Some("Publishing user/plate-07\u{2026}"),
                        Some("Removing old revisions of lab/rna-seq\u{2026}"),
                    ),
                )}
            </Cell>
        </Story>
    }
}

#[component]
fn OutcomeBands() -> impl IntoView {
    view! {
        <Story
            title="Outcome band"
            note="The result of what you just did, on the page that did it, while that page \
                  shows; leave the page and it is dropped. The page words the lead and the \
                  engine's refusal follows it. A result that outlives its page, such as an \
                  uninstall's freed space, goes to the inbox instead."
        >
            <Cell full=true label="success">
                <div class="g-stack">
                <Banner variant=BannerVariant::Success on_dismiss=|_| ()>
                    "Published user/plate-07."
                </Banner>
                </div>
            </Cell>
            <Cell full=true label="warning">
                <div class="g-stack">
                <Banner variant=BannerVariant::Warning on_dismiss=|_| ()>
                    "Got the latest revision of user/plate-07. 2 files were left as they \
                     were, since you changed them here."
                </Banner>
                </div>
            </Cell>
            <Cell full=true label="a failure, with the engine's words after the page's">
                <div class="g-stack">
                <Banner variant=BannerVariant::Critical on_dismiss=|_| ()>
                    "Could not publish user/plate-07. Access denied for role analyst."
                </Banner>
                </div>
            </Cell>
        </Story>
    }
}

#[component]
fn Acknowledge() -> impl IntoView {
    let said = RwSignal::new(false);
    let pending: StoredValue<Option<TimeoutHandle>> = StoredValue::new(None);
    let clear = move || {
        if let Some(Some(earlier)) = pending.try_update_value(Option::take) {
            earlier.clear();
        }
    };
    let press = move |_| {
        said.set(true);
        clear();
        let later = set_timeout_with_handle(
            move || {
                said.try_set(false);
            },
            ACKNOWLEDGE_FOR,
        )
        .ok();
        pending.set_value(later);
    };
    on_cleanup(clear);
    view! {
        <Story
            title="Acknowledgment"
            note="A copy mark, a row's Copied, the accounts card's announcement and the \
                  settings page's Saved all stand for one shared duration, then go. Press \
                  to time it."
        >
            <Cell label="Saved, for the shared duration">
                <div class="g-inline" style="align-items:center">
                    <Button on_click=press>"Save"</Button>
                    <span role="status">{move || said.get().then_some("Saved")}</span>
                </div>
            </Cell>
        </Story>
    }
}
