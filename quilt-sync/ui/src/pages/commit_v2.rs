//! The commit page v2's regions, as views over props.
//!
//! Not routed yet. The gallery draws these over fixture data, and the port
//! that puts the page behind `ByDesign` fills the same props from
//! `get_commit_data`, so the page and its gallery scene are one drawing rather
//! than two. Nothing here calls a command: every effect is a callback the
//! caller owns, which is the kit's rule applied to a page's regions.
//!
//! # One column, in the order you confirm
//!
//! 1. [`CommitHeader`]: the trail, `New revision`, and the one primary.
//! 2. [`ProblemBanner`], only when there is a problem.
//! 3. [`MessageField`]: the generated message is the placeholder, so an empty
//!    field means "use it".
//! 4. [`WorkflowSection`]: folded to the exact workflow and a one-line
//!    preview of the metadata, with `Edit` opening the select and a
//!    full-width metadata editor.
//! 5. [`IncludedList`]: the changed files, last, so a long list scrolls the
//!    page under the form rather than pushing the form below the fold.
//!
//! [`CommitColumn`] fixes that order in one place.
//!
//! # The page's words
//!
//! The v2 vocabulary only: a revision is saved or published, and nothing here
//! names a hash. A test holds every fixed string to the same banned list as
//! the package states.

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

/// What the revision will do, which decides the primary's words and whether it
/// has a caret.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Primary {
    /// Files changed, and the package has a bucket: `Publish N files`.
    Files(usize),
    /// Only the message, workflow or metadata changed: `Publish revision`.
    MetadataOnly,
    /// No bucket to publish to: `Save revision`, and no caret, because there
    /// is no other way to do it.
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

/// The caret's other option: save the revision here and publish it later.
pub const SAVE_WITHOUT_PUBLISHING: &str = "Save without publishing";

/// The primary's callbacks and state, which the page owns.
#[derive(Clone, Copy)]
pub struct PrimaryWiring {
    /// Which option the split button's face shows: `0` publishes, `1` saves.
    /// The page's, so it can remember a deliberate pick.
    pub choice: RwSignal<usize>,
    pub on_publish: Callback<()>,
    pub on_save: Callback<()>,
    /// Why it cannot run, when it cannot: a failed workflow check, no access,
    /// no session. Disables it, and the tooltip says why.
    pub blocked: Signal<Option<String>>,
    /// A save or publish is in flight.
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

/// The page's header: the trail up through the package, the page's name, and
/// one primary beside `Open folder`.
///
/// No state label and no `[⋯]`. The label would describe the package rather
/// than the revision being written, and everything a menu could hold is
/// already on the page.
#[component]
pub fn CommitHeader(
    #[prop(into)] namespace: String,
    /// The package page, which the trail's second crumb goes back to.
    #[prop(into)]
    package_href: String,
    primary: Primary,
    w: PrimaryWiring,
    on_open_folder: Callback<()>,
    /// The trail's first crumb. `/` in the app, which renders whichever main
    /// page is switched on; the gallery points it at its own cell.
    #[prop(optional, into)]
    home_href: Option<String>,
) -> impl IntoView {
    let blocked = w.blocked;
    let home_href = home_href.unwrap_or_else(|| "/".to_string());
    // The tooltip only while there is a reason. The reason is also on the page,
    // under the field or in the banner, because a disabled button takes no
    // focus and a touch screen shows no tooltip; the tooltip is for the pointer
    // that rests on the button wondering why.
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

/// Something that stands in the way of publishing, or would make the revision
/// worse than it means to be.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Problem {
    /// System files the user almost certainly did not mean to publish.
    Junk {
        count: usize,
        /// The distinct names, such as `.DS_Store`.
        names: Vec<String>,
    },
    /// The active role cannot write to the bucket, in the roster's own words.
    /// Blocks saving as well as publishing: saving checks the workflow, which
    /// reads the bucket's config first.
    NoAccess { reason: String },
    /// No session to publish with. Blocks saving too, for the same reason. `host` is the deployment to sign in to;
    /// `None` for ambient credentials, whose remedy is outside the app.
    SignedOut { host: Option<String> },
    // A newer published revision is a problem too, deferred to a later item.
}

impl Problem {
    /// The sentence the banner says.
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
            // The banner's `Sign in` is the rest of the sentence.
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

/// The problems banner: one bar, standing while its cause does, with the
/// remedy beside the sentence when there is one to offer.
#[component]
#[allow(
    clippy::needless_pass_by_value,
    reason = "a component's props are owned; the body reads them from there"
)]
pub fn ProblemBanner(
    problem: Problem,
    /// `Ignore them`, for junk files.
    #[prop(optional)]
    on_ignore_junk: Option<Callback<()>>,
    /// Where `Sign in` goes, for a signed-out page with a host.
    #[prop(optional, into)]
    sign_in_href: Option<String>,
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

/// The message. Its placeholder is the message the revision gets when the
/// field is left empty, so an empty field is a choice and not an omission.
#[component]
pub fn MessageField(
    value: RwSignal<String>,
    /// The generated message.
    #[prop(into)]
    placeholder: String,
    /// A failed workflow check on the message.
    #[prop(optional, into)]
    error: MaybeProp<String>,
    /// Coming from `Create new revision`, the message is the point, so it is
    /// focused; coming from `Review before publishing…`, nothing is.
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

/// The workflow choice, when the bucket offers one.
#[derive(Clone)]
pub struct WorkflowChoice {
    pub options: Vec<String>,
    pub selected: RwSignal<String>,
}

/// Workflow and metadata: what the revision will carry, until somebody wants
/// to change it.
///
/// Folded, it names the exact workflow — the select's own option, so it
/// follows a change — and previews the metadata on one line (see
/// [`JsonDisplay`], folded to a line that opens in place), with `Edit`. Open, it is the workflow `Select` and
/// v1's metadata editor at full width. Its context menu and dropdowns are drawn
/// inside the editor's own box, so nothing between it and the page may clip:
/// no ancestor here sets `overflow`, and the page has no second column that
/// scrolls on its own the way v1's did.
///
/// It opens on its own when a check fails, because an error inside a closed
/// section is an error nobody sees. The reader can close it again; it opens
/// once per failure, not on every keystroke while one stands.
#[component]
#[allow(
    clippy::needless_pass_by_value,
    reason = "a component's props are owned; the body reads them from there"
)]
pub fn WorkflowSection(
    /// What the workflow line says when there is no choice to read it from:
    /// a package with no bucket, or a bucket with no workflows.
    #[prop(optional, into)]
    no_workflow: Option<String>,
    /// The selected workflow is the publish settings' default. Said under the
    /// field, and only then: the other sources — the bucket's default, the
    /// published revision's, a pick made here — are what the reader expects.
    #[prop(optional, into)]
    workflow_from_settings: Signal<bool>,
    /// The metadata is the publish settings' default, for the same reason.
    #[prop(optional, into)]
    metadata_from_settings: Signal<bool>,
    /// Where `Change it in Settings` goes. `/settings` in the app; the gallery
    /// points it at its own cell.
    #[prop(optional, into)]
    settings_href: Option<String>,
    expanded: RwSignal<bool>,
    /// `None` when the bucket has no workflows to choose between.
    #[prop(default = None)]
    workflow: Option<WorkflowChoice>,
    #[prop(optional, into)] workflow_error: MaybeProp<String>,
    /// The metadata, as JSON text.
    metadata: RwSignal<String>,
    #[prop(optional, into)] metadata_error: MaybeProp<String>,
) -> impl IntoView {
    let failing =
        Signal::derive(move || workflow_error.get().is_some() || metadata_error.get().is_some());
    // Opens on the edge into failure, so a reader who closes it over a standing
    // error is not overruled on the next keystroke. Untracked read of
    // `expanded`, so closing it does not run this again.
    Effect::new(move |was: Option<bool>| {
        let now = failing.get();
        if now && !was.unwrap_or(false) && !expanded.get_untracked() {
            expanded.set(true);
        }
        now
    });
    let body_id = crate::kit::unique_id("workflow-meta");
    let controls = body_id.clone();
    let metadata_invalid = Signal::derive(move || metadata_error.get().is_some());
    // The selected option itself, so the folded line names the exact workflow
    // the revision will carry, and follows the select when it changes.
    let selected = workflow.as_ref().map(|choice| choice.selected);
    let no_workflow = no_workflow.unwrap_or_else(|| "None".to_string());
    let workflow_words = move || selected.map_or_else(|| no_workflow.clone(), |s| s.get());
    // Drawn as the catalog draws metadata: one folded line that opens in place,
    // so it can be read without the editor.
    let metadata_view = move || match MetadataPreview::of(&metadata.get()) {
        MetadataPreview::None => view! { <span>"None"</span> }.into_any(),
        MetadataPreview::Invalid => view! { <span>"Not valid JSON"</span> }.into_any(),
        MetadataPreview::Json(value) => view! { <JsonDisplay value=value /> }.into_any(),
    };

    view! {
        <section class=style::workflow aria-label="Workflow and metadata">
            <div class=style::summary_row>
                <span class=style::section_name>"Workflow & metadata"</span>
                <span class=style::summary_action>
                    <Button
                        aria_expanded=Signal::derive(move || Some(expanded.get()))
                        aria_controls=controls
                        on_click=move |_| expanded.update(|open| *open = !*open)
                    >
                        {move || if expanded.get() { "Done" } else { "Edit" }}
                    </Button>
                </span>
            </div>
            // Folded, what the revision will carry; open, the controls say it.
            <Show when=move || !expanded.get()>
                // Two fields in the form's own shape — a name, the value, and a
                // hint under it — read-only until `Edit`.
                <dl class=style::preview>
                    <div class=style::field>
                        <dt class=style::field_name>"Workflow"</dt>
                        <dd class=style::field_value>{workflow_words.clone()}</dd>
                        {settings_hint(workflow_from_settings, settings_href.clone())}
                    </div>
                    <div class=style::field>
                        <dt class=style::field_name>"Metadata"</dt>
                        <dd class=style::field_value>{metadata_view}</dd>
                        {settings_hint(metadata_from_settings, settings_href.clone())}
                    </div>
                </dl>
            </Show>
            <Show when=move || expanded.get()>
                <div class=style::workflow_body id=body_id.clone()>
                    {workflow
                        .clone()
                        .map(|choice| {
                            view! {
                                <FormControl
                                    label="Workflow"
                                    error=workflow_error
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
                            }
                        })}
                    <FormControl
                        label="Metadata"
                        error=metadata_error
                        control=move |id| metadata_editor(id, metadata, metadata_invalid)
                    />
                </div>
            </Show>
        </section>
    }
}

/// The hint under a folded field whose value the publish settings supplied,
/// while they do. A global default can make a publish fail in a bucket that
/// does not expect it, so this is the source worth naming; the others are not.
fn settings_hint(from_settings: Signal<bool>, href: Option<String>) -> impl IntoView {
    let href = href.unwrap_or_else(|| "/settings".to_string());
    move || {
        from_settings.get().then(|| {
            view! {
                <dd class=style::field_hint>
                    "From your publish settings. "
                    <a class=style::remedy_link href=href.clone()>"Change it in Settings"</a>
                </dd>
            }
        })
    }
}

/// The metadata control: v1's editor over its fallback textarea.
fn metadata_editor(
    id: crate::kit::ControlId,
    metadata: RwSignal<String>,
    invalid: Signal<bool>,
) -> AnyView {
    let (id, described_by) = id.into_attrs();
    let editor_ref = NodeRef::<leptos::html::Div>::new();
    let textarea_ref = NodeRef::<leptos::html::Textarea>::new();
    view! {
        // The textarea's own wrapper, because the glue hides the textarea's
        // parent once the editor mounts. It is the fallback, and what the
        // editor writes back to: the glue fires `input` on every edit.
        <div>
            <textarea
                node_ref=textarea_ref
                id=id
                aria-describedby=described_by
                aria-invalid=move || invalid.get().then_some("true")
                class=style::metadata
                rows=6
                spellcheck="false"
                prop:value=move || metadata.get_untracked()
                on:input=move |ev| metadata.set(event_target_value(&ev))
            />
        </div>
        // The red border on the editor's own frame, which `.invalid` reaches
        // through its custom property.
        <div class=move || invalid.get().then_some(style::invalid)>
            <JsonEditor
                node_ref=editor_ref
                textarea_ref=textarea_ref
                initial_value=metadata.get_untracked()
                class=style::editor
            />
        </div>
    }
    .into_any()
}

/// What the folded section shows for the metadata text.
#[derive(Debug, PartialEq)]
pub enum MetadataPreview {
    /// Empty text or an empty object: words, not braces.
    None,
    /// Text that does not parse. Its error is what opens the section.
    Invalid,
    /// A value worth drawing, as the catalog draws it.
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

/// How a file differs from the current revision. The v2 file pane's words.
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

/// One changed file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IncludedFile {
    pub path: String,
    pub size: u64,
    pub change: Change,
}

/// The one sentence for a revision with no file changes.
pub const METADATA_ONLY: &str = "No file changes. This revision only updates metadata.";

fn files_words(n: usize) -> String {
    if n == 1 {
        "1 file".to_string()
    } else {
        format!("{n} files")
    }
}

fn ignored_words(n: usize) -> String {
    if n == 1 {
        "1 ignored file not included".to_string()
    } else {
        format!("{n} ignored files not included")
    }
}

/// What's included: the changed files and nothing else, then how many ignored
/// files are left out. No facets and no search: the list is what will be
/// published, and narrowing it would only hide some of that.
///
/// Last on the page, so however long it is the form above stays where it is.
#[component]
#[allow(
    clippy::needless_pass_by_value,
    reason = "a component's props are owned; the body reads them from there"
)]
pub fn IncludedList(
    files: Vec<IncludedFile>,
    /// Ignored files, which the revision leaves out.
    ignored: usize,
    /// `Ignore` from a row's `[⋯]`, with the row's path.
    on_ignore: Callback<String>,
) -> impl IntoView {
    let total: u64 = files.iter().map(|f| f.size).sum();
    let heading_id = crate::kit::unique_id("included");
    let labelled_by = heading_id.clone();
    let tally = (!files.is_empty())
        .then(|| format!(" · {} · {}", files_words(files.len()), format_size(total)));
    let rows = if files.is_empty() {
        view! { <p class=style::quiet>{METADATA_ONLY}</p> }.into_any()
    } else {
        view! {
            <Card flush=true list=true>
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
        }
        .into_any()
    };
    view! {
        <section class=style::included aria-labelledby=labelled_by>
            <h3 class=style::included_heading id=heading_id>
                "What's included"
                <span class=style::tally>{tally}</span>
            </h3>
            {rows}
            {(ignored > 0)
                .then(|| {
                    view! {
                        <p class=style::quiet>
                            {ignored_words(ignored)}
                        </p>
                    }
                })}
        </section>
    }
}

/// The page's column, in the order the reader confirms: header, problem,
/// message, workflow and metadata, then the files.
#[component]
pub fn CommitColumn(
    header: AnyView,
    #[prop(optional)] problem: Option<AnyView>,
    message: AnyView,
    workflow: AnyView,
    included: AnyView,
) -> impl IntoView {
    view! {
        <div class=style::column>
            {header}
            {problem}
            {message}
            {workflow}
            {included}
        </div>
    }
}

/// The page while `get_commit_data` is in flight: the header's shape, then
/// the message and the workflow line, in the column's own rhythm.
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
mod tests {
    use super::*;

    /// The same list `package_state.rs` holds the package states to, plus the
    /// words this page is likeliest to reach for.
    const BANNED: &[&str] = &[
        "commit", "commits", "push", "pull", "remote", "behind", "ahead", "diverged", "dirty",
        "hash",
    ];

    fn banned_in(words: &str) -> Option<&'static str> {
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
            "Workflow & metadata".to_string(),
            "What's included".to_string(),
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
