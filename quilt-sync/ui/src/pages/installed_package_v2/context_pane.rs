//! The first complete slice of the package context pane.
//!
//! One current revision, the bucket it belongs to, and the revisions this copy
//! holds, loaded when their popover opens. Scope and resolution controls arrive
//! with the data and actions that can make them truthful.

use std::future::Future;
use std::pin::Pin;

use leptos::prelude::*;

use super::revision_history::{History, Key, Shown};
use crate::commands;
use crate::kit::{
    Align, AnchoredOverlay, Button, Card, CatalogLink, LoadFailure, PaneSection, RevisionRow,
    SkeletonBox,
};

stylance::import_crate_style!(
    style,
    "src/pages/installed_package_v2/context_pane.module.scss"
);

/// Where the popover's rows come from. A function pointer rather than a
/// direct call, so the DOM tests and the gallery answer without a Tauri host.
pub type RevisionHistoryFetch =
    fn(String) -> Pin<Box<dyn Future<Output = Result<Vec<commands::RevisionHistoryRow>, String>>>>;

/// The app's fetch.
pub(super) fn fetch_revision_history(
    namespace: String,
) -> Pin<Box<dyn Future<Output = Result<Vec<commands::RevisionHistoryRow>, String>>>> {
    Box::pin(commands::get_revision_history(namespace))
}

/// The skeleton's bar widths, at the lengths messages actually have, so the
/// surface does not resize under the cursor when the rows land.
const BAR_WIDTHS: [&str; 4] = ["200px", "228px", "164px", "188px"];

/// The current revision and package bucket, and the revisions this copy holds.
#[component]
pub fn CurrentRevisionPane(
    data: commands::PackageContextData,
    /// Keys the popover's answers; see `revision_history::History`.
    #[prop(into)]
    namespace: String,
    fetch: RevisionHistoryFetch,
    /// Opens a published row's catalog address. The page's reports failure
    /// on the band; the gallery's does nothing.
    open_catalog: Callback<String>,
) -> impl IntoView {
    let bucket = data.bucket.filter(|bucket| !bucket.is_empty()).map_or_else(
        || "No S3 bucket".to_string(),
        |bucket| format!("s3://{bucket}"),
    );
    let count = data.revision_count;

    let open = RwSignal::new(false);
    let history = RwSignal::new(History::new(namespace.clone()));
    let namespace = StoredValue::new(namespace);

    // `try_update`: a re-read disposes this pane, and a late answer then lands nowhere.
    let load = move |key: Key| {
        let ns = namespace.get_value();
        leptos::task::spawn_local(async move {
            let answer = fetch(ns).await;
            if let Err(detail) = &answer {
                leptos::logging::warn!("Could not load the revision history: {detail}");
            }
            history.try_update(|h| h.settle(&key, answer));
        });
    };

    // The previous value, so the initial `false` closes nothing.
    Effect::new(move |was_open: Option<bool>| {
        let is_open = open.get();
        match (was_open.unwrap_or(false), is_open) {
            (false, true) => {
                if let Some(key) = history.try_update(History::open) {
                    load(key);
                }
            }
            (true, false) => history.update(History::close),
            _ => {}
        }
        is_open
    });

    let retry = Callback::new(move |()| {
        if let Some(key) = history.try_update(History::retry) {
            load(key);
        }
    });

    let trigger = move |surface_id: String| {
        view! {
            // Never disabled: the history is a read, outside the page's command lock.
            <Button
                on_click=move |_| open.update(|o| *o = !*o)
                aria_expanded=open
                aria_controls=surface_id
            >
                {format!("Revisions you have ({count})")}
            </Button>
        }
        .into_any()
    };

    let loading = move || history.with(|h| matches!(h.shown(), Shown::Loading));
    // The overlay renders its body once, so the three states are one closure.
    let body = move || {
        let shown = history.with(|h| h.shown().clone());
        surface_body(shown, count, open_catalog, retry)
    };

    view! {
        <aside aria-label="About this package" class=style::root>
            // The card is visually titleless, but its hidden h2 keeps the
            // PaneSection's h3 in a complete heading hierarchy.
            <Card label="About this package">
                <PaneSection label="Revision">
                    <RevisionRow
                        message=data.revision.message.unwrap_or_default()
                        at=data.revision.obtained_at
                    />
                    <span class=style::bucket>{bucket}</span>
                    <AnchoredOverlay
                        trigger=trigger
                        open=open
                        aria_label="Revisions you have"
                        align=Align::End
                    >
                        // The live region is the caller's (`kit/load_failure.rs`).
                        <div
                            aria-live="polite"
                            aria-busy=move || if loading() { "true" } else { "false" }
                        >
                            {body}
                        </div>
                    </AnchoredOverlay>
                </PaneSection>
            </Card>
        </aside>
    }
}

/// What the popover draws for one state of its history.
fn surface_body(
    shown: Shown,
    count: usize,
    open_catalog: Callback<String>,
    retry: Callback<()>,
) -> AnyView {
    match shown {
        Shown::Loading => view! {
            <PaneSection>
                {BAR_WIDTHS
                    .iter()
                    .take(count.min(BAR_WIDTHS.len()))
                    .map(|width| view! { <SkeletonBox width=*width /> })
                    .collect_view()}
            </PaneSection>
        }
        .into_any(),
        Shown::Rows(rows) => view! {
            <PaneSection>
                {rows
                    .into_iter()
                    .map(|row| {
                        view! {
                            <RevisionRow
                                message=row.message.unwrap_or_default()
                                at=row.obtained_at
                                published=row.published
                                catalog=row
                                    .catalog_url
                                    .map(|href| CatalogLink::new(href, open_catalog))
                            />
                        }
                    })
                    .collect_view()}
            </PaneSection>
        }
        .into_any(),
        Shown::Failed => view! {
            <LoadFailure words="Could not load your revisions." on_retry=retry />
        }
        .into_any(),
    }
}

/// The pane's loading geometry, occupying the same card and section structure.
#[component]
pub fn CurrentRevisionPaneSkeleton() -> impl IntoView {
    view! {
        <aside aria-label="About this package" class=style::root>
            <Card label="About this package" busy=Signal::stored(true)>
                <PaneSection label="Revision">
                    <SkeletonBox width="80%" />
                    <SkeletonBox width="42%" />
                    <SkeletonBox width="68%" />
                </PaneSection>
            </Card>
        </aside>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{CurrentRevisionData, RevisionHistoryRow};
    use crate::test_support::{element_saying, mount, sleep_ms};
    use std::cell::Cell;
    use std::future::Future;
    use std::pin::Pin;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::*;

    const PUBLISHED_URL: &str =
        "https://test.quilt.dev/b/test/packages/team/dataset/tree/published-hash";

    type Answer = Pin<Box<dyn Future<Output = Result<Vec<RevisionHistoryRow>, String>>>>;

    fn data(
        message: Option<&str>,
        bucket: Option<&str>,
        revision_count: usize,
    ) -> commands::PackageContextData {
        commands::PackageContextData {
            revision: CurrentRevisionData {
                message: message.map(ToString::to_string),
                obtained_at: 1_758_500_000_000.0,
            },
            bucket: bucket.map(ToString::to_string),
            revision_count,
            keeping: commands::KeepingData {
                scope: commands::KeepingScope::IndividualFiles,
                total: 1,
                remote_only: Vec::new(),
            },
        }
    }

    fn published_row() -> RevisionHistoryRow {
        RevisionHistoryRow {
            message: Some("Sent".to_string()),
            obtained_at: 1_758_500_000_000.0,
            published: true,
            catalog_url: Some(PUBLISHED_URL.to_string()),
        }
    }

    fn unpublished_row() -> RevisionHistoryRow {
        RevisionHistoryRow {
            message: Some("Kept here".to_string()),
            obtained_at: 1_758_400_000_000.0,
            published: false,
            catalog_url: None,
        }
    }

    fn rows(_: String) -> Answer {
        Box::pin(async { Ok(vec![published_row(), unpublished_row()]) })
    }

    fn never(_: String) -> Answer {
        Box::pin(std::future::pending())
    }

    thread_local! {
        static REFUSALS: Cell<u32> = const { Cell::new(0) };
    }

    fn refuses(_: String) -> Answer {
        REFUSALS.with(|n| n.set(n.get() + 1));
        Box::pin(async { Err("AccessDenied".into()) })
    }

    /// The pane over `count` revisions, answering from `fetch`.
    fn pane(
        count: usize,
        fetch: RevisionHistoryFetch,
        open_catalog: Callback<String>,
    ) -> web_sys::Element {
        mount(move || {
            view! {
                <CurrentRevisionPane
                    data=data(Some("Initial upload"), Some("quilt-lab-plates"), count)
                    namespace="team/dataset"
                    fetch=fetch
                    open_catalog=open_catalog
                />
            }
        })
    }

    fn ignore() -> Callback<String> {
        Callback::new(|_: String| ())
    }

    /// A task boundary and a tick: the fetch settles on the event loop, then the DOM.
    async fn settle() {
        sleep_ms(0).await;
        leptos::task::tick().await;
    }

    fn trigger(el: &web_sys::Element) -> web_sys::HtmlElement {
        el.query_selector("button[aria-expanded]")
            .unwrap()
            .expect("the revisions trigger")
            .unchecked_into()
    }

    /// What the trigger says it controls.
    fn surface(el: &web_sys::Element) -> web_sys::Element {
        let id = trigger(el)
            .get_attribute("aria-controls")
            .expect("the trigger names what it opens");
        web_sys::window()
            .unwrap()
            .document()
            .unwrap()
            .get_element_by_id(&id)
            .expect("an element with that id")
    }

    fn live_region(el: &web_sys::Element) -> web_sys::Element {
        surface(el)
            .query_selector("[aria-live='polite']")
            .unwrap()
            .expect("the surface's live region")
    }

    #[wasm_bindgen_test]
    fn the_pane_names_and_renders_the_current_revision() {
        let el = pane(4, never, ignore());

        let aside = el.query_selector("aside").unwrap().expect("an aside");
        assert_eq!(
            aside.get_attribute("aria-label").as_deref(),
            Some("About this package")
        );
        element_saying(&el, "Revision");
        assert!(
            el.text_content()
                .unwrap_or_default()
                .contains("Initial upload"),
            "RevisionRow quotes the message, but preserves its words"
        );
        element_saying(&el, "s3://quilt-lab-plates");

        let time = el
            .query_selector("time")
            .unwrap()
            .expect("an obtained time");
        assert!(
            time.get_attribute("datetime")
                .is_some_and(|value| !value.is_empty()),
            "the machine-readable time travels with the relative one"
        );
        let buttons = el.query_selector_all("button").unwrap();
        assert_eq!(
            buttons.length(),
            1,
            "the trigger is the slice's one control; markup was {}",
            el.inner_html()
        );
        assert_eq!(
            buttons
                .item(0)
                .unwrap()
                .text_content()
                .unwrap_or_default()
                .trim(),
            "Revisions you have (4)"
        );
        assert!(
            el.query_selector("a, input").unwrap().is_none(),
            "no link or input while closed; markup was {}",
            el.inner_html()
        );
    }

    #[wasm_bindgen_test]
    fn absent_facts_have_honest_words() {
        for message in [None, Some("")] {
            let el = mount(move || {
                view! {
                    <CurrentRevisionPane
                        data=data(message, None, 1)
                        namespace="team/dataset"
                        fetch=never
                        open_catalog=ignore()
                    />
                }
            });
            element_saying(&el, "No message");
            element_saying(&el, "No S3 bucket");
        }
    }

    #[wasm_bindgen_test]
    fn the_trigger_states_the_count_and_what_it_opens() {
        let el = pane(4, never, ignore());
        let button = trigger(&el);
        assert_eq!(
            button.text_content().unwrap_or_default().trim(),
            "Revisions you have (4)"
        );
        assert_eq!(
            button.get_attribute("aria-expanded").as_deref(),
            Some("false")
        );
        assert_eq!(
            surface(&el).get_attribute("aria-label").as_deref(),
            Some("Revisions you have")
        );
    }

    /// The surface opens at once into bars, one per revision up to four, and
    /// the region says it is busy until the answer lands.
    #[wasm_bindgen_test]
    async fn opening_draws_the_skeleton_until_the_answer() {
        for (count, bars) in [(2, 2), (6, 4)] {
            let el = pane(count, never, ignore());
            trigger(&el).click();
            settle().await;

            assert_eq!(
                trigger(&el).get_attribute("aria-expanded").as_deref(),
                Some("true")
            );
            assert_eq!(
                live_region(&el).get_attribute("aria-busy").as_deref(),
                Some("true")
            );
            let drawn = surface(&el)
                .query_selector_all("[aria-hidden='true']")
                .unwrap()
                .length();
            assert_eq!(
                drawn,
                bars,
                "{count} revisions; markup was {}",
                el.inner_html()
            );
        }
    }

    #[wasm_bindgen_test]
    async fn the_rows_arrive_published_and_not() {
        let el = pane(2, rows, ignore());
        trigger(&el).click();
        settle().await;
        settle().await;

        let surface = surface(&el);
        assert_eq!(
            live_region(&el).get_attribute("aria-busy").as_deref(),
            Some("false")
        );
        element_saying(&surface, "Published");
        element_saying(&surface, "Not published");

        let links = surface.query_selector_all("a").unwrap();
        assert_eq!(links.length(), 1, "only the published row links");
        let link = surface
            .query_selector(&format!("a[href=\"{PUBLISHED_URL}\"]"))
            .unwrap()
            .expect("the published row links to its exact revision");
        assert_eq!(
            link.get_attribute("aria-label").as_deref(),
            Some("Open in catalog")
        );
        let sent = element_saying(&surface, "\u{201c}Sent\u{201d}");
        assert!(
            sent.closest("a").unwrap().is_none(),
            "the message is text beside the link, not inside it"
        );
        assert!(
            surface
                .text_content()
                .unwrap_or_default()
                .contains("Kept here"),
            "the unpublished row is text"
        );
        assert_eq!(
            surface
                .query_selector_all("time[datetime]")
                .unwrap()
                .length(),
            2,
            "each row says when"
        );
    }

    #[wasm_bindgen_test]
    async fn a_refusal_offers_a_retry_that_asks_again() {
        REFUSALS.with(|n| n.set(0));
        let el = pane(2, refuses, ignore());
        trigger(&el).click();
        settle().await;
        settle().await;

        let surface = surface(&el);
        element_saying(&surface, "Could not load your revisions.");
        assert_eq!(REFUSALS.with(Cell::get), 1);

        element_saying(&surface, "Try again").click();
        settle().await;
        assert_eq!(REFUSALS.with(Cell::get), 2, "the retry asked again");
    }

    #[wasm_bindgen_test]
    async fn a_published_link_opens_through_the_callback() {
        let opened: RwSignal<Option<String>> = RwSignal::new(None);
        let el = pane(
            2,
            rows,
            Callback::new(move |url: String| opened.set(Some(url))),
        );
        trigger(&el).click();
        settle().await;
        settle().await;

        let before = web_sys::window().unwrap().location().href().unwrap();
        surface(&el)
            .query_selector("[aria-label='Open in catalog']")
            .unwrap()
            .expect("the published row's link")
            .unchecked_into::<web_sys::HtmlElement>()
            .click();
        leptos::task::tick().await;

        assert_eq!(opened.get_untracked().as_deref(), Some(PUBLISHED_URL));
        assert_eq!(
            web_sys::window().unwrap().location().href().unwrap(),
            before,
            "the test page did not navigate"
        );
    }

    #[wasm_bindgen_test]
    fn the_skeleton_marks_the_composing_region_busy() {
        let el = mount(|| view! { <CurrentRevisionPaneSkeleton /> });
        let card = el
            .query_selector("aside > section")
            .unwrap()
            .expect("the composing card");
        assert_eq!(card.get_attribute("aria-busy").as_deref(), Some("true"));

        let bars = el.query_selector_all("[aria-hidden='true']").unwrap();
        assert_eq!(bars.length(), 3);
    }
}
