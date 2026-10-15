//! The commit page v2's regions, as views over props.
//!
//! The gallery draws them over fixtures, and [`CommitV2`] fills the same
//! props from `get_commit_data`. Every effect is a callback.
//! [`CommitColumn`] fixes the order: header, problem, message, workflow,
//! metadata, then the files. Wide, the files are a column beside the form.
//! v2 vocabulary only; a test holds the fixed strings to the banned words.

use leptos::prelude::*;

use crate::kit::ActionMenu;
use crate::kit::Banner;
use crate::kit::BannerVariant;
use crate::kit::Blankslate;
use crate::kit::Button;
use crate::kit::ButtonVariant;
use crate::kit::Card;
use crate::kit::EntryRow;
use crate::kit::FormControl;
use crate::kit::JsonDisplay;
use crate::kit::NameTooltip;
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
use crate::kit::TwoColumns;
use crate::kit::icons;
use crate::util::format_size;

use super::installed_package_v2::{MenuCommand, MenuItem, remote_item, remove_item};
use super::json_editor::{JsonEditor, focus_json_editor};
use super::row_menu::{RowCommand, menu_actions};

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

/// The header's `[⋯]`: the package page's bucket item, then Remove below a
/// rule. Of the package page's other items, *Create new revision* is this
/// page, and undoing a revision belongs on the package page, where the files
/// it restores are in view.
#[must_use]
pub fn header_menu(remote_locked: bool, busy: bool) -> Vec<MenuItem> {
    vec![
        remote_item(remote_locked, busy),
        MenuItem {
            separated: true,
            ..remove_item(busy)
        },
    ]
}

/// The page's header. No state label: it would be about the package, not the
/// revision being written.
#[component]
pub fn CommitHeader(
    #[prop(into)] namespace: String,
    #[prop(into)] package_href: String,
    primary: Primary,
    w: PrimaryWiring,
    on_open_folder: Callback<()>,
    /// The package has been pushed, so `[⋯]` offers *Show remote* in place
    /// of *Change bucket*.
    remote_locked: bool,
    /// The command chosen in `[⋯]`.
    on_menu: Callback<MenuCommand>,
    /// The trail's first crumb; `/` by default.
    #[prop(optional, into)]
    home_href: Option<String>,
) -> impl IntoView {
    let blocked = w.blocked;
    let running = w.running;
    // Derived, so the items follow the lock while the menu is open.
    let menu = Signal::derive(move || {
        header_menu(remote_locked, running.get())
            .into_iter()
            .map(|item| {
                let command = item.command.clone();
                item.action(Callback::new(move |()| on_menu.run(command.clone())))
            })
            .collect()
    });
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
                <ActionMenu aria_label="More actions for this package" actions=menu />
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
    // Not a `FormControl`: the name shares its line with the editor's toggle.
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
    // Here so the label can reach it: the label names the hidden textarea,
    // and a click on it must land in the editor the reader sees.
    let editor_ref = NodeRef::<leptos::html::Div>::new();
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
                editor_ref,
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
                                    on:click=move |ev| {
                                        if focus_json_editor(editor_ref) {
                                            ev.prevent_default();
                                        }
                                    }
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
                <button
                    type="button"
                    class=style::inline_action
                    aria-expanded=move || editing.get().to_string()
                    aria-controls=body_id.clone()
                    on:click=move |_| editing.update(|open| *open = !*open)
                >
                    {move || {
                        if editing.get() {
                            "Close editor"
                        } else if MetadataPreview::of(&metadata.get()) == MetadataPreview::None {
                            "Add"
                        } else {
                            "Edit"
                        }
                    }}
                </button>
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

/// The metadata editor alone, always open, with its label and its error:
/// Settings' publishing popup. The editor reads `metadata` once, as it
/// mounts, so it is drawn again whenever `fresh` changes: a value set from
/// outside it, such as a reset, reaches it that way.
#[component]
pub fn MetadataEditorField(
    metadata: RwSignal<String>,
    #[prop(optional, into)] error: MaybeProp<String>,
    #[prop(into)] label: String,
    /// Changes when `metadata` was set from outside the editor.
    #[prop(optional, into)]
    fresh: Signal<u64>,
    /// The editor's opening height, in px. The grip's lower limit still applies.
    #[prop(optional)]
    height: Option<i32>,
) -> impl IntoView {
    let failing = Signal::derive(move || error.get().is_some());
    let control_id = crate::kit::unique_id("metadata");
    let error_id = format!("{control_id}-error");
    let label_id = format!("{control_id}-label");
    // Here so a height the reader dragged to survives a reset.
    let height = RwSignal::new(height.map_or(EDITOR_HEIGHT, |h| dragged_height(h, 0)));
    let editor_ref = NodeRef::<leptos::html::Div>::new();
    let editor = {
        let control_id = control_id.clone();
        let label_id = label_id.clone();
        let error_id = error_id.clone();
        move || {
            fresh.track();
            metadata_editor(
                control_id.clone(),
                label_id.clone(),
                error_id.clone(),
                metadata,
                failing,
                height,
                editor_ref,
            )
        }
    };
    view! {
        <div class=style::field>
            <label
                class=style::field_name
                id=label_id
                for=control_id
                on:click=move |ev| {
                    if focus_json_editor(editor_ref) {
                        ev.prevent_default();
                    }
                }
            >
                {label}
            </label>
            <div class=style::field_value>{editor}</div>
            <Show when=move || error.get().is_some()>
                <p class=style::field_error id=error_id.clone()>
                    {StateTone::Danger.glyph()}
                    <span>{move || error.get().unwrap_or_default()}</span>
                </p>
            </Show>
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
    editor_ref: NodeRef<leptos::html::Div>,
) -> AnyView {
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

pub const NO_FILE_CHANGES: &str = "No file changes";
pub const METADATA_ONLY: &str = "This revision only updates metadata.";

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

/// The items a row's `[⋯]` offers. Every file here differs from the published
/// one, so its address would name other bytes: there is no catalog item and no
/// *Copy URI*, and *Copy path* copies where the file is on disk. A deleted file
/// has nothing on disk to open, reveal or copy.
fn row_commands(file: &IncludedFile) -> Vec<RowCommand> {
    let path = file.path.clone();
    let mut commands = Vec::new();
    if file.change != Change::Deleted {
        commands.push(RowCommand::Open(path.clone()));
        commands.push(RowCommand::Reveal(path.clone()));
        commands.push(RowCommand::CopyPath(path.clone()));
    }
    commands.push(RowCommand::Ignore(path));
    commands
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
    /// What a row's `[⋯]` chose.
    on_command: Callback<RowCommand>,
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
        view! {
            <Blankslate
                visual=icons::file()
                heading=NO_FILE_CHANGES
                description=METADATA_ONLY
            />
        }
        .into_any()
    } else {
        view! {
            // A box of its own, so the rows scroll under a header that stays.
            <NameTooltip>
            <div class=style::rows>
            <Card flush=true list=true fill=true>
                {files
                    .into_iter()
                    .map(|file| {
                        let actions = menu_actions(&row_commands(&file), on_command);
                        view! {
                            <li>
                                <EntryRow
                                    name=file.path
                                    state=file.change.words().to_string()
                                    tone=file.change.tone()
                                    size=format_size(file.size)
                                    actions=actions
                                    plain=true
                                />
                            </li>
                        }
                    })
                    .collect_view()}
            </Card>
            </div>
            </NameTooltip>
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

/// The page's column, in the order the reader confirms. The header and the
/// banner span it; under them the form and the files are two columns on a
/// wide page and one column on a narrow one.
///
/// It must be a direct child of `PageLayout`'s main: it fills that height,
/// and the list's own scroll box takes what is left of it. A wrapper between
/// them sizes to the content, and the whole page scrolls again.
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
            // Two columns from about a 1024px window, as on Settings.
            <TwoColumns min_column=480 fill_height=true>
                <div class=style::form>{message} {workflow} {metadata}</div>
                {included}
            </TwoColumns>
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
mod layout_tests {
    use super::*;
    use crate::test_support::{load_module_style, mount, sleep_ms};
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::*;

    stylance::import_crate_style!(columns, "src/kit/two_columns.module.scss");
    stylance::import_crate_style!(card, "src/kit/card.module.scss");
    stylance::import_crate_style!(form_control, "src/kit/form_control.module.scss");
    stylance::import_crate_style!(text_input, "src/kit/text_input.module.scss");

    /// The kit's spacing tokens, which the test page does not load.
    const TOKENS: &str = "--q-space-3:12px; --q-space-4:16px";

    fn region(name: &'static str, height: u32) -> AnyView {
        view! { <div data-region=name style=format!("height:{height}px")></div> }.into_any()
    }

    /// The page's, the layout's and the card's stylesheets, as the app has them.
    fn load_styles() {
        load_module_style(
            include_str!("../kit/two_columns.module.scss"),
            &[
                ("grid", columns::grid),
                ("tight", columns::tight),
                ("fill", columns::fill),
            ],
        );
        load_module_style(
            include_str!("../kit/card.module.scss"),
            &[
                ("root", card::root),
                ("title", card::title),
                ("count", card::count),
                ("body", card::body),
                ("flush", card::flush),
                ("fill", card::fill),
            ],
        );
        load_module_style(
            include_str!("commit_v2.module.scss"),
            &[
                ("column", style::column),
                ("form", style::form),
                ("included", style::included),
                ("included_head", style::included_head),
                ("included_heading", style::included_heading),
                ("rows", style::rows),
                ("tally", style::tally),
                ("quiet", style::quiet),
            ],
        );
    }

    /// The column with stand-in regions, `width` px wide.
    async fn column(width: u32) -> web_sys::Element {
        load_styles();
        let el = mount(move || {
            view! {
                <div style=format!(
                    "{TOKENS}; width:{width}px; height:600px; display:flex; flex-direction:column",
                )>
                    <CommitColumn
                        header=region("header", 60)
                        problem=region("problem", 40)
                        message=region("message", 60)
                        workflow=region("workflow", 60)
                        metadata=region("metadata", 60)
                        included=region("included", 100)
                    />
                </div>
            }
        });
        sleep_ms(0).await;
        el
    }

    fn rect(el: &web_sys::Element, name: &str) -> web_sys::DomRect {
        el.query_selector(&format!("[data-region={name}]"))
            .unwrap()
            .unwrap()
            .get_bounding_client_rect()
    }

    /// Wide: the files beside the form, and the header and the banner across
    /// both.
    #[wasm_bindgen_test]
    #[allow(clippy::float_cmp, reason = "box edges here are whole pixels")]
    async fn wide_puts_the_files_beside_the_form() {
        let el = column(1200).await;
        let (message, included) = (rect(&el, "message"), rect(&el, "included"));
        assert_eq!(included.top(), message.top(), "side by side");
        assert!(included.left() > message.right(), "files on the right");
        assert_eq!(rect(&el, "header").width(), 1200.0, "the header spans");
        assert_eq!(rect(&el, "problem").width(), 1200.0, "the banner spans");
    }

    /// Narrow: one column, the files last.
    #[wasm_bindgen_test]
    #[allow(clippy::float_cmp, reason = "box edges here are whole pixels")]
    async fn narrow_puts_the_files_last() {
        let el = column(800).await;
        let (metadata, included) = (rect(&el, "metadata"), rect(&el, "included"));
        assert!(
            included.top() >= metadata.bottom(),
            "the files under the form"
        );
        assert_eq!(included.left(), metadata.left(), "one column");
    }

    /// The column with a real list of `n` files, in a 600px page that scrolls
    /// as `PageLayout`'s frame does.
    async fn page_with_files(width: u32, n: usize) -> web_sys::Element {
        load_styles();
        let el = mount(move || {
            let files = (0..n)
                .map(|i| IncludedFile {
                    path: format!("data/file-{i:03}.csv"),
                    size: 1000,
                    change: Change::New,
                })
                .collect::<Vec<_>>();
            view! {
                <div
                    data-page=""
                    style=format!(
                        "{TOKENS}; width:{width}px; height:600px; display:flex; \
                         flex-direction:column; overflow-y:auto",
                    )
                >
                    <CommitColumn
                        header=region("header", 60)
                        message=region("message", 60)
                        workflow=region("workflow", 60)
                        metadata=region("metadata", 60)
                        included=view! {
                            <IncludedList files=files ignored=0 on_command=Callback::new(|_| ()) />
                        }
                            .into_any()
                    />
                </div>
            }
        });
        sleep_ms(0).await;
        el
    }

    /// A long list scrolls in its own box, so the page does not scroll and
    /// the header stays in view, wide and narrow.
    #[wasm_bindgen_test]
    async fn a_long_list_scrolls_in_its_own_box() {
        for width in [1200, 800] {
            let el = page_with_files(width, 100).await;
            let page = el.query_selector("[data-page]").unwrap().unwrap();
            assert_eq!(
                page.scroll_height(),
                page.client_height(),
                "{width}: the page does not scroll"
            );
            let ul = el.query_selector("ul").unwrap().unwrap();
            assert!(
                ul.client_height() > 0 && ul.scroll_height() > ul.client_height(),
                "{width}: the rows scroll in their box ({} of {})",
                ul.client_height(),
                ul.scroll_height()
            );
        }
    }

    /// The message field's border and the file box's border start on one
    /// line: a field's label and the files' heading leave the same room.
    #[wasm_bindgen_test]
    async fn the_message_and_the_files_line_up() {
        load_styles();
        load_module_style(
            include_str!("../kit/form_control.module.scss"),
            &[
                ("root", form_control::root),
                ("name", form_control::name),
                ("required", form_control::required),
                ("caption", form_control::caption),
                ("error", form_control::error),
            ],
        );
        load_module_style(
            include_str!("../kit/text_input.module.scss"),
            &[("root", text_input::root), ("invalid", text_input::invalid)],
        );
        // The app's baseline (modern-normalize) zeroes a control's margin,
        // which browsers otherwise set differently on each platform.
        load_module_style("input { margin: 0; }", &[]);
        let el = mount(|| {
            let files = vec![IncludedFile {
                path: "data/a.csv".to_string(),
                size: 10,
                change: Change::New,
            }];
            view! {
                <div style=format!(
                    "{TOKENS}; --q-space-1:4px; --q-space-2:8px; --q-text-body:14px; \
                     --q-line-body:20px; font-size:14px; line-height:20px; width:1200px; \
                     height:600px; display:flex; flex-direction:column",
                )>
                    <CommitColumn
                        header=region("header", 60)
                        message=view! {
                            <MessageField value=RwSignal::new(String::new()) placeholder="m" />
                        }
                            .into_any()
                        workflow=region("workflow", 60)
                        metadata=region("metadata", 60)
                        included=view! {
                            <IncludedList files=files ignored=0 on_command=Callback::new(|_| ()) />
                        }
                            .into_any()
                    />
                </div>
            }
        });
        sleep_ms(0).await;
        let top = |sel: &str| {
            el.query_selector(sel)
                .unwrap()
                .unwrap()
                .get_bounding_client_rect()
                .top()
        };
        let card = el
            .query_selector("ul")
            .unwrap()
            .and_then(|ul| ul.parent_element())
            .unwrap()
            .get_bounding_client_rect()
            .top();
        // Within a pixel: fonts differ by platform, and so does the fraction.
        let (input, card) = (top("input"), card);
        assert!(
            (input - card).abs() < 1.0,
            "the input's top {input}, the card's {card}"
        );
    }

    /// The editor's toggle sits right after the field's name and says what
    /// it does: `Edit`, then `Close editor`.
    #[wasm_bindgen_test]
    async fn the_editor_toggle_follows_the_name() {
        let el = mount(|| {
            view! {
                <MetadataField
                    metadata=RwSignal::new("{\"plate\": 7}".to_string())
                    editing=RwSignal::new(false)
                />
            }
        });
        let toggle = |el: &web_sys::Element| {
            el.query_selector("[aria-expanded][aria-controls]")
                .unwrap()
                .expect("the toggle")
        };
        let name = toggle(&el).previous_element_sibling().expect("the name");
        assert_eq!(name.text_content().as_deref(), Some("Metadata"));
        assert_eq!(toggle(&el).text_content().as_deref(), Some("Edit"));
        let button: web_sys::HtmlElement = toggle(&el).unchecked_into();
        button.click();
        sleep_ms(0).await;
        assert_eq!(toggle(&el).text_content().as_deref(), Some("Close editor"));
    }

    /// With no file changes, a blank state with a glyph says so.
    #[wasm_bindgen_test]
    fn metadata_only_is_a_blank_state() {
        let el = mount(|| {
            view! { <IncludedList files=Vec::new() ignored=0 on_command=Callback::new(|_| ()) /> }
        });
        let text = el.text_content().unwrap_or_default();
        assert!(text.contains(NO_FILE_CHANGES), "{text}");
        assert!(text.contains(METADATA_ONLY), "{text}");
        assert!(el.query_selector("svg").unwrap().is_some(), "the glyph");
    }

    /// The rows have no checkboxes, so they start with the path.
    #[wasm_bindgen_test]
    fn rows_start_with_the_path() {
        let el = mount(|| {
            let files = vec![IncludedFile {
                path: "data/a.csv".to_string(),
                size: 10,
                change: Change::New,
            }];
            view! { <IncludedList files=files ignored=0 on_command=Callback::new(|_| ()) /> }
        });
        let row = el
            .query_selector("li")
            .unwrap()
            .and_then(|li| li.first_element_child())
            .unwrap();
        let main = row.first_element_child().unwrap();
        let first = main.first_element_child().unwrap();
        assert_eq!(first.text_content().as_deref(), Some("data/a.csv"));
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
            NO_FILE_CHANGES.to_string(),
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

    /// Before the first publish the bucket can change; after, it can only be
    /// shown. Remove follows below a rule, and both wait for the page's lock.
    #[test]
    fn the_header_menu_offers_the_bucket_then_remove() {
        let words = |items: Vec<MenuItem>| -> Vec<(String, bool, bool)> {
            items
                .into_iter()
                .map(|i| (i.label, i.separated, i.disabled.is_some()))
                .collect()
        };
        let menu = |label: &str, busy: bool| {
            vec![
                (label.to_string(), false, busy),
                ("Remove".to_string(), true, busy),
            ]
        };
        assert_eq!(
            words(header_menu(false, false)),
            menu("Change bucket", false)
        );
        assert_eq!(words(header_menu(true, false)), menu("Show remote", false));
        assert_eq!(words(header_menu(false, true)), menu("Change bucket", true));
        let items = header_menu(true, false);
        assert_eq!(items[0].command, MenuCommand::Remote);
        assert_eq!(items[1].command, MenuCommand::Remove);
        assert_eq!(items[1].tone, crate::kit::ActionTone::Danger);
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

    fn file(change: Change) -> IncludedFile {
        IncludedFile {
            path: "raw/a.csv".to_string(),
            size: 4_100,
            change,
        }
    }

    /// A row's items with a `|` where a rule goes.
    fn grouped(change: Change) -> String {
        menu_actions(&row_commands(&file(change)), Callback::new(|_| ()))
            .iter()
            .map(|a| {
                if a.separated {
                    format!("| {}", a.label)
                } else {
                    a.label.clone()
                }
            })
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// The local file, where it is, then `.quiltignore`; a deleted file has
    /// only the last, with no rule above it.
    #[test]
    fn rules_fall_between_the_groups_a_row_has() {
        let local = "Open file, Reveal in folder, | Copy path, | Ignore";
        assert_eq!(grouped(Change::Changed), local);
        assert_eq!(grouped(Change::New), local);
        assert_eq!(grouped(Change::Deleted), "Ignore");
    }

    async fn open_items(el: &web_sys::Element) -> Vec<web_sys::HtmlElement> {
        use wasm_bindgen::JsCast;
        el.query_selector("button[aria-label='More actions for this file']")
            .unwrap()
            .expect("the row's [⋯]")
            .unchecked_into::<web_sys::HtmlElement>()
            .click();
        leptos::task::tick().await;
        let found = el.query_selector_all("[popover] button").unwrap();
        (0..found.length())
            .map(|i| found.item(i).unwrap().unchecked_into())
            .collect()
    }

    /// The row's `[⋯]` lists its items, and each hands the page its command.
    #[wasm_bindgen_test::wasm_bindgen_test]
    async fn a_row_s_menu_hands_the_chosen_command_to_the_page() {
        let chosen = RwSignal::new(None);
        let el = crate::test_support::mount(move || {
            view! {
                <IncludedList
                    files=vec![file(Change::New)]
                    ignored=0
                    on_command=Callback::new(move |c| chosen.set(Some(c)))
                />
            }
        });
        let words: Vec<String> = open_items(&el)
            .await
            .iter()
            .map(|b| b.text_content().unwrap_or_default())
            .collect();
        assert_eq!(
            words,
            ["Open file", "Reveal in folder", "Copy path", "Ignore"]
        );
        assert_eq!(
            el.query_selector_all("[popover] [role='separator']")
                .unwrap()
                .length(),
            2,
        );
        let path = || "raw/a.csv".to_string();
        for (i, command) in [
            RowCommand::Open(path()),
            RowCommand::Reveal(path()),
            RowCommand::CopyPath(path()),
            RowCommand::Ignore(path()),
        ]
        .into_iter()
        .enumerate()
        {
            open_items(&el).await[i].click();
            assert_eq!(chosen.get_untracked(), Some(command));
        }
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
