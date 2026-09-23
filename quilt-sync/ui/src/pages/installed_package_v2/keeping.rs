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

use super::Wiring;
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
    // Pressed by the store and the download, which arrive with their tests.
    let _ = (namespace, commands);
    let commands::KeepingData {
        scope,
        total,
        remote_only,
    } = data;
    let outstanding = remote_only.len();
    let selected = RwSignal::new(choice_value(scope).to_string());
    let download = download_label(scope, outstanding).map(|label| {
        view! {
            <Button on_click=|_| () disabled=w.busy loading=w.downloading>
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
    use crate::pages::installed_package_v2::Wiring;
    use crate::test_support::{element_saying, mount};
    use leptos::prelude::*;
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

    fn stores_ok(_: String, _: bool) -> Answer<()> {
        Box::pin(async { Ok(()) })
    }

    fn downloads_ok(_: String, _: Vec<String>) -> Answer<()> {
        Box::pin(async { Ok(()) })
    }

    fn idle_commands() -> KeepingCommands {
        KeepingCommands {
            store: stores_ok,
            download: downloads_ok,
        }
    }

    fn section(data: KeepingData, w: Wiring) -> web_sys::Element {
        mount(move || {
            view! {
                <KeepingSection
                    namespace="team/dataset"
                    data=data
                    w=w
                    commands=idle_commands()
                />
            }
        })
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
}
