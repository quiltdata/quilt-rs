//! The two `.quiltignore` popups a file row's `[⋯]` opens: *Ignore* and
//! *Stop ignoring*. Each is a kit popup form, so a refusal shows inside it,
//! and each re-reads the page once its write lands.

use leptos::prelude::*;
use quilt_uri::S3PackageUri;

use crate::commands;
use crate::util::{format_size, thousands};
use quilt_ui_kit::{Choice, ChoiceGroup, FormControl, FormDialog, Submit, TextInput};

stylance::import_crate_style!(style, "src/components/ignore_popup.module.css");

/// Runs `on_close` when the dialog closes, by any of its ways out. The caller
/// mounts the popup while it is open and drops it on close.
fn close_with(open: RwSignal<bool>, on_close: impl Fn() + 'static) {
    Effect::new(move |was_open: Option<bool>| {
        let now = open.get();
        if was_open == Some(true) && !now {
            on_close();
        }
        now
    });
}

/// The page's one-command lock, when the caller gave one, is held by another
/// command.
fn held(lock: Option<RwSignal<bool>>) -> bool {
    lock.is_some_and(|l| l.get())
}

/// Hold the page's lock while `write` runs, so `.quiltignore` never changes
/// under a command writing the same tree.
async fn holding<T>(
    lock: Option<RwSignal<bool>>,
    write: impl std::future::Future<Output = T>,
) -> T {
    if let Some(lock) = lock {
        lock.set(true);
    }
    let answer = write.await;
    // `try_`: the lock is a page's, and the page can be gone by now.
    if let Some(lock) = lock {
        lock.try_set(false);
    }
    answer
}

// ── Ignore popup ──

#[derive(Clone, Debug)]
pub struct IgnorePopupData {
    pub namespace: String,
    pub path: String,
    pub suggested_pattern: String,
    /// The package's remote, carried from the row that opened this popup so the
    /// edit it performs is attributed to the deployment the user is working on.
    pub uri: Option<S3PackageUri>,
}

/// What the typed pattern does to the popup's file.
#[derive(Clone)]
pub enum IgnoreHint {
    WillBeIgnored(String),
    OnlyExact(String),
    NoMatch(String),
}

#[component]
#[allow(clippy::needless_pass_by_value)]
pub fn IgnorePopup(
    data: IgnorePopupData,
    refetch: Trigger,
    on_close: impl Fn() + 'static,
    /// A page's one-command lock. Given one, the popup does not add a pattern
    /// while another command holds it, and holds it while its own write runs,
    /// so `.quiltignore` never changes under a command writing the same tree.
    #[prop(optional)]
    lock: Option<RwSignal<bool>>,
) -> impl IntoView {
    let open = RwSignal::new(true);
    close_with(open, on_close);
    let pattern = RwSignal::new(data.suggested_pattern.clone());
    let hint = RwSignal::new(Option::<IgnoreHint>::None);

    let IgnorePopupData {
        namespace,
        path,
        suggested_pattern: suggested,
        uri,
    } = data;

    // What the pattern does to this file, asked again as it is typed.
    Effect::new(move |_| {
        let current = pattern.get();
        let (path, suggested) = (path.clone(), suggested.clone());
        leptos::task::spawn_local(async move {
            if current.trim().is_empty() {
                hint.try_set(None);
                return;
            }
            let matches = commands::test_quiltignore_pattern(current.clone(), path.clone())
                .await
                .unwrap_or(false);

            let is_suggested = current == suggested;
            let is_exact = current == path;

            let value = if (is_suggested || !is_exact) && matches {
                IgnoreHint::WillBeIgnored(path)
            } else if matches && is_exact {
                IgnoreHint::OnlyExact(path)
            } else {
                IgnoreHint::NoMatch(path)
            };
            hint.try_set(Some(value));
        });
    });

    let submit = Submit::new("Add to .quiltignore", move || {
        let (ns, uri, p) = (namespace.clone(), uri.clone(), pattern.get_untracked());
        async move {
            holding(lock, commands::add_to_quiltignore(ns, p, uri))
                .await
                .map_err(|e| format!("Could not ignore this file: {e}"))?;
            refetch.notify();
            Ok(())
        }
    });
    let blocked = Signal::derive(move || pattern.with(|p| p.trim().is_empty()) || held(lock));

    view! {
        <IgnoreDialog open=open pattern=pattern hint=hint.into() submit=submit blocked=blocked />
    }
}

/// The ignore popup, apart from the commands it calls.
#[component]
pub fn IgnoreDialog(
    open: RwSignal<bool>,
    pattern: RwSignal<String>,
    hint: Signal<Option<IgnoreHint>>,
    submit: Submit,
    #[prop(into)] blocked: Signal<bool>,
) -> impl IntoView {
    view! {
        <FormDialog open=open title="Ignore" submit=submit blocked=blocked>
            <FormControl
                label="Pattern to ignore"
                control=move |id| {
                    view! { <TextInput id=id value=pattern autofocus=true /> }.into_any()
                }
            />
            <p class=style::hint>
                {move || hint.get().map(|h| match h {
                    IgnoreHint::WillBeIgnored(path) => view! {
                        <code>{path}</code>" will be ignored"
                    }.into_any(),
                    IgnoreHint::OnlyExact(path) => view! {
                        "Only "{path}" will be ignored"
                    }.into_any(),
                    IgnoreHint::NoMatch(path) => view! {
                        "Doesn't match "<code>{path}</code>
                    }.into_any(),
                })}
            </p>
        </FormDialog>
    }
}

// ── Unignore popup ──

#[derive(Clone, Debug)]
pub struct UnignorePopupData {
    pub namespace: String,
    /// The row's logical key; a folder's has no trailing `/`.
    pub path: String,
    pub folder: bool,
    /// The rule that ignores it.
    pub pattern: String,
    /// See [`IgnorePopupData::uri`].
    pub uri: Option<S3PackageUri>,
}

impl UnignorePopupData {
    /// The path as the popup names it: a folder with a trailing `/`.
    fn shown(&self) -> String {
        if self.folder {
            format!("{}/", self.path)
        } else {
            self.path.clone()
        }
    }
}

/// What the popup knows of each scope's effect.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Preview {
    Counting,
    Ready(commands::StopIgnoringPreview),
    Failed(String),
}

/// The scope chosen when the popup opens.
pub const THIS_PACKAGE: &str = "thisPackage";
const ALL_PACKAGES: &str = "allPackages";

fn scope_of(value: &str) -> commands::StopIgnoringScope {
    if value == ALL_PACKAGES {
        commands::StopIgnoringScope::AllPackages
    } else {
        commands::StopIgnoringScope::ThisPackage
    }
}

/// What the next sync adds, in a sentence. The count is this package's; a
/// user-wide line also brings the same paths back in every other package.
fn adds_sentence(scope: commands::StopIgnoringScope, files: u64, bytes: u64) -> String {
    let all = scope == commands::StopIgnoringScope::AllPackages;
    let (target, rest) = if all {
        (
            "this package",
            ", and the same paths in your other packages",
        )
    } else {
        ("the package", "")
    };
    match files {
        0 if all => "This adds no files to this package on the next sync, but brings back \
                     the same paths in your other packages."
            .to_string(),
        0 => format!("This adds no files to {target} on the next sync."),
        1 => format!(
            "This adds 1 file ({}) to {target} on the next sync{rest}.",
            format_size(bytes)
        ),
        n => format!(
            "This adds {} files ({}) to {target} on the next sync{rest}.",
            thousands(usize::try_from(n).unwrap_or(usize::MAX)),
            format_size(bytes)
        ),
    }
}

/// *Stop ignoring*: writes `!` lines that bring a path back, either to the
/// package's `.quiltignore` or to the user-wide one, after showing what the
/// next sync adds.
#[component]
#[allow(clippy::needless_pass_by_value)]
pub fn UnignorePopup(
    data: UnignorePopupData,
    refetch: Trigger,
    on_close: impl Fn() + 'static,
    /// A page's one-command lock, as [`IgnorePopup`] takes it: given one, the
    /// popup does not write while another command holds it, and holds it while
    /// its own write runs.
    #[prop(optional)]
    lock: Option<RwSignal<bool>>,
) -> impl IntoView {
    let open = RwSignal::new(true);
    close_with(open, on_close);
    let preview = RwSignal::new(Preview::Counting);
    let scope = RwSignal::new(THIS_PACKAGE.to_string());

    {
        let (ns, path, folder) = (data.namespace.clone(), data.path.clone(), data.folder);
        leptos::task::spawn_local(async move {
            let answer = match commands::stop_ignoring_preview(ns, path, folder).await {
                Ok(p) => Preview::Ready(p),
                Err(e) => Preview::Failed(e),
            };
            preview.try_set(answer);
        });
    }

    let submit = {
        let data = data.clone();
        Submit::new("Stop ignoring", move || {
            let data = data.clone();
            let chosen = scope_of(&scope.get_untracked());
            async move {
                let write = commands::stop_ignoring(
                    data.namespace,
                    data.path,
                    data.folder,
                    chosen,
                    data.uri,
                );
                holding(lock, write)
                    .await
                    .map_err(|e| format!("Could not stop ignoring this file: {e}"))?;
                refetch.notify();
                Ok(())
            }
        })
    };

    view! {
        <StopIgnoringDialog
            open=open
            data=data
            preview=preview.into()
            scope=scope
            lock=lock
            submit=submit
        />
    }
}

/// The *Stop ignoring* popup, apart from the commands it calls.
#[component]
#[allow(clippy::needless_pass_by_value)]
pub fn StopIgnoringDialog(
    open: RwSignal<bool>,
    data: UnignorePopupData,
    preview: Signal<Preview>,
    scope: RwSignal<String>,
    lock: Option<RwSignal<bool>>,
    submit: Submit,
) -> impl IntoView {
    let title = format!("Stop ignoring {}", data.shown());
    let pattern = data.pattern.clone();
    // Nothing to write until the count is in.
    let blocked = Signal::derive(move || held(lock) || !matches!(preview.get(), Preview::Ready(_)));

    let body = move || match preview.get() {
        Preview::Counting => view! { <p class=style::line>"Counting files…"</p> }.into_any(),
        Preview::Failed(e) => view! { <p class=style::error>{e}</p> }.into_any(),
        Preview::Ready(p) => {
            let mut options = vec![Choice::new(THIS_PACKAGE, "This package")];
            if p.all_packages.is_some() {
                options.push(Choice::new(ALL_PACKAGES, "All packages"));
            }
            let chosen = {
                let p = p.clone();
                move || match scope_of(&scope.get()) {
                    commands::StopIgnoringScope::AllPackages => p.all_packages.clone(),
                    commands::StopIgnoringScope::ThisPackage => Some(p.this_package.clone()),
                }
            };
            let caption = {
                let chosen = chosen.clone();
                Signal::derive(move || {
                    chosen().map(|c| adds_sentence(scope_of(&scope.get()), c.files, c.bytes))
                })
            };
            let lines = move || {
                let destination = match scope_of(&scope.get()) {
                    commands::StopIgnoringScope::ThisPackage => " to this package's .quiltignore",
                    commands::StopIgnoringScope::AllPackages => " to your rules for every package",
                };
                chosen().map(|c| {
                    let codes = c
                        .lines
                        .into_iter()
                        .enumerate()
                        .map(|(i, line)| {
                            view! { {(i > 0).then_some(" ")}<code>{line}</code> }
                        })
                        .collect_view();
                    view! { <p class=style::line>"Adds "{codes}{destination}</p> }
                })
            };
            view! {
                <ChoiceGroup
                    label="Stop ignoring in"
                    options=options
                    selected=scope
                    caption=caption
                />
                {lines}
            }
            .into_any()
        }
    };

    view! {
        <FormDialog open=open title=title submit=submit blocked=blocked>
            <p class=style::line>"Ignored by: "<code>{pattern}</code></p>
            {body}
        </FormDialog>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quilt_ui_kit::test_support::{button_saying, element_saying, mount};
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::*;

    fn popup(lock: Option<RwSignal<bool>>) -> web_sys::Element {
        let data = IgnorePopupData {
            namespace: "team/dataset".to_string(),
            path: "raw/a.csv".to_string(),
            suggested_pattern: "raw/a.csv".to_string(),
            uri: None,
        };
        mount(move || match lock {
            Some(lock) => view! {
                <IgnorePopup
                    data=data.clone()
                    refetch=Trigger::new()
                    on_close=|| ()
                    lock=lock
                />
            }
            .into_any(),
            None => view! {
                <IgnorePopup
                    data=data.clone()
                    refetch=Trigger::new()
                    on_close=|| ()
                />
            }
            .into_any(),
        })
    }

    fn submit(el: &web_sys::Element) -> web_sys::HtmlButtonElement {
        button_saying(el, "Add to .quiltignore")
    }

    /// Given the page's lock, the popup cannot add a pattern while another
    /// command holds it, and can once it is released.
    #[wasm_bindgen_test]
    async fn a_held_lock_holds_the_submit() {
        let lock = RwSignal::new(true);
        let el = popup(Some(lock));
        assert!(submit(&el).disabled(), "markup was {}", el.inner_html());
        lock.set(false);
        leptos::task::tick().await;
        assert!(!submit(&el).disabled());
    }

    fn unignore_data() -> UnignorePopupData {
        UnignorePopupData {
            namespace: "team/dataset".to_string(),
            path: ".snakemake".to_string(),
            folder: true,
            pattern: ".snakemake/".to_string(),
            uri: None,
        }
    }

    fn ready(all_packages: bool) -> Preview {
        Preview::Ready(commands::StopIgnoringPreview {
            this_package: commands::ScopePreview {
                lines: vec!["!/.snakemake/".to_string()],
                files: 5412,
                bytes: 210_000_000,
            },
            all_packages: all_packages.then(|| commands::ScopePreview {
                lines: vec!["!.snakemake/".to_string()],
                files: 1,
                bytes: 300,
            }),
        })
    }

    fn form(preview: Preview, lock: Option<RwSignal<bool>>) -> web_sys::Element {
        mount(move || {
            view! {
                <StopIgnoringDialog
                    open=RwSignal::new(true)
                    data=unignore_data()
                    preview=Signal::stored(preview)
                    scope=RwSignal::new(THIS_PACKAGE.to_string())
                    lock=lock
                    submit=Submit::new("Stop ignoring", || async { Ok(()) })
                />
            }
        })
    }

    fn radios(el: &web_sys::Element) -> Vec<web_sys::HtmlInputElement> {
        let all = el.query_selector_all("input[type=radio]").unwrap();
        (0..all.length())
            .map(|i| all.item(i).unwrap().unchecked_into())
            .collect()
    }

    /// Given the page's lock, *Stop ignoring* waits while another command
    /// holds it, as the ignore popup's submit does; without one it is live.
    #[wasm_bindgen_test]
    async fn a_held_lock_holds_the_stop() {
        let lock = RwSignal::new(true);
        let el = form(ready(true), Some(lock));
        assert!(button_saying(&el, "Stop ignoring").disabled());
        lock.set(false);
        leptos::task::tick().await;
        assert!(!button_saying(&el, "Stop ignoring").disabled());
        let el = form(ready(true), None);
        assert!(!button_saying(&el, "Stop ignoring").disabled());
    }

    /// *This package* is chosen when the popup opens, and the popup says what
    /// it writes and what the next sync adds.
    #[wasm_bindgen_test]
    fn this_package_is_chosen_at_open() {
        let el = form(ready(true), None);
        let radios = radios(&el);
        assert_eq!(radios.len(), 2, "markup was {}", el.inner_html());
        assert!(radios[0].checked());
        assert!(!radios[1].checked());
        element_saying(&el, "This package");
        element_saying(&el, "!/.snakemake/");
        element_saying(
            &el,
            "This adds 5,412 files (210.0\u{a0}MB) to the package on the next sync.",
        );
        element_saying(&el, "Adds !/.snakemake/ to this package's .quiltignore");
    }

    /// Picking *All packages* shows that scope's line and count.
    #[wasm_bindgen_test]
    async fn all_packages_shows_its_own_line() {
        let el = form(ready(true), None);
        let all = &radios(&el)[1];
        all.set_checked(true);
        all.dispatch_event(&web_sys::Event::new("change").unwrap())
            .unwrap();
        leptos::task::tick().await;
        element_saying(&el, "Adds !.snakemake/ to your rules for every package");
        element_saying(
            &el,
            "This adds 1 file (300\u{a0}B) to this package on the next sync, \
             and the same paths in your other packages.",
        );
    }

    /// When no user-wide line can bring the path back, that option is not drawn.
    #[wasm_bindgen_test]
    fn all_packages_is_absent_when_it_cannot_help() {
        let el = form(ready(false), None);
        assert_eq!(radios(&el).len(), 1, "markup was {}", el.inner_html());
        assert!(!el.inner_html().contains("All packages"));
    }

    /// While counting, and when counting failed, there is nothing to write yet.
    #[wasm_bindgen_test]
    fn no_stop_before_the_count() {
        let el = form(Preview::Counting, None);
        element_saying(&el, "Counting files…");
        assert!(button_saying(&el, "Stop ignoring").disabled());
        let el = form(Preview::Failed("no package".to_string()), None);
        element_saying(&el, "no package");
        assert!(button_saying(&el, "Stop ignoring").disabled());
    }

    /// The popup names the folder with its trailing `/`.
    #[wasm_bindgen_test]
    fn the_title_names_the_folder() {
        let el = form(Preview::Counting, None);
        element_saying(&el, "Stop ignoring .snakemake/");
    }

    #[test]
    fn the_sentence_counts_files_and_size() {
        use commands::StopIgnoringScope::ThisPackage;
        assert_eq!(
            adds_sentence(ThisPackage, 0, 0),
            "This adds no files to the package on the next sync."
        );
        assert_eq!(
            adds_sentence(ThisPackage, 1, 44_000),
            "This adds 1 file (44.0\u{a0}kB) to the package on the next sync."
        );
        assert_eq!(
            adds_sentence(ThisPackage, 5412, 210_000_000),
            "This adds 5,412 files (210.0\u{a0}MB) to the package on the next sync."
        );
    }

    /// A user-wide line applies to every package, and the count is this one's.
    #[test]
    fn the_all_packages_sentence_says_other_packages_change_too() {
        use commands::StopIgnoringScope::AllPackages;
        assert_eq!(
            adds_sentence(AllPackages, 5412, 210_000_000),
            "This adds 5,412 files (210.0\u{a0}MB) to this package on the next sync, \
             and the same paths in your other packages."
        );
        assert_eq!(
            adds_sentence(AllPackages, 1, 44_000),
            "This adds 1 file (44.0\u{a0}kB) to this package on the next sync, \
             and the same paths in your other packages."
        );
        assert_eq!(
            adds_sentence(AllPackages, 0, 0),
            "This adds no files to this package on the next sync, but brings back \
             the same paths in your other packages."
        );
    }

    /// The pattern starts as the one the caller suggested: the pane's path.
    #[wasm_bindgen_test]
    async fn the_pattern_starts_as_suggested_and_no_lock_leaves_the_submit_live() {
        let el = popup(None);
        leptos::task::tick().await;
        let input: web_sys::HtmlInputElement = el
            .query_selector("input[type=text]")
            .unwrap()
            .expect("the pattern field")
            .unchecked_into();
        assert_eq!(input.value(), "raw/a.csv");
        assert!(!submit(&el).disabled());
    }
}
