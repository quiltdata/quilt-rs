use leptos::prelude::*;
use quilt_uri::S3PackageUri;

use crate::commands;
use crate::components::Notification;
use crate::components::buttons;
use crate::kit::{Choice, ChoiceGroup};
use crate::util::{format_size, thousands};

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

#[derive(Clone)]
enum IgnoreHint {
    WillBeIgnored(String),
    OnlyExact(String),
    NoMatch(String),
}

#[component]
#[allow(clippy::needless_pass_by_value)]
#[allow(
    clippy::too_many_lines,
    reason = "declarative Leptos view; length is markup, not logic complexity"
)]
pub fn IgnorePopup(
    data: IgnorePopupData,
    notification: RwSignal<Option<Notification>>,
    refetch: Trigger,
    on_close: impl Fn() + Clone + 'static,
    /// A page's one-command lock. Given one, the popup does not add a pattern
    /// while another command holds it, and holds it while its own write runs,
    /// so `.quiltignore` never changes under a command writing the same tree.
    /// v1 and the commit page pass none.
    #[prop(optional)]
    lock: Option<RwSignal<bool>>,
) -> impl IntoView {
    let pattern = RwSignal::new(data.suggested_pattern.clone());
    let held = move || lock.is_some_and(|l| l.get());
    let hint = RwSignal::new(Option::<IgnoreHint>::None);
    let submitting = RwSignal::new(false);

    let path = data.path.clone();
    let suggested = data.suggested_pattern.clone();
    let namespace = data.namespace.clone();

    let path_for_hint = path.clone();
    let suggested_for_hint = suggested.clone();
    let update_hint = move || {
        let current = pattern.get_untracked();
        let path = path_for_hint.clone();
        let suggested = suggested_for_hint.clone();
        leptos::task::spawn_local(async move {
            if current.trim().is_empty() {
                hint.set(None);
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
            hint.set(Some(value));
        });
    };

    update_hint();

    let update_hint_clone = update_hint.clone();
    let on_input = move |ev: leptos::ev::Event| {
        pattern.set(event_target_value(&ev));
        update_hint_clone();
    };

    let ns_for_submit = namespace.clone();
    let uri_for_submit = data.uri.clone();
    let on_close_for_submit = on_close.clone();
    let on_submit = move || {
        let p = pattern.get_untracked();
        if p.trim().is_empty()
            || submitting.get_untracked()
            || lock.is_some_and(|l| l.get_untracked())
        {
            return;
        }
        submitting.set(true);
        if let Some(lock) = lock {
            lock.set(true);
        }
        let ns = ns_for_submit.clone();
        let uri = uri_for_submit.clone();
        let on_close = on_close_for_submit.clone();
        leptos::task::spawn_local(async move {
            let answer = commands::add_to_quiltignore(ns, p, uri).await;
            // `try_`: the lock is a page's, and the page can be gone by now.
            if let Some(lock) = lock {
                lock.try_set(false);
            }
            match answer {
                Ok(msg) => {
                    notification.set(Some(Notification::Success(msg)));
                    on_close();
                    refetch.notify();
                }
                Err(e) => {
                    notification.set(Some(Notification::Error(e)));
                    submitting.set(false);
                }
            }
        });
    };

    let on_submit_click = {
        let on_submit = on_submit.clone();
        move |_| on_submit()
    };

    let on_close_cancel = on_close.clone();
    let on_cancel = move |_: leptos::ev::MouseEvent| on_close_cancel();

    let on_submit_key = on_submit.clone();
    let on_close_key = on_close.clone();
    let on_keydown = move |ev: leptos::ev::KeyboardEvent| {
        if ev.key() == "Enter" {
            on_submit_key();
        } else if ev.key() == "Escape" {
            on_close_key();
        }
    };

    view! {
        <div class="popup-overlay" on:click={
            let on_close = on_close.clone();
            move |_| on_close()
        }>
            <div class="popup-content" on:click=|ev| ev.stop_propagation()>
                <div class="ignore-popup">
                    <label>"Pattern to ignore:"</label>
                    <input
                        class="ignore-input"
                        type="text"
                        prop:value=move || pattern.get()
                        on:input=on_input
                        on:keydown=on_keydown
                    />
                    <div class="ignore-hint">
                        {move || hint.get().map(|h| match h {
                            IgnoreHint::WillBeIgnored(path) => view! {
                                <code class="inactive">{path}</code>" will be ignored"
                            }.into_any(),
                            IgnoreHint::OnlyExact(path) => view! {
                                "Only "{path}" will be ignored"
                            }.into_any(),
                            IgnoreHint::NoMatch(path) => view! {
                                "Doesn't match "<code class="inactive">{path}</code>
                            }.into_any(),
                        })}
                    </div>
                    <div class="ignore-actions">
                        <buttons::FormPrimary
                            on_click=on_submit_click
                            disabled=Signal::derive(move || submitting.get() || held())
                        >
                            "Add to .quiltignore"
                        </buttons::FormPrimary>
                        <buttons::FormSecondary on_click=on_cancel />
                    </div>
                </div>
            </div>
        </div>
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
enum Preview {
    Counting,
    Ready(commands::StopIgnoringPreview),
    Failed(String),
}

const THIS_PACKAGE: &str = "thisPackage";
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
    notification: RwSignal<Option<Notification>>,
    refetch: Trigger,
    on_close: impl Fn() + Clone + 'static,
    /// A page's one-command lock, as [`IgnorePopup`] takes it: given one, the
    /// popup does not write while another command holds it, and holds it while
    /// its own write runs. v1 and the commit page pass none.
    #[prop(optional)]
    lock: Option<RwSignal<bool>>,
) -> impl IntoView {
    let preview = RwSignal::new(Preview::Counting);
    let scope = RwSignal::new(THIS_PACKAGE.to_string());
    let submitting = RwSignal::new(false);

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

    let on_submit = {
        let data = data.clone();
        let on_close = on_close.clone();
        UnsyncCallback::new(move |()| {
            if submitting.get_untracked()
                || lock.is_some_and(|l| l.get_untracked())
                || !matches!(preview.get_untracked(), Preview::Ready(_))
            {
                return;
            }
            submitting.set(true);
            if let Some(lock) = lock {
                lock.set(true);
            }
            let data = data.clone();
            let chosen = scope_of(&scope.get_untracked());
            let on_close = on_close.clone();
            leptos::task::spawn_local(async move {
                let answer = commands::stop_ignoring(
                    data.namespace,
                    data.path,
                    data.folder,
                    chosen,
                    data.uri,
                )
                .await;
                // `try_`: the lock is a page's, and the page can be gone by now.
                if let Some(lock) = lock {
                    lock.try_set(false);
                }
                match answer {
                    Ok(msg) => {
                        notification.set(Some(Notification::Success(msg)));
                        on_close();
                        refetch.notify();
                    }
                    Err(e) => {
                        notification.set(Some(Notification::Error(e)));
                        submitting.set(false);
                    }
                }
            });
        })
    };

    let on_cancel = {
        let on_close = on_close.clone();
        UnsyncCallback::new(move |()| on_close())
    };
    let escape = window_event_listener(leptos::ev::keydown, move |ev| {
        if ev.key() == "Escape" {
            on_cancel.run(());
        }
    });
    on_cleanup(move || escape.remove());

    view! {
        <div class="popup-overlay" on:click=move |_| on_cancel.run(())>
            <div class="popup-content" on:click=|ev| ev.stop_propagation()>
                <StopIgnoringForm
                    data=data
                    preview=preview.into()
                    scope=scope
                    submitting=submitting
                    lock=lock
                    on_submit=on_submit
                    on_cancel=on_cancel
                />
            </div>
        </div>
    }
}

/// The popup's body, apart from the commands it calls.
#[component]
#[allow(clippy::needless_pass_by_value)]
fn StopIgnoringForm(
    data: UnignorePopupData,
    preview: Signal<Preview>,
    scope: RwSignal<String>,
    submitting: RwSignal<bool>,
    lock: Option<RwSignal<bool>>,
    on_submit: UnsyncCallback<()>,
    on_cancel: UnsyncCallback<()>,
) -> impl IntoView {
    let shown = data.shown();
    let pattern = data.pattern.clone();
    let disabled = Signal::derive(move || {
        submitting.get()
            || lock.is_some_and(|l| l.get())
            || !matches!(preview.get(), Preview::Ready(_))
    });

    let body = move || match preview.get() {
        Preview::Counting => view! { <p class="unignore-status">"Counting files…"</p> }.into_any(),
        Preview::Failed(e) => view! { <p class="unignore-error">{e}</p> }.into_any(),
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
                    view! { <p class="unignore-lines">"Adds "{codes}{destination}</p> }
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
        <div class="unignore-popup">
            <h2 class="section-title">"Stop ignoring "<code>{shown}</code></h2>
            <span>"Ignored by: "<span class="pattern-display">{pattern}</span></span>
            {body}
            <div class="ignore-actions">
                <buttons::FormPrimary on_click=move |_| on_submit.run(()) disabled=disabled>
                    "Stop ignoring"
                </buttons::FormPrimary>
                <buttons::FormSecondary on_click=move |_| on_cancel.run(()) />
            </div>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{button_saying, element_saying, mount};
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
                    notification=RwSignal::new(None)
                    refetch=Trigger::new()
                    on_close=|| ()
                    lock=lock
                />
            }
            .into_any(),
            None => view! {
                <IgnorePopup
                    data=data.clone()
                    notification=RwSignal::new(None)
                    refetch=Trigger::new()
                    on_close=|| ()
                />
            }
            .into_any(),
        })
    }

    fn submit(el: &web_sys::Element) -> web_sys::HtmlButtonElement {
        el.query_selector(".ignore-actions button")
            .unwrap()
            .expect("the submit")
            .unchecked_into()
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
                <StopIgnoringForm
                    data=unignore_data()
                    preview=Signal::stored(preview)
                    scope=RwSignal::new(THIS_PACKAGE.to_string())
                    submitting=RwSignal::new(false)
                    lock=lock
                    on_submit=UnsyncCallback::new(|()| ())
                    on_cancel=UnsyncCallback::new(|()| ())
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

    /// The pattern starts as the one the caller suggested: the v2 pane's path.
    #[wasm_bindgen_test]
    async fn the_pattern_starts_as_suggested_and_no_lock_leaves_the_submit_live() {
        let el = popup(None);
        leptos::task::tick().await;
        let input: web_sys::HtmlInputElement = el
            .query_selector(".ignore-input")
            .unwrap()
            .expect("the pattern field")
            .unchecked_into();
        assert_eq!(input.value(), "raw/a.csv");
        assert!(!submit(&el).disabled());
    }
}
