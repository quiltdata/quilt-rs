//! The context pane's Keeping section: which files this copy keeps, and what
//! that choice means now.
//!
//! The words are pure so their rules are host-tested: the caption always
//! counts what is here, only the whole package promises files added later, and
//! only the whole package with files outstanding offers the counted download
//! (`scope-and-backlog`). Choosing a scope stores it and moves no bytes; the
//! download installs exactly the paths the page read listed.
//!
//! The size joins the count: the count says how much of the package is here,
//! and the size says it in bytes. A file deleted here counts as downloaded — it
//! is a local change waiting to commit, and Download would not fetch it — so
//! the caption names those files rather than leave the count to explain them.

use std::future::Future;
use std::pin::Pin;

use leptos::prelude::*;

use super::{Wiring, run};
use crate::commands::{self, KeepingScope, PackageSize};
use crate::kit::{Button, Choice, ChoiceGroup, PaneSection};
use crate::util::{format_size, thousands};

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
            // The skipped paths are the file pane's to show; Keeping's
            // download reports only whether it ran.
            download: |namespace, paths| {
                Box::pin(async move {
                    commands::package_download_backlog(namespace, paths)
                        .await
                        .map(|_skipped| ())
                })
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
    /// The revision's bytes, for the caption and the download; `None` leaves
    /// them to the counts.
    size: Option<PackageSize>,
    w: Wiring,
    commands: KeepingCommands,
) -> impl IntoView {
    let commands::KeepingData {
        scope,
        total,
        remote_only,
        deleted_here,
    } = data;
    let outstanding = remote_only.len();
    // What the backend holds: the payload's scope until a store succeeds, since
    // the group is live again before that store's re-read lands.
    let stored = StoredValue::new(choice_value(scope).to_string());
    // Written only by the radio and the rollbacks below: any other write stores a scope.
    let selected = RwSignal::new(stored.get_value());

    {
        let namespace = namespace.clone();
        Effect::new(move |_| {
            let want = selected.get();
            let prior = stored.get_value();
            // The first run, and every rollback, land here.
            if want == prior {
                return;
            }
            // `run` would drop this press silently; don't leave it drawn as chosen.
            if w.busy.get_untracked() {
                selected.set(prior);
                return;
            }
            let ns = namespace.clone();
            let task = async move {
                let answer = (commands.store)(ns, want == "all").await;
                // `try_`: a section rebuilt by a re-read already draws the stored scope.
                if answer.is_ok() {
                    stored.try_set_value(want);
                } else {
                    selected.try_set(prior);
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

    let download = download_label(scope, outstanding, size).map(|label| {
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
                caption=caption(scope, Held { total, outstanding, deleted_here, size })
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

/// What the caption counts: the revision's files, those not here, those
/// deleted here, and their bytes when they could be read.
#[derive(Clone, Copy)]
pub(super) struct Held {
    pub total: usize,
    pub outstanding: usize,
    pub deleted_here: usize,
    pub size: Option<PackageSize>,
}

/// What the choice means now: the present count, and — only under the whole
/// package — the promise about files that do not exist yet.
pub(super) fn caption(scope: KeepingScope, held: Held) -> String {
    let counted = counted(held);
    match scope {
        KeepingScope::EntirePackage => {
            format!("{counted} — files added later are downloaded too.")
        }
        KeepingScope::IndividualFiles => format!("{counted}."),
    }
}

/// The count, with the size joined to it, and the files deleted here named.
/// An empty revision says so: "All files are downloaded" is true of nothing.
/// A size that cannot be read says nothing, not a dash: the count is still
/// true.
fn counted(held: Held) -> String {
    let Held {
        total,
        outstanding,
        deleted_here,
        size,
    } = held;
    if total == 0 {
        return "This revision has no files".to_string();
    }
    // The read guarantees `outstanding <= total`; a wrong count beats a panic.
    let here = total.saturating_sub(outstanding);
    let counted = match size {
        Some(size) if outstanding == 0 => {
            format!("All files are downloaded · {}", format_size(size.total))
        }
        Some(size) if here == 0 => {
            format!("No files are downloaded · {}", format_size(size.total))
        }
        Some(size) => format!(
            "{} of {} files · {} of {} downloaded",
            thousands(here),
            thousands(total),
            format_size(size.downloaded),
            format_size(size.total),
        ),
        None if outstanding == 0 => "All files are downloaded".to_string(),
        None if here == 0 => "No files are downloaded".to_string(),
        None => format!("{} of {} downloaded", thousands(here), thousands(total)),
    };
    with_deleted_here(counted, deleted_here)
}

/// `words · 3 deleted here`, or `words` alone with nothing deleted here. One
/// suffix for this caption and the file pane's, so the two say it alike.
pub(crate) fn with_deleted_here(words: String, deleted_here: usize) -> String {
    if deleted_here == 0 {
        words
    } else {
        format!("{words} · {} deleted here", thousands(deleted_here))
    }
}

/// `Download N files · 1.5 MB`, or `None`: only the whole package with a
/// backlog offers it (`scope-and-backlog`). The bytes are the backlog's, total
/// less downloaded, so a size that cannot be read leaves the count alone.
pub(super) fn download_label(
    scope: KeepingScope,
    outstanding: usize,
    size: Option<PackageSize>,
) -> Option<String> {
    let files = match (scope, outstanding) {
        (KeepingScope::IndividualFiles, _) | (KeepingScope::EntirePackage, 0) => return None,
        (KeepingScope::EntirePackage, 1) => "Download 1 file".to_string(),
        (KeepingScope::EntirePackage, n) => format!("Download {} files", thousands(n)),
    };
    Some(match size {
        Some(size) => format!(
            "{files} · {}",
            format_size(size.total.saturating_sub(size.downloaded))
        ),
        None => files,
    })
}

#[cfg(test)]
mod tests {
    use super::{Answer, Held, KeepingCommands, KeepingSection, caption, download_label};
    use crate::commands::KeepingScope::{self, EntirePackage, IndividualFiles};
    use crate::commands::{KeepingData, PackageSize};
    use crate::kit::BannerVariant;
    use crate::pages::installed_package_v2::{Outcome, Wiring};
    use crate::test_support::{element_saying, mount, sleep_ms};
    use leptos::prelude::*;
    use std::cell::{Cell, RefCell};
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::*;

    const MB: u64 = 1_000_000;

    /// 56 files and 3.4 MB, of which 1.9 MB is here when two files are not.
    fn plate(outstanding: usize) -> Held {
        Held {
            total: 56,
            outstanding,
            deleted_here: 0,
            size: Some(PackageSize {
                total: 34 * MB / 10,
                downloaded: if outstanding == 0 {
                    34 * MB / 10
                } else {
                    19 * MB / 10
                },
            }),
        }
    }

    /// [`plate`] whose rows' sizes could not be read.
    fn unread(outstanding: usize) -> Held {
        Held {
            size: None,
            ..plate(outstanding)
        }
    }

    /// The words as read, with the no-break spaces shown as spaces.
    fn plain(words: &str) -> String {
        words.replace('\u{a0}', " ")
    }

    #[test]
    fn the_caption_counts_what_is_here_with_its_size() {
        assert_eq!(
            plain(&caption(IndividualFiles, plate(2))),
            "54 of 56 files · 1.9 MB of 3.4 MB downloaded."
        );
    }

    #[test]
    fn a_complete_copy_says_so_and_its_size() {
        assert_eq!(
            plain(&caption(IndividualFiles, plate(0))),
            "All files are downloaded · 3.4 MB."
        );
    }

    #[test]
    fn a_copy_with_no_file_here_says_so() {
        assert_eq!(
            plain(&caption(IndividualFiles, plate(56))),
            "No files are downloaded · 3.4 MB."
        );
        assert_eq!(
            caption(IndividualFiles, unread(56)),
            "No files are downloaded."
        );
    }

    /// Not "All files are downloaded", which is true of nothing.
    #[test]
    fn an_empty_revision_says_it_has_no_files() {
        let empty = Held {
            total: 0,
            outstanding: 0,
            deleted_here: 0,
            size: Some(PackageSize {
                total: 0,
                downloaded: 0,
            }),
        };
        assert_eq!(
            caption(IndividualFiles, empty),
            "This revision has no files."
        );
        assert_eq!(
            caption(
                IndividualFiles,
                Held {
                    size: None,
                    ..empty
                }
            ),
            "This revision has no files."
        );
    }

    #[test]
    fn a_huge_package_groups_its_counts() {
        let huge = Held {
            total: 140_000,
            outstanding: 127_600,
            deleted_here: 0,
            size: Some(PackageSize {
                total: 1_200_000 * MB,
                downloaded: 86_300 * MB,
            }),
        };
        assert_eq!(
            plain(&caption(IndividualFiles, huge)),
            "12,400 of 140,000 files · 86.3 GB of 1.2 TB downloaded."
        );
        assert_eq!(
            caption(IndividualFiles, Held { size: None, ..huge }),
            "12,400 of 140,000 downloaded."
        );
    }

    /// A size that cannot be read leaves the count alone.
    #[test]
    fn an_unread_size_leaves_the_count() {
        assert_eq!(caption(IndividualFiles, unread(2)), "54 of 56 downloaded.");
        assert_eq!(
            caption(IndividualFiles, unread(0)),
            "All files are downloaded."
        );
    }

    /// A file deleted here counts as downloaded, so the caption names it.
    #[test]
    fn files_deleted_here_are_named() {
        let deleted = |held: Held, n| Held {
            deleted_here: n,
            ..held
        };
        assert_eq!(
            plain(&caption(IndividualFiles, deleted(plate(2), 1))),
            "54 of 56 files · 1.9 MB of 3.4 MB downloaded · 1 deleted here."
        );
        assert_eq!(
            plain(&caption(EntirePackage, deleted(plate(0), 1_200))),
            "All files are downloaded · 3.4 MB · 1,200 deleted here — files added later are \
             downloaded too."
        );
        assert_eq!(
            caption(IndividualFiles, deleted(unread(2), 1)),
            "54 of 56 downloaded · 1 deleted here."
        );
    }

    /// Under individual-file scope the next revision can add a file and
    /// falsify any promise about later files (v1's
    /// `the_complete_caption_promises_nothing_about_later_files`).
    #[test]
    fn only_the_whole_package_promises_later_files() {
        assert_eq!(
            plain(&caption(EntirePackage, plate(2))),
            "54 of 56 files · 1.9 MB of 3.4 MB downloaded — files added later are downloaded too."
        );
        assert_eq!(
            plain(&caption(EntirePackage, plate(0))),
            "All files are downloaded · 3.4 MB — files added later are downloaded too."
        );
        for outstanding in [2, 0] {
            let words = caption(IndividualFiles, plate(outstanding));
            for word in ["later", "will"] {
                assert!(!words.contains(word), "{words:?} should not mention {word}");
            }
        }
    }

    #[test]
    fn the_download_is_counted_in_files_and_bytes() {
        let size = plate(2).size;
        assert_eq!(
            download_label(EntirePackage, 2, size)
                .map(|w| plain(&w))
                .as_deref(),
            Some("Download 2 files · 1.5 MB")
        );
        assert_eq!(
            download_label(EntirePackage, 1, size)
                .map(|w| plain(&w))
                .as_deref(),
            Some("Download 1 file · 1.5 MB")
        );
        assert_eq!(
            download_label(EntirePackage, 127_600, None).as_deref(),
            Some("Download 127,600 files")
        );
    }

    #[test]
    fn an_unread_size_leaves_the_download_counted_in_files() {
        assert_eq!(
            download_label(EntirePackage, 1, None).as_deref(),
            Some("Download 1 file")
        );
        assert_eq!(
            download_label(EntirePackage, 7, None).as_deref(),
            Some("Download 7 files")
        );
    }

    #[test]
    fn nothing_to_download_offers_nothing() {
        assert_eq!(download_label(EntirePackage, 0, plate(0).size), None);
    }

    #[test]
    fn files_i_pick_never_carries_the_whole_backlog() {
        assert_eq!(download_label(IndividualFiles, 7, plate(2).size), None);
    }

    fn data(scope: KeepingScope, total: usize, outstanding: &[&str]) -> KeepingData {
        KeepingData {
            scope,
            total,
            remote_only: outstanding.iter().map(ToString::to_string).collect(),
            deleted_here: 0,
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
        mounted(data, None, w, commands)
    }

    /// [`pressable`], with the page read's size.
    fn mounted(
        data: KeepingData,
        size: Option<PackageSize>,
        w: Wiring,
        commands: KeepingCommands,
    ) -> web_sys::Element {
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
                    size=size
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

    /// The page read's size reaches the caption and the button, and a file
    /// deleted here is named.
    #[wasm_bindgen_test]
    fn the_size_joins_the_count_and_the_download() {
        let el = mounted(
            KeepingData {
                deleted_here: 1,
                ..data(EntirePackage, 56, &["plate/b.csv", "plate/c.csv"])
            },
            plate(2).size,
            Wiring::new(),
            idle_commands(),
        );
        element_saying(
            &el,
            "54 of 56 files · 1.9\u{a0}MB of 3.4\u{a0}MB downloaded · 1 deleted here — files \
             added later are downloaded too.",
        );
        let button = element_saying(&el, "Download 2 files · 1.5\u{a0}MB")
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

    /// The group is live again before the re-read lands, so a press back to
    /// the prior choice is a change from what is now stored, not a no-op.
    #[wasm_bindgen_test]
    async fn changing_back_before_the_re_read_stores_again() {
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
        element_saying(&el, "Files I pick").click();
        settle().await;

        assert_eq!(
            stored(),
            vec![
                ("team/dataset".to_string(), true),
                ("team/dataset".to_string(), false),
            ]
        );
        assert_eq!(RELOADS.get(), 2, "each stored scope re-reads");
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
