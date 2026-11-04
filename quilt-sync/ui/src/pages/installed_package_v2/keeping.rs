//! The context pane's Keeping section: which files this copy keeps, and what
//! that choice means now.
//!
//! The words are pure so their rules are host-tested: the caption always
//! counts what is here, only the whole package promises files added later, and
//! only the whole package or kept folders with files outstanding offer the
//! counted download (`scope-and-backlog`). Choosing a scope stores it and
//! moves no bytes; the download installs exactly the paths the page read
//! listed.
//!
//! Folders lists the kept folders read-only and changes them in a dialog.
//! Choosing Folders with none stored opens the dialog, and closing it unsaved
//! returns the radio to the stored choice. Saving stores and downloads nothing;
//! choosing another option replaces the folders.
//!
//! The size joins the count: the count says how much of the package is here,
//! and the size says it in bytes. A file deleted here counts as downloaded — it
//! is a local change waiting to commit, and Download would not fetch it — so
//! the caption names those files rather than leave the count to explain them.

use std::future::Future;
use std::pin::Pin;

use leptos::prelude::*;

use super::{Reread, Wiring, run};
use crate::commands::{self, KeepingScope, NotDownloaded, PackageSize};
use crate::kit::folder_list::filled;
use crate::kit::{
    Button, ButtonVariant, Choice, ChoiceGroup, FolderList, FormDialog, PaneSection, Submit,
};
use crate::util::{Held, counted, format_size, thousands};

type Answer<T> = Pin<Box<dyn Future<Output = Result<T, String>>>>;

/// Stores a package's scope: `(namespace, scope, folders)`; the folders are
/// read only for a folders scope.
pub type ScopeStore = fn(String, KeepingScope, Vec<String>) -> Answer<()>;
/// Installs the listed backlog: `(namespace, paths)`, answering the paths it left out.
pub type BacklogDownload = fn(String, Vec<String>) -> Answer<NotDownloaded>;

/// Keeping's two commands. Function pointers, so the DOM tests and the
/// gallery answer without an engine.
#[derive(Clone, Copy)]
pub struct KeepingCommands {
    pub store: ScopeStore,
    pub download: BacklogDownload,
}

impl KeepingCommands {
    /// The app's: `package_set_sync_scope` and `package_download_backlog`.
    pub(super) fn app() -> Self {
        Self {
            store: |namespace, scope, folders| {
                Box::pin(commands::package_set_sync_scope(namespace, scope, folders))
            },
            download: |namespace, paths| {
                Box::pin(commands::package_download_backlog(namespace, paths))
            },
        }
    }
}

/// The folders dialog's open flag and typed rows. The page provides them, so
/// a re-read that rebuilds the section keeps both; a section drawn alone has
/// its own.
#[derive(Clone, Copy)]
pub struct FoldersDialog {
    pub open: RwSignal<bool>,
    pub rows: RwSignal<Vec<String>>,
}

impl FoldersDialog {
    pub(super) fn new() -> Self {
        Self {
            open: RwSignal::new(false),
            rows: RwSignal::new(Vec::new()),
        }
    }

    /// Shut, with nothing typed: the page has moved to another package.
    pub(super) fn close(self) {
        self.open.set(false);
        self.rows.set(Vec::new());
    }
}

/// The value each scope has in the group.
fn choice_value(scope: KeepingScope) -> &'static str {
    match scope {
        KeepingScope::IndividualFiles => "pick",
        KeepingScope::EntirePackage => "all",
        KeepingScope::Folders => "folders",
    }
}

/// The scope a radio stores.
fn chosen_scope(value: &str) -> KeepingScope {
    match value {
        "all" => KeepingScope::EntirePackage,
        "folders" => KeepingScope::Folders,
        _ => KeepingScope::IndividualFiles,
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
        folders,
        total,
        outstanding,
        remote_only,
        deleted_here,
    } = data;
    // What the backend holds: the payload's scope until a store succeeds, since
    // the group is live again before that store's re-read lands.
    let stored = StoredValue::new(choice_value(scope).to_string());
    // Written only by the radio and the rollbacks below: any other write stores a scope.
    let selected = RwSignal::new(stored.get_value());
    let FoldersDialog {
        open: editing,
        rows,
    } = use_context::<FoldersDialog>().unwrap_or_else(FoldersDialog::new);

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
            // Folders are stored by the dialog's Save, never by the radio.
            if want == "folders" {
                rows.set(Vec::new());
                editing.set(true);
                return;
            }
            let ns = namespace.clone();
            let task = async move {
                let answer = (commands.store)(ns, chosen_scope(&want), Vec::new()).await;
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
                Reread::OnSuccess(w.reload),
                task,
            );
        });
    }

    let dialog = folders_dialog(
        namespace.clone(),
        rows,
        editing,
        selected,
        stored,
        w,
        commands,
    );
    let download = download_label(scope, remote_only.len(), size)
        .map(|label| download_button(label, namespace, remote_only, w, commands));

    let kept = (scope == KeepingScope::Folders).then(|| kept_list(folders, rows, editing, w.busy));

    view! {
        <PaneSection>
            // Sealed by the page's signal, so a section rebuilt mid-command is drawn sealed.
            <ChoiceGroup
                label="Keeping"
                caption=caption(scope, Held { total, outstanding, deleted_here, size })
                options=vec![
                    Choice::new("pick", "Files I pick"),
                    Choice::new("all", "The whole package"),
                    Choice::new("folders", "Folders"),
                ]
                selected=selected
                disabled=w.busy
            >
                {kept}
            </ChoiceGroup>
            {download}
        </PaneSection>
        // Beside the section, not in it: the section's children must be `Send`
        // and the dialog's action is not.
        {dialog}
    }
}

/// *Download N files*: installs the paths the page read listed.
fn download_button(
    label: String,
    namespace: String,
    remote_only: Vec<String>,
    w: Wiring,
    commands: KeepingCommands,
) -> impl IntoView {
    let press = move |_| {
        // The read's list, not what is outstanding by the time of the press.
        let (ns, paths) = (namespace.clone(), remote_only.clone());
        let (downloading, outcome) = (w.downloading, w.outcome);
        let task = async move {
            let asked = paths.len();
            downloading.set(true);
            let answer = (commands.download)(ns.clone(), paths).await;
            downloading.try_set(false);
            answer.map(|left_out| {
                // The file pane's warning, so both downloads say it alike.
                if let Some(said) = super::download_outcome(ns, asked, &left_out) {
                    outcome.try_set(Some(said));
                }
                String::new()
            })
        };
        run(
            w.busy,
            w.outcome,
            namespace.clone(),
            "Could not download the files.",
            Reread::OnSuccess(w.reload),
            task,
        );
    };
    view! {
        <Button
            variant=ButtonVariant::Primary
            on_click=press
            disabled=w.busy
            loading=w.downloading
        >
            {label}
        </Button>
    }
}

/// The dialog that types the kept folders. Closed unsaved, it returns the
/// radio to the stored choice.
fn folders_dialog(
    namespace: String,
    rows: RwSignal<Vec<String>>,
    editing: RwSignal<bool>,
    selected: RwSignal<String>,
    stored: StoredValue<String>,
    w: Wiring,
    commands: KeepingCommands,
) -> impl IntoView {
    Effect::new(move |_| {
        if !editing.get() && selected.get_untracked() != stored.get_value() {
            selected.set(stored.get_value());
        }
    });
    // Drawn only while open, so a closed one leaves nothing in the pane.
    move || {
        editing.get().then(|| {
            view! {
                <FormDialog
                    open=editing
                    title="Folders to keep"
                    submit=save_folders(namespace.clone(), rows, w, commands)
                    running=w.busy
                    blocked=Signal::derive(move || rows.with(|rows| filled(rows).is_empty()))
                >
                    <FolderList rows=rows />
                </FormDialog>
            }
        })
    }
}

/// The dialog's Save: stores the filled rows as kept folders and re-reads.
/// A refusal is the dialog's to show, beside the rows to fix.
fn save_folders(
    namespace: String,
    rows: RwSignal<Vec<String>>,
    w: Wiring,
    commands: KeepingCommands,
) -> Submit {
    Submit::new("Save", move || {
        let ns = namespace.clone();
        async move {
            let folders = filled(&rows.get_untracked());
            super::holding(
                w.busy,
                w.outcome,
                (commands.store)(ns, KeepingScope::Folders, folders),
            )
            .await?;
            // The re-read says what was stored: `.` is the whole package.
            w.reload.notify();
            Ok(())
        }
    })
}

/// The kept folders under their option, and *Change folders*, which opens
/// the dialog on them.
fn kept_list(
    folders: Vec<String>,
    rows: RwSignal<Vec<String>>,
    editing: RwSignal<bool>,
    busy: RwSignal<bool>,
) -> impl IntoView {
    let open = {
        let folders = folders.clone();
        move |_| {
            rows.set(folders.clone());
            editing.set(true);
        }
    };
    view! {
        // Indented to the options' words, so the list reads as Folders' own.
        <div style="display:flex; flex-direction:column; align-items:flex-start; \
                    gap:var(--q-space-2); padding-left:calc(17px + var(--q-space-2))">
            <ul style="margin:0; padding-left:1.25em; list-style:disc; \
                       font-size:var(--q-text-body); line-height:var(--q-line-body)">
                {folders.into_iter().map(|folder| view! { <li>{folder}</li> }).collect_view()}
            </ul>
            <Button on_click=open disabled=busy>"Change folders"</Button>
        </div>
    }
}

/// What the choice means now: the present count, and — only under the whole
/// package — the promise about files that do not exist yet. Kept folders are
/// listed above it, so it does not name them.
pub(super) fn caption(scope: KeepingScope, held: Held) -> String {
    let counted = counted(held);
    match scope {
        KeepingScope::EntirePackage => {
            format!("{counted} — files added later are downloaded too.")
        }
        KeepingScope::IndividualFiles | KeepingScope::Folders => format!("{counted}."),
    }
}

/// `Download N files · 1.5 MB`, or `None`: only a scope with a backlog offers
/// it (`scope-and-backlog`). The bytes are the backlog's, so a size that
/// cannot be read leaves the count alone.
pub(super) fn download_label(
    scope: KeepingScope,
    backlog: usize,
    size: Option<PackageSize>,
) -> Option<String> {
    let files = match (scope, backlog) {
        (KeepingScope::IndividualFiles, _) | (_, 0) => return None,
        (_, 1) => "Download 1 file".to_string(),
        (_, n) => format!("Download {} files", thousands(n)),
    };
    Some(match size {
        Some(size) => format!("{files} · {}", format_size(size.backlog)),
        None => files,
    })
}

#[cfg(test)]
mod tests {
    use super::{Answer, FoldersDialog, Held, KeepingCommands, KeepingSection, download_label};
    use crate::commands::KeepingScope::{self, EntirePackage, Folders, IndividualFiles};
    use crate::commands::{KeepingData, NotDownloaded, PackageSize};
    use crate::kit::BannerVariant;
    use crate::pages::installed_package_v2::{Outcome, Wiring};
    use crate::test_support::{button_saying, element_saying, mount, sleep_ms};
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
                backlog: if outstanding == 0 { 0 } else { 15 * MB / 10 },
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

    fn caption(scope: KeepingScope, held: Held) -> String {
        super::caption(scope, held)
    }

    fn folders(names: &[&str]) -> Vec<String> {
        names.iter().map(ToString::to_string).collect()
    }

    /// The list above names them, so the caption is the count alone.
    #[test]
    fn kept_folders_leave_the_caption_to_the_count() {
        assert_eq!(
            plain(&caption(Folders, plate(2))),
            "54 of 56 files · 1.9 MB of 3.4 MB downloaded."
        );
    }

    #[test]
    fn kept_folders_offer_their_backlog() {
        assert_eq!(
            download_label(Folders, 2, plate(2).size)
                .map(|w| plain(&w))
                .as_deref(),
            Some("Download 2 files · 1.5 MB")
        );
        assert_eq!(download_label(Folders, 0, plate(2).size), None);
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
                backlog: 0,
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
                backlog: 0,
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

    /// Keeping with `outstanding` not here, every one in the backlog unless
    /// the scope is picked files.
    fn data(scope: KeepingScope, total: usize, outstanding: &[&str]) -> KeepingData {
        KeepingData {
            scope,
            folders: if scope == Folders {
                vec!["plate".to_string(), "qc".to_string()]
            } else {
                Vec::new()
            },
            total,
            outstanding: outstanding.len(),
            remote_only: if scope == IndividualFiles {
                Vec::new()
            } else {
                outstanding.iter().map(ToString::to_string).collect()
            },
            deleted_here: 0,
        }
    }

    thread_local! {
        static STORED: RefCell<Vec<(String, KeepingScope, Vec<String>)>> = const { RefCell::new(Vec::new()) };
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

    fn stored() -> Vec<(String, KeepingScope, Vec<String>)> {
        STORED.with_borrow(Clone::clone)
    }

    fn downloaded() -> Vec<(String, Vec<String>)> {
        DOWNLOADED.with_borrow(Clone::clone)
    }

    fn stores_ok(namespace: String, scope: KeepingScope, folders: Vec<String>) -> Answer<()> {
        STORED.with_borrow_mut(|calls| calls.push((namespace, scope, folders)));
        Box::pin(async { Ok(()) })
    }

    fn refuses_store(namespace: String, scope: KeepingScope, folders: Vec<String>) -> Answer<()> {
        STORED.with_borrow_mut(|calls| calls.push((namespace, scope, folders)));
        Box::pin(async { Err("AccessDenied".into()) })
    }

    fn stores_never(namespace: String, scope: KeepingScope, folders: Vec<String>) -> Answer<()> {
        STORED.with_borrow_mut(|calls| calls.push((namespace, scope, folders)));
        Box::pin(std::future::pending())
    }

    fn downloads_ok(namespace: String, paths: Vec<String>) -> Answer<NotDownloaded> {
        DOWNLOADED.with_borrow_mut(|calls| calls.push((namespace, paths)));
        Box::pin(async { Ok(NotDownloaded::default()) })
    }

    fn refuses_download(namespace: String, paths: Vec<String>) -> Answer<NotDownloaded> {
        DOWNLOADED.with_borrow_mut(|calls| calls.push((namespace, paths)));
        Box::pin(async { Err("AccessDenied".into()) })
    }

    fn download_never(namespace: String, paths: Vec<String>) -> Answer<NotDownloaded> {
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
    fn kept_folders_are_listed_under_their_option_with_their_backlog() {
        let el = section(
            data(Folders, 56, &["plate/b.csv", "plate/c.csv"]),
            Wiring::new(),
        );
        let group = group(&el);
        assert!(radio(&group, "Folders").checked());
        let listed: Vec<String> = (0..group.query_selector_all("li").unwrap().length())
            .map(|i| {
                group
                    .query_selector_all("li")
                    .unwrap()
                    .item(i)
                    .unwrap()
                    .text_content()
                    .unwrap()
            })
            .collect();
        assert_eq!(listed, folders(&["plate", "qc"]));
        button_saying(&el, "Change folders");
        element_saying(&el, "54 of 56 downloaded.");
        download_button(&el);
    }

    fn open_dialog(el: &web_sys::Element) -> web_sys::Element {
        el.query_selector("dialog[open]")
            .unwrap()
            .unwrap_or_else(|| panic!("an open dialog; markup was {}", el.inner_html()))
    }

    fn fields(dialog: &web_sys::Element) -> Vec<web_sys::HtmlInputElement> {
        let all = dialog.query_selector_all("input[type=text]").unwrap();
        (0..all.length())
            .map(|i| all.item(i).unwrap().unchecked_into())
            .collect()
    }

    fn type_folder(field: &web_sys::HtmlInputElement, folder: &str) {
        field.set_value(folder);
        field
            .dispatch_event(&web_sys::Event::new("input").unwrap())
            .unwrap();
    }

    /// Folders are stored by Save: the radio only opens the dialog, with one
    /// blank row and nothing to save yet.
    #[wasm_bindgen_test]
    async fn choosing_folders_opens_the_dialog_and_stores_nothing() {
        clear();
        let el = pressable(
            a_backlog_in(IndividualFiles),
            Wiring::new(),
            commands(stores_ok, downloads_ok),
        );
        leptos::task::tick().await;

        element_saying(&el, "Folders").click();
        settle().await;

        let dialog = open_dialog(&el);
        assert_eq!(fields(&dialog).len(), 1);
        assert_eq!(fields(&dialog)[0].value(), "");
        assert!(button_saying(&dialog, "Save").disabled());
        assert_eq!(stored(), Vec::new());
    }

    #[wasm_bindgen_test]
    async fn cancel_returns_the_radio_to_the_stored_choice() {
        clear();
        let el = pressable(
            a_backlog_in(IndividualFiles),
            Wiring::new(),
            commands(stores_ok, downloads_ok),
        );
        leptos::task::tick().await;
        element_saying(&el, "Folders").click();
        settle().await;

        button_saying(&open_dialog(&el), "Cancel").click();
        settle().await;

        assert!(el.query_selector("dialog[open]").unwrap().is_none());
        assert!(radio(&group(&el), "Files I pick").checked());
        assert!(!radio(&group(&el), "Folders").checked());
        assert_eq!(stored(), Vec::new());
    }

    #[wasm_bindgen_test]
    async fn save_stores_the_filled_folders_and_rereads() {
        clear();
        let el = pressable(
            a_backlog_in(IndividualFiles),
            Wiring::new(),
            commands(stores_ok, downloads_ok),
        );
        leptos::task::tick().await;
        element_saying(&el, "Folders").click();
        settle().await;

        let dialog = open_dialog(&el);
        type_folder(&fields(&dialog)[0], "data/raw");
        button_saying(&dialog, "Add folder").click();
        leptos::task::tick().await;
        button_saying(&dialog, "Save").click();
        settle().await;

        assert_eq!(
            stored(),
            vec![("team/dataset".to_string(), Folders, folders(&["data/raw"]))],
            "the blank row is dropped"
        );
        assert_eq!(RELOADS.get(), 1);
        assert!(el.query_selector("dialog[open]").unwrap().is_none());
    }

    /// Saving `.` stores the whole package, which the re-read says; until then
    /// the radio goes back to what is stored rather than claim Folders.
    #[wasm_bindgen_test]
    async fn a_saved_root_leaves_the_radio_on_the_stored_choice() {
        clear();
        let el = pressable(
            a_backlog_in(EntirePackage),
            Wiring::new(),
            commands(stores_ok, downloads_ok),
        );
        leptos::task::tick().await;
        element_saying(&el, "Folders").click();
        settle().await;

        let dialog = open_dialog(&el);
        type_folder(&fields(&dialog)[0], ".");
        leptos::task::tick().await;
        button_saying(&dialog, "Save").click();
        settle().await;

        assert!(el.query_selector("dialog[open]").unwrap().is_none());
        assert!(radio(&group(&el), "The whole package").checked());
        assert_eq!(RELOADS.get(), 1);
    }

    /// The library's refusal stays in the dialog, beside the rows to fix.
    #[wasm_bindgen_test]
    async fn a_refused_save_keeps_the_dialog_open_with_the_reason() {
        clear();
        let el = pressable(
            a_backlog_in(IndividualFiles),
            Wiring::new(),
            commands(refuses_store, downloads_ok),
        );
        leptos::task::tick().await;
        element_saying(&el, "Folders").click();
        settle().await;

        let dialog = open_dialog(&el);
        type_folder(&fields(&dialog)[0], "data/../x");
        leptos::task::tick().await;
        button_saying(&dialog, "Save").click();
        settle().await;

        let alert = open_dialog(&el)
            .query_selector("[role=alert]")
            .unwrap()
            .expect("the refusal, in the dialog");
        assert!(
            alert
                .text_content()
                .unwrap_or_default()
                .contains("AccessDenied")
        );
        assert_eq!(RELOADS.get(), 0);
    }

    /// The section as the page draws it, under the page's folders dialog.
    fn on_page(dialog: FoldersDialog, namespace: &'static str, w: Wiring) -> web_sys::Element {
        mount(move || {
            view! {
                <leptos::context::Provider value=dialog>
                    <KeepingSection
                        namespace=namespace
                        data=a_backlog_in(IndividualFiles)
                        size=None
                        w=w
                        commands=commands(stores_ok, downloads_ok)
                    />
                </leptos::context::Provider>
            }
        })
    }

    /// The page moving to another package closes the dialog and drops its rows.
    #[wasm_bindgen_test]
    async fn closing_for_another_package_drops_the_draft() {
        clear();
        let (dialog, w) = (FoldersDialog::new(), Wiring::new());
        let el = on_page(dialog, "team/dataset", w);
        leptos::task::tick().await;
        element_saying(&el, "Folders").click();
        settle().await;
        type_folder(&fields(&open_dialog(&el))[0], "data/raw");
        leptos::task::tick().await;

        dialog.close();
        settle().await;

        assert!(el.query_selector("dialog[open]").unwrap().is_none());
        assert_eq!(dialog.rows.get_untracked(), Vec::<String>::new());
        assert!(radio(&group(&el), "Files I pick").checked());
    }

    /// A re-read rebuilds the section; the open dialog and its rows survive,
    /// since the page holds them.
    #[wasm_bindgen_test]
    async fn a_reread_keeps_the_open_dialog_and_its_rows() {
        clear();
        let (dialog, w) = (FoldersDialog::new(), Wiring::new());
        let el = on_page(dialog, "team/dataset", w);
        leptos::task::tick().await;
        element_saying(&el, "Folders").click();
        settle().await;
        type_folder(&fields(&open_dialog(&el))[0], "data/raw");
        leptos::task::tick().await;

        let rebuilt = on_page(dialog, "team/dataset", w);
        settle().await;

        let typed: Vec<String> = fields(&open_dialog(&rebuilt))
            .iter()
            .map(web_sys::HtmlInputElement::value)
            .collect();
        assert_eq!(typed, folders(&["data/raw"]));
    }

    #[wasm_bindgen_test]
    async fn change_folders_opens_the_dialog_with_the_kept_ones() {
        clear();
        let el = pressable(
            a_backlog_in(Folders),
            Wiring::new(),
            commands(stores_ok, downloads_ok),
        );
        leptos::task::tick().await;

        button_saying(&el, "Change folders").click();
        settle().await;

        let kept: Vec<String> = fields(&open_dialog(&el))
            .iter()
            .map(web_sys::HtmlInputElement::value)
            .collect();
        assert_eq!(kept, folders(&["plate", "qc"]));
    }

    #[wasm_bindgen_test]
    async fn choosing_an_option_replaces_kept_folders() {
        clear();
        let el = pressable(
            a_backlog_in(Folders),
            Wiring::new(),
            commands(stores_ok, downloads_ok),
        );
        leptos::task::tick().await;

        element_saying(&el, "Files I pick").click();
        settle().await;

        assert_eq!(
            stored(),
            vec![("team/dataset".to_string(), IndividualFiles, Vec::new())]
        );
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

        assert_eq!(
            stored(),
            vec![("team/dataset".to_string(), EntirePackage, Vec::new())]
        );
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
                ("team/dataset".to_string(), EntirePackage, Vec::new()),
                ("team/dataset".to_string(), IndividualFiles, Vec::new()),
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

    fn downloads_skipping_one(namespace: String, paths: Vec<String>) -> Answer<NotDownloaded> {
        DOWNLOADED.with_borrow_mut(|calls| calls.push((namespace, paths)));
        Box::pin(async {
            Ok(NotDownloaded {
                skipped: vec!["plate/c.csv".to_string()],
                hash_mismatch: Vec::new(),
            })
        })
    }

    fn downloads_mismatching_one(namespace: String, paths: Vec<String>) -> Answer<NotDownloaded> {
        DOWNLOADED.with_borrow_mut(|calls| calls.push((namespace, paths)));
        Box::pin(async {
            Ok(NotDownloaded {
                skipped: Vec::new(),
                hash_mismatch: vec!["plate/c.csv".to_string()],
            })
        })
    }

    /// A file whose bytes do not match the package's hash gets a warning that
    /// says the package may be corrupt.
    #[wasm_bindgen_test]
    async fn a_download_with_a_hash_mismatch_names_it_on_the_band() {
        clear();
        let w = Wiring::new();
        let el = pressable(
            a_backlog_in(EntirePackage),
            w,
            commands(stores_ok, downloads_mismatching_one),
        );
        leptos::task::tick().await;

        download_button(&el).click();
        settle().await;

        let said = w.outcome.get_untracked().expect("the band warns");
        assert_eq!(said.variant, BannerVariant::Warning);
        assert_eq!(
            said.lead,
            "Downloaded 1 of 2. 1 file doesn't match the hash the package records. \
             The package may be corrupt."
        );
        assert_eq!(said.detail.as_deref(), Some("plate/c.csv"));
    }

    /// A file the remote no longer holds gets the file pane's warning.
    #[wasm_bindgen_test]
    async fn a_download_that_skips_files_names_them_on_the_band() {
        clear();
        let w = Wiring::new();
        let el = pressable(
            a_backlog_in(EntirePackage),
            w,
            commands(stores_ok, downloads_skipping_one),
        );
        leptos::task::tick().await;

        download_button(&el).click();
        settle().await;

        assert_eq!(RELOADS.get(), 1, "a partial download still re-reads");
        let said = w.outcome.get_untracked().expect("the band warns");
        assert_eq!(said.variant, BannerVariant::Warning);
        assert_eq!(
            said.lead,
            "Downloaded 1 of 2. 1 file is no longer on the remote at this revision."
        );
        assert_eq!(said.detail.as_deref(), Some("plate/c.csv"));
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
