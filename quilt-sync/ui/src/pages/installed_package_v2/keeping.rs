//! The context pane's Keeping section: which files this copy keeps, and what
//! that choice means now.
//!
//! The words are pure so their rules are host-tested: the caption always
//! counts what is here, only the whole package promises files added later, and
//! only the whole package with files outstanding offers the counted download
//! (`scope-and-backlog`). Choosing a scope stores it and moves no bytes; the
//! download installs exactly the paths the page read listed.

use std::future::Future;
use std::pin::Pin;

use leptos::prelude::*;

use super::{Wiring, run};
use crate::commands::{self, KeepingScope};
use crate::kit::{Button, Choice, ChoiceGroup, PaneSection};

type Answer<T> = Pin<Box<dyn Future<Output = Result<T, String>>>>;

/// Stores a package's scope: `(namespace, entire_package)`.
pub type ScopeStore = fn(String, bool) -> Answer<()>;
/// Installs the listed backlog: `(namespace, paths)`.
pub type BacklogDownload = fn(String, Vec<String>) -> Answer<()>;

/// Keeping's two commands. Function pointers, so the DOM tests and the
/// gallery answer without a Tauri host.
#[derive(Clone, Copy)]
pub struct KeepingCommands {
    pub store: ScopeStore,
    pub download: BacklogDownload,
}

impl KeepingCommands {
    /// The app's: `package_set_sync_scope` and `package_download_backlog`.
    pub(super) fn app() -> Self {
        Self {
            store: |namespace, entire_package| {
                Box::pin(commands::package_set_sync_scope(namespace, entire_package))
            },
            download: |namespace, paths| {
                Box::pin(commands::package_download_backlog(namespace, paths))
            },
        }
    }
}

/// The value each scope has in the group.
fn choice_value(scope: KeepingScope) -> &'static str {
    match scope {
        KeepingScope::IndividualFiles => "pick",
        KeepingScope::EntirePackage => "all",
    }
}

/// Which files this copy keeps, and what that means now. The caption and the
/// download read the payload's scope, not the group's: they change only once
/// a stored choice has been re-read.
#[component]
pub(super) fn KeepingSection(
    #[prop(into)] namespace: String,
    data: commands::KeepingData,
    w: Wiring,
    commands: KeepingCommands,
) -> impl IntoView {
    let commands::KeepingData {
        scope,
        total,
        remote_only,
    } = data;
    let outstanding = remote_only.len();
    let stored = choice_value(scope).to_string();
    // Written only by the radio and the rollbacks below: any other write stores a scope.
    let selected = RwSignal::new(stored.clone());

    {
        let namespace = namespace.clone();
        Effect::new(move |_| {
            let want = selected.get();
            // The first run, and every rollback, land here.
            if want == stored {
                return;
            }
            // `run` would drop this press silently; don't leave it drawn as chosen.
            if w.busy.get_untracked() {
                selected.set(stored.clone());
                return;
            }
            let (ns, stored) = (namespace.clone(), stored.clone());
            let task = async move {
                let answer = (commands.store)(ns, want == "all").await;
                if answer.is_err() {
                    // `try_`: a section rebuilt by a re-read already draws the stored scope.
                    selected.try_set(stored);
                }
                answer.map(|()| String::new())
            };
            run(
                w.busy,
                w.outcome,
                namespace.clone(),
                "Could not change what this package keeps.",
                Some(w.reload),
                task,
            );
        });
    }

    let download = download_label(scope, outstanding).map(|label| {
        let press = move |_| {
            // The read's list, not what is outstanding by the time of the press.
            let (ns, paths) = (namespace.clone(), remote_only.clone());
            let downloading = w.downloading;
            let task = async move {
                downloading.set(true);
                let answer = (commands.download)(ns, paths).await;
                downloading.try_set(false);
                answer.map(|()| String::new())
            };
            run(
                w.busy,
                w.outcome,
                namespace.clone(),
                "Could not download the files.",
                Some(w.reload),
                task,
            );
        };
        view! {
            <Button on_click=press disabled=w.busy loading=w.downloading>
                {label}
            </Button>
        }
    });

    view! {
        <PaneSection>
            // Sealed by the page's signal, so a section rebuilt mid-command is drawn sealed.
            <ChoiceGroup
                label="Keeping"
                caption=caption(scope, total, outstanding)
                options=vec![
                    Choice::new("pick", "Files I pick"),
                    Choice::new("all", "The whole package"),
                ]
                selected=selected
                disabled=w.busy
            />
            {download}
        </PaneSection>
    }
}

/// What the choice means now: the present count, and — only under the whole
/// package — the promise about files that do not exist yet.
pub(super) fn caption(scope: KeepingScope, total: usize, outstanding: usize) -> String {
    let counted = if outstanding == 0 {
        "All files are downloaded".to_string()
    } else {
        // The read guarantees `outstanding <= total`; a wrong count beats a panic.
        let here = total.saturating_sub(outstanding);
        format!("{here} of {total} downloaded")
    };
    match scope {
        KeepingScope::EntirePackage => {
            format!("{counted} — files added later are downloaded too.")
        }
        KeepingScope::IndividualFiles => format!("{counted}."),
    }
}

/// `Download N files`, or `None`: only the whole package with a backlog offers
/// it (`scope-and-backlog`).
pub(super) fn download_label(scope: KeepingScope, outstanding: usize) -> Option<String> {
    match (scope, outstanding) {
        (KeepingScope::IndividualFiles, _) | (KeepingScope::EntirePackage, 0) => None,
        (KeepingScope::EntirePackage, 1) => Some("Download 1 file".to_string()),
        (KeepingScope::EntirePackage, n) => Some(format!("Download {n} files")),
    }
}

#[cfg(test)]
mod tests {
    use super::{Answer, KeepingCommands, KeepingSection, caption, download_label};
    use crate::commands::KeepingData;
    use crate::commands::KeepingScope::{self, EntirePackage, IndividualFiles};
    use crate::kit::BannerVariant;
    use crate::pages::installed_package_v2::{Outcome, Wiring};
    use crate::test_support::{element_saying, mount, sleep_ms};
    use leptos::prelude::*;
    use std::cell::{Cell, RefCell};
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::*;

    #[test]
    fn the_caption_counts_what_is_here() {
        assert_eq!(caption(IndividualFiles, 56, 2), "54 of 56 downloaded.");
    }

    #[test]
    fn a_complete_copy_says_so() {
        assert_eq!(caption(IndividualFiles, 56, 0), "All files are downloaded.");
    }

    /// Under individual-file scope the next revision can add a file and
    /// falsify any promise about later files (v1's
    /// `the_complete_caption_promises_nothing_about_later_files`).
    #[test]
    fn only_the_whole_package_promises_later_files() {
        assert_eq!(
            caption(EntirePackage, 56, 2),
            "54 of 56 downloaded — files added later are downloaded too."
        );
        assert_eq!(
            caption(EntirePackage, 56, 0),
            "All files are downloaded — files added later are downloaded too."
        );
        for outstanding in [2, 0] {
            let words = caption(IndividualFiles, 56, outstanding);
            for word in ["later", "will"] {
                assert!(!words.contains(word), "{words:?} should not mention {word}");
            }
        }
    }

    #[test]
    fn the_download_is_counted() {
        assert_eq!(
            download_label(EntirePackage, 1).as_deref(),
            Some("Download 1 file")
        );
        assert_eq!(
            download_label(EntirePackage, 7).as_deref(),
            Some("Download 7 files")
        );
    }

    #[test]
    fn nothing_to_download_offers_nothing() {
        assert_eq!(download_label(EntirePackage, 0), None);
    }

    #[test]
    fn files_i_pick_never_carries_the_whole_backlog() {
        assert_eq!(download_label(IndividualFiles, 7), None);
    }

    fn data(scope: KeepingScope, total: usize, outstanding: &[&str]) -> KeepingData {
        KeepingData {
            scope,
            total,
            remote_only: outstanding.iter().map(ToString::to_string).collect(),
        }
    }

    thread_local! {
        static STORED: RefCell<Vec<(String, bool)>> = const { RefCell::new(Vec::new()) };
        static DOWNLOADED: RefCell<Vec<(String, Vec<String>)>> =
            const { RefCell::new(Vec::new()) };
        static RELOADS: Cell<u32> = const { Cell::new(0) };
    }

    /// Forget what earlier tests recorded: the cells outlive each test.
    fn clear() {
        STORED.with_borrow_mut(Vec::clear);
        DOWNLOADED.with_borrow_mut(Vec::clear);
        RELOADS.set(0);
    }

    fn stored() -> Vec<(String, bool)> {
        STORED.with_borrow(Clone::clone)
    }

    fn downloaded() -> Vec<(String, Vec<String>)> {
        DOWNLOADED.with_borrow(Clone::clone)
    }

    fn stores_ok(namespace: String, entire: bool) -> Answer<()> {
        STORED.with_borrow_mut(|calls| calls.push((namespace, entire)));
        Box::pin(async { Ok(()) })
    }

    fn refuses_store(namespace: String, entire: bool) -> Answer<()> {
        STORED.with_borrow_mut(|calls| calls.push((namespace, entire)));
        Box::pin(async { Err("AccessDenied".into()) })
    }

    fn stores_never(namespace: String, entire: bool) -> Answer<()> {
        STORED.with_borrow_mut(|calls| calls.push((namespace, entire)));
        Box::pin(std::future::pending())
    }

    fn downloads_ok(namespace: String, paths: Vec<String>) -> Answer<()> {
        DOWNLOADED.with_borrow_mut(|calls| calls.push((namespace, paths)));
        Box::pin(async { Ok(()) })
    }

    fn refuses_download(namespace: String, paths: Vec<String>) -> Answer<()> {
        DOWNLOADED.with_borrow_mut(|calls| calls.push((namespace, paths)));
        Box::pin(async { Err("AccessDenied".into()) })
    }

    fn download_never(namespace: String, paths: Vec<String>) -> Answer<()> {
        DOWNLOADED.with_borrow_mut(|calls| calls.push((namespace, paths)));
        Box::pin(std::future::pending())
    }

    fn idle_commands() -> KeepingCommands {
        KeepingCommands {
            store: stores_ok,
            download: downloads_ok,
        }
    }

    fn section(data: KeepingData, w: Wiring) -> web_sys::Element {
        pressable(data, w, idle_commands())
    }

    /// The section answering from `commands`, with `RELOADS` counting each
    /// re-read it asks for.
    fn pressable(data: KeepingData, w: Wiring, commands: KeepingCommands) -> web_sys::Element {
        mount(move || {
            Effect::new(move |seen: Option<()>| {
                w.reload.track();
                // The first run is the subscription, not a re-read.
                if seen.is_some() {
                    RELOADS.set(RELOADS.get() + 1);
                }
            });
            view! {
                <KeepingSection
                    namespace="team/dataset"
                    data=data
                    w=w
                    commands=commands
                />
            }
        })
    }

    /// A task boundary and a tick: the command settles on the event loop, then the DOM.
    async fn settle() {
        sleep_ms(0).await;
        leptos::task::tick().await;
    }

    fn a_backlog_in(scope: KeepingScope) -> KeepingData {
        data(scope, 56, &["plate/b.csv", "plate/c.csv"])
    }

    /// The group, found by the words its `aria-labelledby` points at.
    fn group(el: &web_sys::Element) -> web_sys::Element {
        let group = el
            .query_selector("[role=radiogroup]")
            .unwrap()
            .expect("a radiogroup");
        let id = group
            .get_attribute("aria-labelledby")
            .expect("the group names its label");
        let label = web_sys::window()
            .unwrap()
            .document()
            .unwrap()
            .get_element_by_id(&id)
            .expect("an element with that id");
        assert_eq!(label.text_content().unwrap_or_default().trim(), "Keeping");
        group
    }

    fn radio(el: &web_sys::Element, words: &str) -> web_sys::HtmlInputElement {
        element_saying(el, words)
            .closest("label")
            .unwrap()
            .expect("the option's label")
            .query_selector("input[type=radio]")
            .unwrap()
            .expect("the option's radio")
            .unchecked_into()
    }

    fn no_button(el: &web_sys::Element) {
        assert!(
            el.query_selector("button").unwrap().is_none(),
            "no download; markup was {}",
            el.inner_html()
        );
    }

    #[wasm_bindgen_test]
    fn files_i_pick_with_a_backlog_counts_it_and_offers_no_download() {
        let el = section(
            data(IndividualFiles, 56, &["plate/b.csv", "plate/c.csv"]),
            Wiring::new(),
        );
        let group = group(&el);
        assert!(radio(&group, "Files I pick").checked());
        assert!(!radio(&group, "The whole package").checked());
        element_saying(&el, "54 of 56 downloaded.");
        no_button(&el);
    }

    #[wasm_bindgen_test]
    fn the_whole_package_with_a_backlog_offers_the_counted_download() {
        let el = section(
            data(EntirePackage, 56, &["plate/b.csv", "plate/c.csv"]),
            Wiring::new(),
        );
        let group = group(&el);
        assert!(radio(&group, "The whole package").checked());
        assert!(!radio(&group, "Files I pick").checked());
        element_saying(
            &el,
            "54 of 56 downloaded — files added later are downloaded too.",
        );
        let button = element_saying(&el, "Download 2 files")
            .closest("button")
            .unwrap();
        assert!(button.is_some(), "the count is a button");
    }

    #[wasm_bindgen_test]
    fn the_whole_package_with_nothing_outstanding_offers_nothing() {
        let el = section(data(EntirePackage, 56, &[]), Wiring::new());
        group(&el);
        element_saying(
            &el,
            "All files are downloaded — files added later are downloaded too.",
        );
        no_button(&el);
    }

    #[wasm_bindgen_test]
    async fn while_another_command_runs_the_choice_and_download_are_sealed() {
        let w = Wiring::new();
        let el = section(data(EntirePackage, 56, &["plate/b.csv", "plate/c.csv"]), w);
        w.busy.set(true);
        leptos::task::tick().await;

        for words in ["Files I pick", "The whole package"] {
            assert!(radio(&el, words).disabled(), "{words} is sealed");
        }
        let button = el
            .query_selector("button[disabled]")
            .unwrap()
            .expect("the download is sealed");
        assert_eq!(
            button.get_attribute("aria-busy").as_deref(),
            Some("false"),
            "sealed by another command, not running itself"
        );
    }

    #[wasm_bindgen_test]
    async fn a_running_download_shows_on_its_button() {
        let w = Wiring::new();
        let el = section(data(EntirePackage, 56, &["plate/b.csv", "plate/c.csv"]), w);
        w.busy.set(true);
        w.downloading.set(true);
        leptos::task::tick().await;

        let button = el.query_selector("button").unwrap().expect("the download");
        assert_eq!(button.get_attribute("aria-busy").as_deref(), Some("true"));
    }

    fn commands(store: super::ScopeStore, download: super::BacklogDownload) -> KeepingCommands {
        KeepingCommands { store, download }
    }

    fn download_button(el: &web_sys::Element) -> web_sys::HtmlElement {
        element_saying(el, "Download 2 files")
            .closest("button")
            .unwrap()
            .expect("the count is a button")
            .unchecked_into()
    }

    #[wasm_bindgen_test]
    async fn choosing_the_whole_package_stores_it_and_re_reads() {
        clear();
        let w = Wiring::new();
        let el = pressable(
            a_backlog_in(IndividualFiles),
            w,
            commands(stores_ok, downloads_ok),
        );
        leptos::task::tick().await;

        element_saying(&el, "The whole package").click();
        settle().await;

        assert_eq!(stored(), vec![("team/dataset".to_string(), true)]);
        assert_eq!(RELOADS.get(), 1, "a stored scope re-reads");
        // The caption is the payload's, so it waits for the re-read.
        element_saying(&el, "54 of 56 downloaded.");
        assert!(downloaded().is_empty(), "storing a scope moves no bytes");
    }

    #[wasm_bindgen_test]
    async fn the_group_is_sealed_while_the_store_runs() {
        clear();
        let w = Wiring::new();
        let el = pressable(
            a_backlog_in(IndividualFiles),
            w,
            commands(stores_never, downloads_ok),
        );
        leptos::task::tick().await;

        element_saying(&el, "The whole package").click();
        // The store starts from an `Effect`, so `busy` reaches the DOM a tick later.
        leptos::task::tick().await;
        leptos::task::tick().await;

        assert_eq!(stored().len(), 1, "the store is running");
        assert!(w.busy.get_untracked(), "the store holds the page");
        for words in ["Files I pick", "The whole package"] {
            assert!(radio(&el, words).disabled(), "{words} is sealed");
        }
    }

    #[wasm_bindgen_test]
    async fn a_refused_store_restores_the_prior_choice_and_tells_the_band() {
        clear();
        let w = Wiring::new();
        let el = pressable(
            a_backlog_in(IndividualFiles),
            w,
            commands(refuses_store, downloads_ok),
        );
        leptos::task::tick().await;

        element_saying(&el, "The whole package").click();
        settle().await;
        settle().await;

        assert!(
            radio(&el, "Files I pick").checked(),
            "the prior choice is back"
        );
        assert!(!radio(&el, "The whole package").checked());
        assert_eq!(stored().len(), 1, "the rollback stored nothing");
        assert_eq!(
            w.outcome.get_untracked(),
            Some(Outcome {
                namespace: "team/dataset".to_string(),
                variant: BannerVariant::Critical,
                lead: "Could not change what this package keeps.".to_string(),
                detail: Some("AccessDenied".to_string()),
            })
        );
        assert_eq!(RELOADS.get(), 0, "a refusal does not re-read");
    }

    #[wasm_bindgen_test]
    async fn the_download_installs_exactly_the_listed_paths_and_re_reads() {
        clear();
        let w = Wiring::new();
        let el = pressable(
            a_backlog_in(EntirePackage),
            w,
            commands(stores_ok, downloads_ok),
        );
        leptos::task::tick().await;

        download_button(&el).click();
        settle().await;

        assert_eq!(
            downloaded(),
            vec![(
                "team/dataset".to_string(),
                vec!["plate/b.csv".to_string(), "plate/c.csv".to_string()]
            )]
        );
        assert_eq!(RELOADS.get(), 1, "a download re-reads");
        assert!(!w.downloading.get_untracked());
        assert!(stored().is_empty(), "a download stores no scope");
        assert_eq!(w.outcome.get_untracked(), None, "success says nothing");
    }

    #[wasm_bindgen_test]
    async fn a_running_download_holds_the_page_and_its_spinner() {
        clear();
        let w = Wiring::new();
        let el = pressable(
            a_backlog_in(EntirePackage),
            w,
            commands(stores_ok, download_never),
        );
        leptos::task::tick().await;

        download_button(&el).click();
        leptos::task::tick().await;

        assert_eq!(downloaded().len(), 1, "the download is running");
        assert!(w.busy.get_untracked(), "the download holds the page");
        assert!(w.downloading.get_untracked(), "and says it is the download");
        assert_eq!(
            download_button(&el).get_attribute("aria-busy").as_deref(),
            Some("true")
        );
        for words in ["Files I pick", "The whole package"] {
            assert!(radio(&el, words).disabled(), "{words} is sealed");
        }
    }

    #[wasm_bindgen_test]
    async fn a_refused_download_tells_the_band_and_leaves_the_count() {
        clear();
        let w = Wiring::new();
        let el = pressable(
            a_backlog_in(EntirePackage),
            w,
            commands(stores_ok, refuses_download),
        );
        leptos::task::tick().await;

        download_button(&el).click();
        settle().await;

        assert_eq!(downloaded().len(), 1, "the download was asked for");
        let outcome = w.outcome.get_untracked().expect("the band is told");
        assert_eq!(outcome.namespace, "team/dataset");
        assert_eq!(outcome.lead, "Could not download the files.");
        assert_eq!(outcome.detail.as_deref(), Some("AccessDenied"));
        download_button(&el);
        assert!(!w.downloading.get_untracked());
        assert_eq!(RELOADS.get(), 0, "a refusal does not re-read");
    }

    /// Mounted busy, as a section rebuilt mid-command is: the radio is
    /// disabled, and a `change` dispatched at it anyway must store nothing.
    #[wasm_bindgen_test]
    async fn a_press_while_busy_stores_nothing() {
        clear();
        let w = Wiring::new();
        w.busy.set(true);
        let el = pressable(
            a_backlog_in(IndividualFiles),
            w,
            commands(stores_ok, downloads_ok),
        );
        leptos::task::tick().await;

        let whole = radio(&el, "The whole package");
        assert!(whole.disabled());
        whole
            .dispatch_event(&web_sys::Event::new("change").unwrap())
            .unwrap();
        settle().await;

        assert!(stored().is_empty(), "nothing stored while busy");
        assert!(
            radio(&el, "Files I pick").checked(),
            "the stored choice is drawn"
        );
        assert!(!radio(&el, "The whole package").checked());
    }

    /// The guard itself: the press lands while the page is idle, and another
    /// command takes the page before the store's `Effect` runs.
    #[wasm_bindgen_test]
    async fn a_press_overtaken_by_another_command_is_rolled_back() {
        clear();
        let w = Wiring::new();
        let el = pressable(
            a_backlog_in(IndividualFiles),
            w,
            commands(stores_ok, downloads_ok),
        );
        leptos::task::tick().await;

        element_saying(&el, "The whole package").click();
        // Synchronously after the click: the `Effect` has not run yet.
        w.busy.set(true);
        settle().await;

        assert!(stored().is_empty(), "the dropped press stored nothing");
        assert!(
            radio(&el, "Files I pick").checked(),
            "and shows no unstored choice"
        );
        assert!(!radio(&el, "The whole package").checked());
    }
}
