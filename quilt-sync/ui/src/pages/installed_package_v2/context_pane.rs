//! The first complete slice of the package context pane.
//!
//! One current revision, the bucket it belongs to, the revisions this copy
//! holds, loaded when their popover opens, and what this copy keeps. This is
//! the pane's ordinary mode; the page swaps it whole for `resolve.rs`'s
//! `ResolvePane` while the resolve mode is open.

use std::future::Future;
use std::pin::Pin;

use leptos::prelude::*;

use super::Wiring;
use super::keeping::{KeepingCommands, KeepingSection};
use super::old_revisions::{self, RemoveRevisionsFn, Remover};
use super::revision_history::{History, Key, Shown};
use crate::commands;
use crate::kit::{
    Align, AnchoredOverlay, Button, Card, LoadFailure, PaneSection, RevisionRow, SkeletonBox,
};

stylance::import_crate_style!(
    style,
    "src/pages/installed_package_v2/context_pane.module.scss"
);

/// Where the popover's rows come from. A function pointer rather than a
/// direct call, so the DOM tests and the gallery answer without a Tauri host.
pub type RevisionHistoryFetch =
    fn(String) -> Pin<Box<dyn Future<Output = Result<commands::RevisionHistoryData, String>>>>;

/// The app's fetch.
pub(super) fn fetch_revision_history(
    namespace: String,
) -> Pin<Box<dyn Future<Output = Result<commands::RevisionHistoryData, String>>>> {
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
    /// The page's command lock and band, which Keeping's commands take.
    w: Wiring,
    commands: KeepingCommands,
    /// Removes old revisions from the popover. `None` offers no removal.
    #[prop(optional)]
    remove: Option<RemoveRevisionsFn>,
) -> impl IntoView {
    let keeping = data.keeping;
    let bucket = data.bucket.filter(|bucket| !bucket.is_empty()).map_or_else(
        || "No S3 bucket".to_string(),
        |bucket| format!("s3://{bucket}"),
    );
    let count = data.revision_count;

    let open = RwSignal::new(false);
    let history = RwSignal::new(History::new(namespace.clone()));
    let namespace = StoredValue::new(namespace);
    let remover = remove.map(|remove| Remover::new(namespace, w, remove));

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
        surface_body(shown, count, open_catalog, retry, remover)
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
                <KeepingSection
                    namespace=namespace.get_value()
                    data=keeping
                    w=w
                    commands=commands
                />
            </Card>
            // Beside the popover, not in it: see `old_revisions`.
            {remover.map(old_revisions::dialog)}
        </aside>
    }
}

/// What the popover draws for one state of its history.
fn surface_body(
    shown: Shown,
    count: usize,
    open_catalog: Callback<String>,
    retry: Callback<()>,
    remover: Option<Remover>,
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
        Shown::Rows(data) => old_revisions::rows_and_footer(data, open_catalog, remover),
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
    use crate::commands::{
        CurrentRevisionData, KeptReason, RevisionHistoryData, RevisionHistoryRow,
    };
    use crate::pages::installed_package_v2::Wiring;
    use crate::pages::installed_package_v2::keeping::KeepingCommands;
    use crate::pages::installed_package_v2::old_revisions::RemoveRevisionsFn;
    use crate::test_support::{element_saying, mount, sleep_ms};
    use std::cell::Cell;
    use std::future::Future;
    use std::pin::Pin;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::*;

    const PUBLISHED_URL: &str =
        "https://quilt.test/b/test/packages/team/dataset/tree/published-hash";

    type Answer = Pin<Box<dyn Future<Output = Result<RevisionHistoryData, String>>>>;

    fn data(
        message: Option<&str>,
        bucket: Option<&str>,
        revision_count: usize,
    ) -> commands::PackageContextData {
        commands::PackageContextData {
            revision: CurrentRevisionData {
                hash: "abc123".to_string(),
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
            resolve: None,
        }
    }

    fn published_row() -> RevisionHistoryRow {
        RevisionHistoryRow {
            hash: "published-hash".to_string(),
            message: Some("Sent".to_string()),
            obtained_at: 1_758_500_000_000.0,
            published: true,
            catalog_url: Some(PUBLISHED_URL.to_string()),
            kept: Vec::new(),
            frees: None,
        }
    }

    fn unpublished_row() -> RevisionHistoryRow {
        RevisionHistoryRow {
            hash: "local-hash".to_string(),
            message: Some("Kept here".to_string()),
            obtained_at: 1_758_400_000_000.0,
            published: false,
            catalog_url: None,
            kept: vec![KeptReason::Unpublished],
            frees: None,
        }
    }

    fn rows(_: String) -> Answer {
        Box::pin(async {
            Ok(RevisionHistoryData {
                rows: vec![published_row(), unpublished_row()],
                removable_frees: None,
            })
        })
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
                    w=Wiring::new()
                    commands=idle_commands()
                />
            }
        })
    }

    fn stores_ok(_: String, _: bool) -> Pin<Box<dyn Future<Output = Result<(), String>>>> {
        Box::pin(async { Ok(()) })
    }

    fn downloads_ok(
        _: String,
        _: Vec<String>,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>>>> {
        Box::pin(async { Ok(()) })
    }

    fn idle_commands() -> KeepingCommands {
        KeepingCommands {
            store: stores_ok,
            download: downloads_ok,
        }
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

    fn removal_row(
        hash: &str,
        message: &str,
        kept: Vec<KeptReason>,
        frees: Option<u64>,
    ) -> RevisionHistoryRow {
        RevisionHistoryRow {
            hash: hash.to_string(),
            message: Some(message.to_string()),
            obtained_at: 1_758_500_000_000.0,
            published: true,
            catalog_url: Some(PUBLISHED_URL.to_string()),
            kept,
            frees,
        }
    }

    /// One kept revision and two old ones, the set freeing more than its rows.
    fn removable(_: String) -> Answer {
        Box::pin(async {
            Ok(RevisionHistoryData {
                rows: vec![
                    removal_row(
                        "mine",
                        "Mine",
                        vec![KeptReason::Current, KeptReason::NotPushed],
                        None,
                    ),
                    removal_row("old-1", "Old", Vec::new(), Some(1_200_000)),
                    removal_row("old-2", "Older", Vec::new(), Some(0)),
                ],
                removable_frees: Some(6_900_000),
            })
        })
    }

    fn all_kept(_: String) -> Answer {
        Box::pin(async {
            Ok(RevisionHistoryData {
                rows: vec![
                    removal_row("mine", "Mine", vec![KeptReason::Current], None),
                    removal_row(
                        "tip",
                        "Tip",
                        vec![KeptReason::Latest, KeptReason::Base],
                        None,
                    ),
                ],
                removable_frees: None,
            })
        })
    }

    thread_local! {
        static REMOVED: std::cell::RefCell<Vec<(String, Vec<String>)>> =
            const { std::cell::RefCell::new(Vec::new()) };
    }

    type Removed = Pin<Box<dyn Future<Output = Result<String, String>>>>;

    fn removes_ok(namespace: String, hashes: Vec<String>) -> Removed {
        REMOVED.with(|r| r.borrow_mut().push((namespace, hashes)));
        Box::pin(async { Ok(String::new()) })
    }

    fn removes_never(_: String, _: Vec<String>) -> Removed {
        Box::pin(std::future::pending())
    }

    fn removes_busy(_: String, _: Vec<String>) -> Removed {
        Box::pin(async { Err("team/dataset is busy in another quilt process".to_string()) })
    }

    /// The pane over `fetch`, removing with `remove`, on `w`, under `activities`.
    fn removal_pane(
        fetch: RevisionHistoryFetch,
        remove: RemoveRevisionsFn,
        w: Wiring,
        activities: crate::kit::Activities,
    ) -> web_sys::Element {
        mount(move || {
            provide_context(activities);
            view! {
                <CurrentRevisionPane
                    data=data(Some("Mine"), Some("quilt-lab-plates"), 3)
                    namespace="team/dataset"
                    fetch=fetch
                    open_catalog=ignore()
                    w=w
                    commands=idle_commands()
                    remove=remove
                />
            }
        })
    }

    async fn opened(el: &web_sys::Element) -> web_sys::Element {
        trigger(el).click();
        settle().await;
        settle().await;
        surface(el)
    }

    /// The surface with its rows drawn again: the confirmation's modal closes
    /// the popover, so it may take a press to open it and one more if the
    /// trigger still thought it open.
    async fn reopened(el: &web_sys::Element, surface: &web_sys::Element) -> web_sys::Element {
        for _ in 0..2 {
            if surface
                .query_selector("button[aria-label^='Remove']")
                .unwrap()
                .is_some()
            {
                break;
            }
            trigger(el).click();
            settle().await;
            settle().await;
        }
        surface.clone()
    }

    fn button_named(root: &web_sys::Element, label: &str) -> web_sys::HtmlButtonElement {
        root.query_selector(&format!("button[aria-label='{label}']"))
            .unwrap()
            .unwrap_or_else(|| panic!("no {label:?}; markup was {}", root.inner_html()))
            .unchecked_into()
    }

    #[wasm_bindgen_test]
    async fn a_removable_row_says_what_it_frees_and_a_kept_one_why() {
        let el = removal_pane(
            removable,
            removes_ok,
            Wiring::new(),
            crate::kit::Activities::new(),
        );
        let surface = opened(&el).await;

        element_saying(&surface, "frees 1.2 MB");
        element_saying(&surface, "frees nothing");
        let tag = element_saying(&surface, "current \u{b7} not pushed");
        assert_eq!(
            tag.get_attribute("title").as_deref(),
            Some("Kept: your files are at this revision, and it has not been pushed.")
        );
        button_named(&surface, "Remove \u{201c}Old\u{201d}");
        assert!(
            surface
                .query_selector("button[aria-label='Remove \u{201c}Mine\u{201d}']")
                .unwrap()
                .is_none(),
            "a kept row has no trash"
        );
        element_saying(&surface, "Remove 2 older \u{b7} frees 6.9 MB");
    }

    /// The trash asks first; Remove closes the dialog and starts the removal,
    /// which shows on the activity line until it answers.
    #[wasm_bindgen_test]
    async fn the_trash_confirms_then_removes_that_revision() {
        REMOVED.with(|r| r.borrow_mut().clear());
        let w = Wiring::new();
        let activities = crate::kit::Activities::new();
        let el = removal_pane(removable, removes_ok, w, activities);
        let surface = opened(&el).await;

        button_named(&surface, "Remove \u{201c}Old\u{201d}").click();
        leptos::task::tick().await;
        let dialog = el
            .query_selector("dialog")
            .unwrap()
            .expect("the confirmation");
        assert!(
            dialog
                .text_content()
                .unwrap_or_default()
                .contains("Remove \u{201c}Old\u{201d}? This frees 1.2 MB."),
            "markup was {}",
            dialog.inner_html()
        );
        assert!(
            REMOVED.with(|r| r.borrow().is_empty()),
            "nothing before the answer"
        );

        crate::test_support::button_saying(&dialog, "Remove").click();
        settle().await;
        settle().await;

        assert_eq!(
            REMOVED.with(|r| r.borrow().clone()),
            vec![("team/dataset".to_string(), vec!["old-1".to_string()])]
        );
        assert!(!w.removal.open.get_untracked(), "the dialog closed");
        assert_eq!(
            w.removal.running.get_untracked(),
            None,
            "and the removal ended"
        );
        assert_eq!(untrack(|| activities.first_label()), "", "the line cleared");
        assert_eq!(w.outcome.get_untracked(), None);
    }

    #[wasm_bindgen_test]
    async fn while_removing_the_line_says_so_and_every_button_refuses() {
        let w = Wiring::new();
        let activities = crate::kit::Activities::new();
        let el = removal_pane(removable, removes_never, w, activities);
        let surface = opened(&el).await;

        button_named(&surface, "Remove \u{201c}Old\u{201d}").click();
        leptos::task::tick().await;
        let dialog = el
            .query_selector("dialog")
            .unwrap()
            .expect("the confirmation");
        crate::test_support::button_saying(&dialog, "Remove").click();
        settle().await;

        assert_eq!(
            untrack(|| activities.first_label()),
            "Removing 1 old revision of team/dataset\u{2026}"
        );
        // The modal closed the popover; reopened, it lists what is there.
        let surface = reopened(&el, &surface).await;
        assert!(button_named(&surface, "Remove \u{201c}Old\u{201d}").disabled());
        assert!(button_named(&surface, "Remove \u{201c}Older\u{201d}").disabled());
    }

    #[wasm_bindgen_test]
    async fn a_refusal_reaches_the_band() {
        let w = Wiring::new();
        let el = removal_pane(removable, removes_busy, w, crate::kit::Activities::new());
        let surface = opened(&el).await;

        element_saying(&surface, "Remove 2 older \u{b7} frees 6.9 MB").click();
        leptos::task::tick().await;
        let dialog = el
            .query_selector("dialog")
            .unwrap()
            .expect("the confirmation");
        assert!(
            dialog
                .text_content()
                .unwrap_or_default()
                .contains("Remove 2 older revisions? This frees 6.9 MB.")
        );
        crate::test_support::button_saying(&dialog, "Remove").click();
        settle().await;
        settle().await;

        let outcome = w.outcome.get_untracked().expect("the band speaks");
        assert_eq!(outcome.namespace, "team/dataset");
        assert_eq!(outcome.lead, "Could not remove old revisions.");
        assert!(outcome.detail.unwrap_or_default().contains("busy"));
    }

    /// Autopull syncing this package holds its lock: the buttons refuse, and
    /// the footer says why.
    #[wasm_bindgen_test]
    async fn a_sync_of_this_package_disables_removal_and_says_why() {
        let activities = crate::kit::Activities::new();
        activities.set(
            crate::kit::ActivityKind::Autopull,
            vec![crate::kit::Activity {
                kind: crate::kit::ActivityKind::Autopull,
                label: "Getting latest for team/dataset\u{2026}".to_string(),
                package: Some("team/dataset".to_string()),
            }],
        );
        let el = removal_pane(removable, removes_ok, Wiring::new(), activities);
        let surface = opened(&el).await;

        element_saying(
            &surface,
            "team/dataset is busy syncing \u{2014} try again in a moment",
        );
        assert!(button_named(&surface, "Remove \u{201c}Old\u{201d}").disabled());
    }

    #[wasm_bindgen_test]
    async fn a_page_command_disables_removal() {
        let w = Wiring::new();
        w.busy.set(true);
        let el = removal_pane(removable, removes_ok, w, crate::kit::Activities::new());
        let surface = opened(&el).await;

        assert!(button_named(&surface, "Remove \u{201c}Old\u{201d}").disabled());
    }

    #[wasm_bindgen_test]
    async fn everything_kept_says_there_is_nothing_to_remove() {
        let el = removal_pane(
            all_kept,
            removes_ok,
            Wiring::new(),
            crate::kit::Activities::new(),
        );
        let surface = opened(&el).await;

        element_saying(
            &surface,
            "Nothing to remove \u{2014} every revision here is in use",
        );
        element_saying(&surface, "latest \u{b7} base");
        assert!(
            surface
                .query_selector("button[aria-label^='Remove']")
                .unwrap()
                .is_none()
        );
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
        // Keeping draws radios beside it, so the section is found by its heading.
        let revision = element_saying(&el, "Revision")
            .closest("section")
            .unwrap()
            .expect("the Revision section");
        let buttons = revision.query_selector_all("button").unwrap();
        assert_eq!(
            buttons.length(),
            1,
            "the trigger is the section's one control; markup was {}",
            revision.inner_html()
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
            revision.query_selector("a, input").unwrap().is_none(),
            "no link or input while closed; markup was {}",
            revision.inner_html()
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
                        w=Wiring::new()
                        commands=idle_commands()
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
