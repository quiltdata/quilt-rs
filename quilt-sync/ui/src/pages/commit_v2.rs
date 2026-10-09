//! The commit page v2's regions, as views over props.
//!
//! The gallery draws them over fixtures, and [`CommitV2`] fills the same
//! props from `get_commit_data`. Every effect is a callback.
//! [`CommitColumn`] fixes the order: header, problem, message, workflow,
//! metadata, then the files, last so a long list scrolls under the form.
//! v2 vocabulary only; a test holds the fixed strings to the banned words.

use leptos::prelude::*;

use crate::kit::Banner;
use crate::kit::BannerVariant;
use crate::kit::Button;
use crate::kit::ButtonVariant;
use crate::kit::Card;
use crate::kit::EntryRow;
use crate::kit::FormControl;
use crate::kit::JsonDisplay;
use crate::kit::MenuAction;
use crate::kit::Naming;
use crate::kit::PageHeader;
use crate::kit::PageHeaderSkeleton;
use crate::kit::Select;
use crate::kit::SkeletonBox;
use crate::kit::SplitButton;
use crate::kit::SplitOption;
use crate::kit::StateTone;
use crate::kit::TextInput;
use crate::kit::Tooltip;
use crate::kit::Trail;
use crate::util::format_size;

use super::json_editor::JsonEditor;

stylance::import_crate_style!(style, "src/pages/commit_v2.module.scss");

mod page;
pub use page::{CommitV2, CommitV2Skeleton};

/// What the revision will do, which decides the primary's words.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Primary {
    /// `Publish N files`.
    Files(usize),
    /// Only metadata changed: `Publish revision`.
    MetadataOnly,
    /// No bucket: `Save revision`, with no caret.
    LocalOnly,
}

impl Primary {
    #[must_use]
    pub fn label(self) -> String {
        match self {
            Self::Files(1) => "Publish 1 file".to_string(),
            Self::Files(n) => format!("Publish {n} files"),
            Self::MetadataOnly => "Publish revision".to_string(),
            Self::LocalOnly => "Save revision".to_string(),
        }
    }
}

pub const SAVE_WITHOUT_PUBLISHING: &str = "Save without publishing";

/// The primary's callbacks and state, which the page owns.
#[derive(Clone, Copy)]
pub struct PrimaryWiring {
    /// The split button's face: `0` publishes, `1` saves.
    pub choice: RwSignal<usize>,
    pub on_publish: Callback<()>,
    pub on_save: Callback<()>,
    /// Why it cannot run; disables it and is its tooltip.
    pub blocked: Signal<Option<String>>,
    pub running: Signal<bool>,
}

fn primary_face(primary: Primary, w: PrimaryWiring) -> AnyView {
    let PrimaryWiring {
        choice,
        on_publish,
        on_save,
        blocked,
        running,
    } = w;
    let disabled = Signal::derive(move || blocked.with(Option::is_some));
    match primary {
        Primary::LocalOnly => view! {
            <Button
                variant=ButtonVariant::Primary
                disabled=disabled
                loading=running
                on_click=move |_| on_save.run(())
            >
                {primary.label()}
            </Button>
        }
        .into_any(),
        Primary::Files(_) | Primary::MetadataOnly => view! {
            <SplitButton
                options=vec![
                    SplitOption::new(primary.label(), on_publish),
                    SplitOption::new(SAVE_WITHOUT_PUBLISHING, on_save),
                ]
                selected=choice
                menu_label="Change what this button does"
                variant=ButtonVariant::Primary
                disabled=disabled
                loading=running
            />
        }
        .into_any(),
    }
}

/// The page's header. No state label and no `[⋯]`: both would be about the
/// package, not the revision being written.
#[component]
pub fn CommitHeader(
    #[prop(into)] namespace: String,
    #[prop(into)] package_href: String,
    primary: Primary,
    w: PrimaryWiring,
    on_open_folder: Callback<()>,
    /// The trail's first crumb; `/` by default.
    #[prop(optional, into)]
    home_href: Option<String>,
) -> impl IntoView {
    let blocked = w.blocked;
    let home_href = home_href.unwrap_or_else(|| "/".to_string());
    // The reason is also on the page: a disabled button takes no focus, and
    // touch shows no tooltip.
    let face = move || match blocked.get() {
        Some(reason) => view! {
            <Tooltip
                text=reason
                align=crate::kit::Align::End
                trigger=move |id| {
                    view! {
                        <span class=style::slot data-primary-action aria-describedby=id>
                            {primary_face(primary, w)}
                        </span>
                    }
                    .into_any()
                }
            />
        }
        .into_any(),
        None => view! {
            <span class=style::slot data-primary-action>
                {primary_face(primary, w)}
            </span>
        }
        .into_any(),
    };

    view! {
        <PageHeader
            trail=view! {
                <Trail crumbs=vec![
                    (home_href, "Packages".to_string()),
                    (package_href, namespace),
                ] />
            }
                .into_any()
            title="New revision"
            actions=view! {
                {face}
                <Button disabled=w.running on_click=move |_| on_open_folder.run(())>
                    "Open folder"
                </Button>
            }
                .into_any()
        />
    }
}

/// What the problems banner reports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Problem {
    /// System files such as `.DS_Store`.
    Junk { count: usize, names: Vec<String> },
    /// The role cannot write to the bucket. Blocks saving too: saving checks
    /// the workflow, which reads the bucket's config.
    NoAccess { reason: String },
    /// No session; blocks saving too. `host` is `None` for ambient credentials.
    SignedOut { host: Option<String> },
    // A newer published revision: deferred.
}

impl Problem {
    #[must_use]
    pub fn words(&self) -> String {
        match self {
            Self::Junk { count, names } => {
                let files = if *count == 1 {
                    "1 system file would be published".to_string()
                } else {
                    format!("{count} system files would be published")
                };
                if names.is_empty() {
                    files
                } else {
                    format!("{files} ({})", names.join(", "))
                }
            }
            Self::NoAccess { reason } => format!("{reason}."),
            Self::SignedOut { host: Some(host) } => format!("You are signed out of {host}."),
            Self::SignedOut { host: None } => {
                "There are no credentials for this bucket. Add some to publish.".to_string()
            }
        }
    }

    const fn variant(&self) -> BannerVariant {
        match self {
            Self::NoAccess { .. } | Self::SignedOut { .. } => BannerVariant::Critical,
            Self::Junk { .. } => BannerVariant::Warning,
        }
    }
}

/// The problems banner, with its remedy as the banner's action.
#[component]
#[allow(
    clippy::needless_pass_by_value,
    reason = "a component's props are owned; the body reads them from there"
)]
pub fn ProblemBanner(
    problem: Problem,
    #[prop(optional)] on_ignore_junk: Option<Callback<()>>,
    #[prop(optional, into)] sign_in_href: Option<String>,
) -> impl IntoView {
    let remedy = match &problem {
        Problem::Junk { .. } => on_ignore_junk.map(|ignore| {
            view! {
                <Button on_click=move |_| ignore.run(())>
                    "Ignore them"
                </Button>
            }
            .into_any()
        }),
        Problem::SignedOut { host: Some(_) } => sign_in_href
            .map(|href| view! { <a class=style::remedy_link href=href>"Sign in"</a> }.into_any()),
        _ => None,
    };
    match remedy {
        Some(remedy) => view! {
            <Banner variant=problem.variant() action=remedy>
                {problem.words()}
            </Banner>
        }
        .into_any(),
        None => view! { <Banner variant=problem.variant()>{problem.words()}</Banner> }.into_any(),
    }
}

/// The message. The placeholder is what an empty field publishes.
#[component]
pub fn MessageField(
    value: RwSignal<String>,
    #[prop(into)] placeholder: String,
    #[prop(optional, into)] error: MaybeProp<String>,
    /// Set when arriving from `Create new revision`.
    #[prop(optional)]
    autofocus: bool,
) -> impl IntoView {
    let invalid = Signal::derive(move || error.get().is_some());
    view! {
        <FormControl
            label="Message"
            error=error
            control=move |id| {
                view! {
                    <TextInput
                        id=id
                        value=value
                        placeholder=placeholder
                        invalid=invalid
                        autofocus=autofocus
                    />
                }
                    .into_any()
            }
        />
    }
}

#[derive(Clone)]
pub struct WorkflowChoice {
    pub options: Vec<String>,
    pub selected: RwSignal<String>,
}

/// The workflow: a live `Select`, or its words when there is nothing to choose.
#[component]
#[allow(
    clippy::needless_pass_by_value,
    reason = "a component's props are owned; the body reads them from there"
)]
pub fn WorkflowField(
    /// `None` shows `no_workflow` instead.
    #[prop(default = None)]
    workflow: Option<WorkflowChoice>,
    #[prop(optional, into)] no_workflow: Option<String>,
    #[prop(optional, into)] error: MaybeProp<String>,
    /// The value came from the publish settings, the one source worth naming.
    #[prop(optional, into)]
    from_settings: Signal<bool>,
    /// `/settings` by default.
    #[prop(optional, into)]
    settings_href: Option<String>,
    /// A warning under the field that is not the check's: the Settings
    /// workflow this bucket cannot use.
    #[prop(optional, into)]
    note: Option<String>,
) -> impl IntoView {
    let hint = settings_hint(from_settings, settings_href);
    let note = note.map(|note| view! { <p class=style::field_hint>{note}</p> });
    let Some(choice) = workflow else {
        let words = no_workflow.unwrap_or_else(|| "None".to_string());
        return view! {
            <div class=style::field>
                <div class=style::field_name>"Workflow"</div>
                <div class=style::field_value>{words}</div>
                {note}
                {hint}
            </div>
        }
        .into_any();
    };
    view! {
        <div class=style::field>
            <FormControl
                label="Workflow"
                error=error
                control=move |id| {
                    view! {
                        <Select
                            naming=Naming::FormControl(id)
                            options=choice.options.clone()
                            selected=choice.selected
                        />
                    }
                        .into_any()
                }
            />
            {note}
            {hint}
        </div>
    }
    .into_any()
}

/// The metadata: a [`JsonDisplay`] preview until `Edit` swaps in v1's editor,
/// the one tall control on the page. Nothing above the editor may set
/// `overflow`, or its context menu is cropped. A failed check opens it.
#[component]
pub fn MetadataField(
    metadata: RwSignal<String>,
    editing: RwSignal<bool>,
    #[prop(optional, into)] error: MaybeProp<String>,
    /// See [`WorkflowField`].
    #[prop(optional, into)]
    from_settings: Signal<bool>,
    #[prop(optional, into)] settings_href: Option<String>,
    /// `Metadata` by default; Settings' reads `Default metadata`.
    #[prop(optional, into)]
    label: Option<String>,
) -> impl IntoView {
    let label = label.unwrap_or_else(|| "Metadata".to_string());
    let failing = Signal::derive(move || error.get().is_some());
    // On the edge into failure only, so folding it over an error sticks.
    Effect::new(move |was: Option<bool>| {
        let now = failing.get();
        if now && !was.unwrap_or(false) && !editing.get_untracked() {
            editing.set(true);
        }
        now
    });
    // Not a `FormControl`: the name shares its row with `Edit`.
    let control_id = crate::kit::unique_id("metadata");
    let error_id = format!("{control_id}-error");
    let label_id = format!("{control_id}-label");
    let body_id = format!("{control_id}-body");
    let preview = move || match MetadataPreview::of(&metadata.get()) {
        MetadataPreview::None => {
            view! { <span class=style::field_empty>"No metadata"</span> }.into_any()
        }
        MetadataPreview::Invalid => view! { <span>"Not valid JSON"</span> }.into_any(),
        MetadataPreview::Json(value) => view! { <JsonDisplay value=value /> }.into_any(),
    };
    // Here rather than in the editor, so a height the reader dragged to
    // survives `Done` and `Edit`.
    let height = RwSignal::new(EDITOR_HEIGHT);
    let editor = {
        let control_id = control_id.clone();
        let label_id = label_id.clone();
        let error_id = error_id.clone();
        move || {
            metadata_editor(
                control_id.clone(),
                label_id.clone(),
                error_id.clone(),
                metadata,
                failing,
                height,
            )
        }
    };

    view! {
        <div class=style::field>
            <div class=style::field_head>
                {
                    let control_id = control_id.clone();
                    move || {
                        let label = label.clone();
                        let label_id = label_id.clone();
                        if editing.get() {
                            view! {
                                <label
                                    class=style::field_name
                                    id=label_id
                                    for=control_id.clone()
                                >
                                    {label}
                                </label>
                            }
                                .into_any()
                        } else {
                            view! { <span class=style::field_name id=label_id>{label}</span> }
                                .into_any()
                        }
                    }
                }
                <Button
                    aria_expanded=Signal::derive(move || Some(editing.get()))
                    aria_controls=body_id.clone()
                    on_click=move |_| editing.update(|open| *open = !*open)
                >
                    {move || {
                        if editing.get() {
                            "Done"
                        } else if MetadataPreview::of(&metadata.get()) == MetadataPreview::None {
                            "Add"
                        } else {
                            "Edit"
                        }
                    }}
                </Button>
            </div>
            <div class=style::field_value id=body_id>
                {move || if editing.get() { editor.clone()() } else { preview().into_any() }}
            </div>
            <Show when=move || error.get().is_some()>
                <p class=style::field_error id=error_id.clone()>
                    {StateTone::Danger.glyph()}
                    <span>{move || error.get().unwrap_or_default()}</span>
                </p>
            </Show>
            {settings_hint(from_settings, settings_href)}
        </div>
    }
}

/// Shown while the publish settings supplied the value: a global default can
/// fail in a bucket that does not expect it.
fn settings_hint(from_settings: Signal<bool>, href: Option<String>) -> impl IntoView {
    let href = href.unwrap_or_else(|| "/settings".to_string());
    move || {
        from_settings.get().then(|| {
            view! {
                <p class=style::field_hint>
                    "From your publish settings. "
                    <a class=style::remedy_link href=href.clone()>"Change it in Settings"</a>
                </p>
            }
        })
    }
}

/// The editor's height on opening, and the least it can be dragged to. 150px
/// keeps a failed check's error inside the 560px window.
const EDITOR_HEIGHT: i32 = 150;

/// The most the grip drags the editor to: about 30 lines of JSON, and short
/// of the 900px default window.
const EDITOR_MAX_HEIGHT: i32 = 600;

/// How far one arrow key moves the editor's lower edge.
const EDITOR_STEP: i32 = 24;

/// The editor's height after a drag of `dy` from `start`.
fn dragged_height(start: i32, dy: i32) -> i32 {
    (start + dy).clamp(EDITOR_HEIGHT, EDITOR_MAX_HEIGHT)
}

/// The editor's height after `key` on its grip, or `None` for another key.
fn keyed_height(height: i32, key: &str) -> Option<i32> {
    match key {
        "ArrowDown" => Some(dragged_height(height, EDITOR_STEP)),
        "ArrowUp" => Some(dragged_height(height, -EDITOR_STEP)),
        "Home" => Some(EDITOR_HEIGHT),
        "End" => Some(EDITOR_MAX_HEIGHT),
        _ => None,
    }
}

/// The metadata control: v1's editor over its fallback textarea, with a grip
/// under it that drags it taller. Not CSS `resize`: that needs `overflow` on
/// the box, which crops the editor's context menu.
fn metadata_editor(
    id: String,
    labelled_by: String,
    described_by: String,
    metadata: RwSignal<String>,
    invalid: Signal<bool>,
    height: RwSignal<i32>,
) -> AnyView {
    let editor_ref = NodeRef::<leptos::html::Div>::new();
    let textarea_ref = NodeRef::<leptos::html::Textarea>::new();
    let editor_described_by = described_by.clone();
    view! {
        // Its own wrapper: the glue hides the textarea's parent on mount.
        <div>
            <textarea
                node_ref=textarea_ref
                id=id
                aria-describedby=move || invalid.get().then(|| described_by.clone())
                aria-invalid=move || invalid.get().then_some("true")
                class=style::metadata
                rows=6
                spellcheck="false"
                prop:value=move || metadata.get_untracked()
                on:input=move |ev| metadata.set(event_target_value(&ev))
            />
        </div>
        // The glue hides the textarea, so the editor a reader sees carries
        // the field's name and its error too, as a named group.
        <div
            role="group"
            aria-labelledby=labelled_by
            aria-describedby=move || invalid.get().then(|| editor_described_by.clone())
            class=move || invalid.get().then_some(style::invalid)
            style:height=move || format!("{}px", height.get())
        >
            <JsonEditor
                node_ref=editor_ref
                textarea_ref=textarea_ref
                initial_value=metadata.get_untracked()
                class=style::editor
            />
        </div>
        {editor_grip(height)}
    }
    .into_any()
}

/// The bar under the editor: drag it, or focus it and use the arrow keys.
fn editor_grip(height: RwSignal<i32>) -> impl IntoView {
    // (pointer, its y, height) where the drag began. Only that pointer moves
    // or ends it, so a second finger cannot take over.
    let drag = StoredValue::new(None::<(i32, i32, i32)>);
    let end = move |ev: leptos::ev::PointerEvent| {
        if drag
            .get_value()
            .is_some_and(|(id, _, _)| id == ev.pointer_id())
        {
            drag.set_value(None);
        }
    };
    view! {
        <div
            class=style::grip
            role="separator"
            aria-orientation="horizontal"
            aria-label="Resize the metadata editor"
            aria-valuemin=EDITOR_HEIGHT
            aria-valuemax=EDITOR_MAX_HEIGHT
            aria-valuenow=move || height.get()
            tabindex="0"
            on:pointerdown=move |ev: leptos::ev::PointerEvent| {
                if ev.button() != 0 || drag.get_value().is_some() {
                    return;
                }
                ev.prevent_default();
                if let Some(grip) = ev
                    .current_target()
                    .and_then(|t| wasm_bindgen::JsCast::dyn_into::<web_sys::Element>(t).ok())
                {
                    let _ = grip.set_pointer_capture(ev.pointer_id());
                }
                drag.set_value(Some((ev.pointer_id(), ev.client_y(), height.get_untracked())));
            }
            on:pointermove=move |ev: leptos::ev::PointerEvent| {
                if let Some((id, y, start)) = drag.get_value()
                    && id == ev.pointer_id()
                {
                    height.set(dragged_height(start, ev.client_y() - y));
                }
            }
            on:pointerup=end
            on:pointercancel=end
            on:keydown=move |ev: leptos::ev::KeyboardEvent| {
                if let Some(next) = keyed_height(height.get_untracked(), &ev.key()) {
                    ev.prevent_default();
                    height.set(next);
                }
            }
        ></div>
    }
}

/// What the folded metadata shows.
#[derive(Debug, PartialEq)]
pub enum MetadataPreview {
    /// Empty text or `{}`.
    None,
    Invalid,
    Json(serde_json::Value),
}

impl MetadataPreview {
    #[must_use]
    pub fn of(text: &str) -> Self {
        if text.trim().is_empty() {
            return Self::None;
        }
        match serde_json::from_str::<serde_json::Value>(text) {
            Ok(serde_json::Value::Object(fields)) if fields.is_empty() => Self::None,
            Ok(value) => Self::Json(value),
            Err(_) => Self::Invalid,
        }
    }
}

/// How a file differs, in the v2 file pane's words.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    Changed,
    New,
    Deleted,
}

impl Change {
    #[must_use]
    pub const fn words(self) -> &'static str {
        match self {
            Self::Changed => "Changed",
            Self::New => "New",
            Self::Deleted => "Deleted",
        }
    }

    #[must_use]
    pub const fn tone(self) -> StateTone {
        match self {
            Self::Changed | Self::New => StateTone::Attention,
            Self::Deleted => StateTone::Danger,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IncludedFile {
    pub path: String,
    pub size: u64,
    pub change: Change,
}

pub const METADATA_ONLY: &str = "No file changes. This revision only updates metadata.";

fn files_words(n: usize) -> String {
    if n == 1 {
        "1 file".to_string()
    } else {
        format!("{n} files")
    }
}

/// Said when the list shows fewer files than the revision includes: the
/// page's read is capped, the revision is not.
#[must_use]
pub fn cut_words(shown: usize, total: usize) -> String {
    format!("Showing {shown} of {total} files. All {total} are included.")
}

fn ignored_words(n: usize) -> String {
    if n == 1 {
        "1 ignored file not included".to_string()
    } else {
        format!("{n} ignored files not included")
    }
}

/// The changed files, then how many ignored files are left out. No facets and
/// no search: narrowing what will be published only hides some of it.
#[component]
#[allow(
    clippy::needless_pass_by_value,
    reason = "a component's props are owned; the body reads them from there"
)]
pub fn IncludedList(
    files: Vec<IncludedFile>,
    ignored: usize,
    /// A row's `[⋯]` → `Ignore`, with its path.
    on_ignore: Callback<String>,
    /// The heading's `(files, bytes)` when `files` is not all of them: the
    /// page's list is capped, its totals are not. Counted from `files` by
    /// default.
    #[prop(optional)]
    totals: Option<(usize, u64)>,
) -> impl IntoView {
    let (count, total) =
        totals.unwrap_or_else(|| (files.len(), files.iter().map(|f| f.size).sum()));
    let cut = (count > files.len() && !files.is_empty()).then(|| cut_words(files.len(), count));
    let heading_id = crate::kit::unique_id("included");
    let labelled_by = heading_id.clone();
    let tally = (count > 0).then(|| format!(" · {} · {}", files_words(count), format_size(total)));
    let rows = if files.is_empty() {
        view! { <p class=style::quiet>{METADATA_ONLY}</p> }.into_any()
    } else {
        view! {
            // A box of its own, so the rows scroll under a header that stays.
            <div class=style::rows>
            <Card flush=true list=true fill=true>
                {files
                    .into_iter()
                    .map(|file| {
                        let path = file.path.clone();
                        let ignore = MenuAction::new(
                            "Ignore",
                            Callback::new(move |()| on_ignore.run(path.clone())),
                        );
                        view! {
                            <li>
                                <EntryRow
                                    name=file.path
                                    state=file.change.words().to_string()
                                    tone=file.change.tone()
                                    size=format_size(file.size)
                                    actions=vec![ignore]
                                />
                            </li>
                        }
                    })
                    .collect_view()}
            </Card>
            </div>
        }
        .into_any()
    };
    view! {
        <section class=style::included aria-labelledby=labelled_by>
            // The ignored line shares the heading's row: under the box it
            // took a row from the list in a short window.
            <div class=style::included_head>
                <h3 class=style::included_heading id=heading_id>
                    "What's included"
                    <span class=style::tally>{tally}</span>
                </h3>
                {(ignored > 0)
                    .then(|| {
                        view! {
                            <p class=style::quiet>
                                {ignored_words(ignored)}
                            </p>
                        }
                    })}
            </div>
            {rows}
            {cut.map(|words| view! { <p class=style::quiet>{words}</p> })}
        </section>
    }
}

/// The page's column, in the order the reader confirms. It must be a direct
/// child of `PageLayout`'s main: it fills that height, and the list's own
/// scroll box takes what the form leaves of it. A wrapper between them sizes
/// to the content, and the whole page scrolls again.
#[component]
pub fn CommitColumn(
    /// For the page to reach the column's fields, in place of a wrapper.
    #[prop(optional)]
    node_ref: NodeRef<leptos::html::Div>,
    header: AnyView,
    #[prop(optional)] problem: Option<AnyView>,
    message: AnyView,
    workflow: AnyView,
    metadata: AnyView,
    included: AnyView,
) -> impl IntoView {
    view! {
        <div class=style::column node_ref=node_ref>
            {header}
            {problem}
            {message}
            {workflow}
            {metadata}
            {included}
        </div>
    }
}

/// The page while `get_commit_data` is in flight.
#[component]
pub fn CommitPageSkeleton() -> impl IntoView {
    view! {
        <div class=style::column aria-busy="true">
            <PageHeaderSkeleton />
            <div class=style::skeleton_field>
                <SkeletonBox width="64px" height="16px" />
                <SkeletonBox width="100%" height="var(--q-control-height)" />
            </div>
            <SkeletonBox width="100%" height="var(--q-control-height)" />
            <SkeletonBox width="200px" height="16px" />
        </div>
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// `package_state.rs`'s list, plus `commits` and `hash`.
    const BANNED: &[&str] = &[
        "commit", "commits", "push", "pull", "remote", "behind", "ahead", "diverged", "dirty",
        "hash",
    ];

    pub(crate) fn banned_in(words: &str) -> Option<&'static str> {
        let lower = words.to_lowercase();
        BANNED.iter().copied().find(|bad| {
            lower
                .split_whitespace()
                .any(|w| w.trim_matches(|c: char| !c.is_alphanumeric()) == *bad)
        })
    }

    #[test]
    fn no_fixed_string_uses_a_banned_word() {
        let mut all = vec![
            SAVE_WITHOUT_PUBLISHING.to_string(),
            METADATA_ONLY.to_string(),
            "New revision".to_string(),
            "Ignore them".to_string(),
            "Metadata".to_string(),
            "What's included".to_string(),
            cut_words(1000, 2000),
        ];
        for primary in [
            Primary::Files(1),
            Primary::Files(3),
            Primary::MetadataOnly,
            Primary::LocalOnly,
        ] {
            all.push(primary.label());
        }
        for problem in [
            Problem::Junk {
                count: 2,
                names: vec![".DS_Store".to_string()],
            },
            Problem::NoAccess {
                reason: "Your role cannot write to this bucket".to_string(),
            },
            Problem::SignedOut {
                host: Some("example.com".to_string()),
            },
            Problem::SignedOut { host: None },
        ] {
            all.push(problem.words());
        }
        for change in [Change::Changed, Change::New, Change::Deleted] {
            all.push(change.words().to_string());
        }
        for words in &all {
            assert_eq!(banned_in(words), None, "{words:?}");
        }
    }

    #[test]
    fn the_editor_grows_between_its_opening_height_and_its_cap() {
        assert_eq!(dragged_height(EDITOR_HEIGHT, 90), EDITOR_HEIGHT + 90);
        assert_eq!(dragged_height(EDITOR_HEIGHT + 30, -90), EDITOR_HEIGHT);
        assert_eq!(
            dragged_height(EDITOR_MAX_HEIGHT - 10, 90),
            EDITOR_MAX_HEIGHT
        );
        assert_eq!(
            keyed_height(EDITOR_MAX_HEIGHT, "ArrowDown"),
            Some(EDITOR_MAX_HEIGHT)
        );
        assert_eq!(keyed_height(EDITOR_HEIGHT, "End"), Some(EDITOR_MAX_HEIGHT));
        assert_eq!(
            keyed_height(EDITOR_HEIGHT, "ArrowDown"),
            Some(EDITOR_HEIGHT + EDITOR_STEP)
        );
        assert_eq!(keyed_height(EDITOR_HEIGHT, "ArrowUp"), Some(EDITOR_HEIGHT));
        assert_eq!(keyed_height(EDITOR_HEIGHT * 2, "Home"), Some(EDITOR_HEIGHT));
        assert_eq!(keyed_height(EDITOR_HEIGHT, "Enter"), None);
    }

    #[test]
    fn the_metadata_preview_draws_json_and_words_for_the_rest() {
        assert_eq!(
            MetadataPreview::of("{\"plate\": 7}"),
            MetadataPreview::Json(serde_json::json!({"plate": 7}))
        );
        assert_eq!(MetadataPreview::of("  "), MetadataPreview::None);
        assert_eq!(MetadataPreview::of("{}"), MetadataPreview::None);
        assert_eq!(MetadataPreview::of("{\"a\": "), MetadataPreview::Invalid);
    }

    #[test]
    fn the_primary_counts_its_files() {
        assert_eq!(Primary::Files(1).label(), "Publish 1 file");
        assert_eq!(Primary::Files(12).label(), "Publish 12 files");
    }

    #[test]
    fn the_junk_banner_names_what_it_found() {
        let problem = Problem::Junk {
            count: 2,
            names: vec![".DS_Store".to_string()],
        };
        assert_eq!(
            problem.words(),
            "2 system files would be published (.DS_Store)"
        );
    }
}
