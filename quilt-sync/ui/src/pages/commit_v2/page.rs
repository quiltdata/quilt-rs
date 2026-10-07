//! The routed commit page v2: `get_commit_data` mapped onto the views above.
//!
//! The form is the page's, not the body's: an answer re-read after Ignore, or
//! after a failed publish, rebuilds the column, and what the reader typed must
//! survive that. It is seeded once per package from the first answer.
//!
//! Live validation is v1's — the rules load, the candidate check, the 400ms
//! debounce, the keyed answer — with one difference: every violation shows at
//! once and blocks the primary. This page is the review before a publish, so
//! a publish that would be refused says so before the click.

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use leptos::prelude::*;
use leptos_router::NavigateOptions;
use leptos_router::hooks::{use_navigate, use_query_map};

use crate::commands::{self, CommitData, CommitViolation, EntryData, ViolationField};
use crate::commands::{CommitWorkflows, WorkflowIntent};
use crate::components::appbar::appbar_actions;
use crate::components::build_workflow_view;
use crate::components::workflow_select::{
    WorkflowOption, WorkflowView, WorkflowViewKind, missing_settings_workflow,
    shows_settings_workflow_hint,
};
use crate::components::{IgnorePopup, IgnorePopupData, Notification, PreviousWorkflow};
use crate::kit::{Banner, BannerVariant, LoadFailure, PageLayout};
use crate::routes;

use super::super::commit::{
    effective_metadata, field_violations, should_debounce, starting_metadata,
};
use super::{
    Change, CommitColumn, CommitHeader, CommitPageSkeleton, IncludedFile, IncludedList,
    MessageField, MetadataField, Primary, PrimaryWiring, Problem, ProblemBanner, WorkflowChoice,
    WorkflowField,
};

/// The page's one read. A parameter so a test can answer without a Tauri host.
pub(crate) type CommitRead =
    fn(String) -> Pin<Box<dyn Future<Output = Result<CommitData, String>>>>;

fn read_commit(namespace: String) -> Pin<Box<dyn Future<Output = Result<CommitData, String>>>> {
    Box::pin(commands::get_commit_data(namespace))
}

/// What `/commit` renders when *New design preview* is on.
#[component]
pub fn CommitV2() -> impl IntoView {
    view! { <CommitScreen read=read_commit /> }
}

/// What `/commit` draws while it reads which page it is, for a v2 reader: the
/// page's own first paint.
#[component]
pub fn CommitV2Skeleton(actions: AnyView) -> impl IntoView {
    view! {
        <PageLayout heading="New revision" banner=().into_any() actions=actions>
            <CommitPageSkeleton />
        </PageLayout>
    }
}

// ── Mapping ──

/// The changed files, as the list draws them. Ignored and unchanged entries
/// are left out: the list is what the revision will carry.
pub(crate) fn included_files(entries: &[EntryData]) -> Vec<IncludedFile> {
    entries
        .iter()
        .filter(|e| e.ignored_by.is_none())
        .filter_map(|e| {
            let change = match e.status.as_str() {
                "added" => Change::New,
                "modified" => Change::Changed,
                "deleted" => Change::Deleted,
                _ => return None,
            };
            Some(IncludedFile {
                path: e.filename.clone(),
                size: e.size,
                change,
            })
        })
        .collect()
}

/// The primary's words: what the revision does, from the whole-package count.
pub(crate) const fn primary(has_bucket: bool, changed: usize) -> Primary {
    match (has_bucket, changed) {
        (false, _) => Primary::LocalOnly,
        (true, 0) => Primary::MetadataOnly,
        (true, n) => Primary::Files(n),
    }
}

/// The system files that would be published: the first one's path and the
/// pattern that matches it, for *Ignore them*, and every distinct file name,
/// for the banner's words.
pub(crate) fn junk(entries: &[EntryData]) -> Option<(String, String, usize, Vec<String>)> {
    let junky: Vec<_> = entries
        .iter()
        .filter(|e| e.ignored_by.is_none())
        .filter_map(|e| e.junky_pattern.as_ref().map(|p| (e, p)))
        .collect();
    let (first, pattern) = junky.first()?;
    let mut names: Vec<String> = Vec::new();
    for (e, _) in &junky {
        let name = e
            .filename
            .rsplit('/')
            .next()
            .unwrap_or(&e.filename)
            .to_string();
        if !names.contains(&name) {
            names.push(name);
        }
    }
    Some((
        first.filename.clone(),
        (*pattern).clone(),
        junky.len(),
        names,
    ))
}

/// The one problem the banner reports, worst first: a role that cannot write,
/// no session, then system files.
pub(crate) fn problem(
    no_access_reason: Option<&str>,
    no_session: bool,
    no_session_host: Option<&str>,
    entries: &[EntryData],
) -> Option<Problem> {
    if let Some(reason) = no_access_reason.filter(|r| !r.is_empty()) {
        return Some(Problem::NoAccess {
            reason: reason.to_string(),
        });
    }
    if no_session {
        return Some(Problem::SignedOut {
            host: no_session_host
                .filter(|h| !h.is_empty())
                .map(str::to_string),
        });
    }
    junk(entries).map(|(_, _, count, names)| Problem::Junk { count, names })
}

/// Why the primary cannot run, in its tooltip. A role or a session blocks
/// saving as well, because a save runs the workflow check, which reads the
/// bucket. A failed check blocks both too: the save would be refused alike.
pub(crate) fn blocked_reason(
    no_access_reason: Option<&str>,
    no_session: bool,
    no_session_host: Option<&str>,
    check: Option<&str>,
) -> Option<String> {
    if let Some(reason) = no_access_reason.filter(|r| !r.is_empty()) {
        return Some(format!("{reason}. Switch role to publish."));
    }
    if no_session {
        return Some(match no_session_host.filter(|h| !h.is_empty()) {
            Some(host) => format!("Sign in to {host} to publish."),
            None => "No usable AWS credentials. Update ~/.aws/credentials to publish.".to_string(),
        });
    }
    check.map(str::to_string)
}

/// What the message field submits: what was typed, or the placeholder, which
/// is the publish message, when nothing was.
pub(crate) fn effective_message(typed: &str, placeholder: &str) -> String {
    if typed.trim().is_empty() {
        placeholder.to_string()
    } else {
        typed.to_string()
    }
}

/// The workflow in words, when there is nothing to choose.
pub(crate) fn workflow_words(kind: &WorkflowViewKind, has_bucket: bool) -> String {
    match kind {
        _ if !has_bucket => "None — this package has no bucket yet".to_string(),
        WorkflowViewKind::NotConfigured => "None".to_string(),
        WorkflowViewKind::Unavailable => {
            "The bucket's default. Its workflows could not be loaded.".to_string()
        }
        WorkflowViewKind::Invalid { reason } => {
            format!("The bucket's workflows file is not valid: {reason}")
        }
        WorkflowViewKind::Available { .. } => String::new(),
    }
}

/// Whether the address asks for the message to be focused: *Create new
/// revision*'s link does, every other way in does not.
pub(crate) fn focuses_message(focus: Option<&str>) -> bool {
    focus == Some("message")
}

/// The warning under the workflow when the publish settings name one this
/// bucket cannot use, which one-click Publish would send and be refused.
pub(crate) fn missing_workflow_note(id: &str) -> String {
    format!(
        "Your default workflow in Settings, \"{id}\", isn't one of this bucket's workflows, \
         so Publish would fail here."
    )
}

/// The violations under one field, as one caption.
fn caption(violations: &[CommitViolation], field: ViolationField) -> Option<String> {
    let messages = field_violations(violations, field);
    (!messages.is_empty()).then(|| messages.join(" "))
}

/// The form's fixed facts, from one answer: what the page validates and
/// submits against. Compared, so a re-read that changed nothing here does not
/// re-run validation.
#[derive(Clone, Debug, PartialEq)]
struct Form {
    namespace: String,
    placeholder: String,
    previous_meta: String,
    settings_meta: Option<String>,
    settings_workflow: Option<String>,
    /// The choosable options, in order; a disabled one (`None` on a bucket
    /// that requires a workflow) is left out, as the select cannot grey one.
    options: Vec<WorkflowOption>,
    initial: String,
    choosable: bool,
    words: String,
    missing_settings_workflow: Option<String>,
}

impl Form {
    fn of(d: &CommitData) -> Self {
        let has_bucket = d.uri.as_ref().is_some_and(|u| u.catalog.is_some());
        let previous = PreviousWorkflow::from_stamp(d.workflow.as_ref());
        let view: WorkflowView = build_workflow_view(
            &d.workflows,
            previous.preselect_id(),
            d.settings_workflow.as_deref(),
        );
        let initial = view
            .options
            .get(view.initial)
            .map(|o| o.label.clone())
            .unwrap_or_default();
        let missing =
            missing_settings_workflow(&view, d.settings_workflow.as_deref()).map(str::to_string);
        let choosable = matches!(d.workflows, CommitWorkflows::Available { .. });
        Self {
            namespace: d.namespace.to_string(),
            placeholder: d.message.clone(),
            previous_meta: d.user_meta.clone(),
            settings_meta: d.settings_user_meta.clone(),
            settings_workflow: d.settings_workflow.clone(),
            words: workflow_words(&view.kind, has_bucket),
            options: view.options.into_iter().filter(|o| !o.disabled).collect(),
            initial,
            choosable,
            missing_settings_workflow: missing,
        }
    }

    fn intent(&self, label: &str) -> WorkflowIntent {
        self.options
            .iter()
            .find(|o| o.label == label)
            .map_or(WorkflowIntent::BucketDefault, |o| o.intent.clone())
    }
}

/// What the reader is writing. The page's, so a re-read keeps it.
#[derive(Clone, Copy)]
struct Draft {
    message: RwSignal<String>,
    metadata: RwSignal<String>,
    workflow: RwSignal<String>,
    editing: RwSignal<bool>,
    /// The package the draft was seeded for.
    seeded: StoredValue<Option<String>>,
}

impl Draft {
    fn new() -> Self {
        Self {
            message: RwSignal::new(String::new()),
            metadata: RwSignal::new(String::new()),
            workflow: RwSignal::new(String::new()),
            editing: RwSignal::new(false),
            seeded: StoredValue::new(None),
        }
    }

    /// Start from the answer's values, once per package.
    fn seed(self, d: &CommitData, form: &Form) {
        if self.seeded.get_value().as_deref() == Some(form.namespace.as_str()) {
            return;
        }
        self.seeded.set_value(Some(form.namespace.clone()));
        self.message.set(String::new());
        self.metadata.set(starting_metadata(
            d.settings_user_meta.as_deref(),
            &d.user_meta,
        ));
        self.workflow.set(form.initial.clone());
        self.editing.set(false);
    }
}

/// What the last command said on this page.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Said {
    lead: String,
    detail: Option<String>,
}

// ── The page ──

#[component]
#[allow(
    clippy::too_many_lines,
    reason = "the page's wiring in one place: the read, the draft, validation and the commands"
)]
fn CommitScreen(read: CommitRead) -> impl IntoView {
    let query = use_query_map();
    let ns = Memo::new(move |_| query.read().get("namespace").unwrap_or_default());
    let focus = query
        .read_untracked()
        .get("focus")
        .is_some_and(|f| focuses_message(Some(&f)));

    let reload = Trigger::new();
    let in_flight = RwSignal::new(false);
    let data = LocalResource::new(move || {
        reload.track();
        let namespace = ns.get();
        async move {
            in_flight.set(true);
            let answer = read(namespace.clone()).await;
            in_flight.try_set(false);
            (namespace, answer)
        }
    });
    let answer = move || {
        data.get()
            .filter(|(for_ns, _)| *for_ns == ns.get())
            .map(|(_, answer)| answer)
    };

    let draft = Draft::new();
    let form = Memo::new(move |_| match answer() {
        Some(Ok(d)) => Some(Form::of(&d)),
        _ => None,
    });
    let running = RwSignal::new(false);
    let said = RwSignal::new(None::<Said>);
    let ignoring = RwSignal::new(None::<IgnorePopupData>);
    let goto = RwSignal::new(None::<String>);
    let navigate = use_navigate();
    Effect::new(move |_| {
        if let Some(to) = goto.get() {
            navigate(&to, NavigateOptions::default());
        }
    });

    // ── Live validation, v1's ──
    let workflow_id = Memo::new(move |_| {
        let label = draft.workflow.get();
        form.with(|f| match f.as_ref().map(|f| f.intent(&label)) {
            Some(WorkflowIntent::Named(id)) => Some(id),
            _ => None,
        })
    });
    let live_key = Memo::new(move |_| {
        let placeholder = form.with(|f| f.as_ref().map(|f| f.placeholder.clone()));
        (
            effective_message(&draft.message.get(), &placeholder.unwrap_or_default()),
            draft.metadata.get(),
            workflow_id.get(),
        )
    });
    let debounced = RwSignal::new(live_key.get_untracked());
    let timer: StoredValue<Option<TimeoutHandle>> = StoredValue::new(None);
    Effect::new(move |_| {
        let key = live_key.get();
        if !should_debounce(&key, &debounced.get_untracked()) {
            return;
        }
        if let Some(handle) = timer.get_value() {
            handle.clear();
        }
        if let Ok(handle) =
            set_timeout_with_handle(move || debounced.set(key), Duration::from_millis(400))
        {
            timer.set_value(Some(handle));
        }
    });
    on_cleanup(move || {
        if let Some(Some(handle)) = timer.try_get_value() {
            handle.clear();
        }
    });
    // The first load of this visit refreshes the backend's rules cache, so a
    // config changed since the last visit is read again.
    let first_load = StoredValue::new(true);
    let validation = LocalResource::new(move || {
        let key = debounced.get();
        let fixed = form.get();
        async move {
            let (message, metadata, workflow_id) = key.clone();
            let (Some(id), Some(fixed)) = (workflow_id, fixed) else {
                return (key, Vec::new());
            };
            let refresh = first_load.try_get_value().unwrap_or(false);
            first_load.try_set_value(false);
            let _ =
                commands::load_workflow_rules(fixed.namespace.clone(), id.clone(), refresh).await;
            // An emptied editor keeps the previous revision's metadata, so
            // that is what the check sees, as the commit gate does.
            let metadata = effective_metadata(&metadata, &fixed.previous_meta);
            let violations = commands::validate_commit_candidate(
                fixed.namespace.clone(),
                id,
                message,
                metadata,
                fixed.namespace,
            )
            .await
            .unwrap_or_default();
            (key, violations)
        }
    });
    let violations = Memo::new(move |_| match validation.get() {
        Some((key, violations)) if key == live_key.get() => violations,
        _ => Vec::new(),
    });

    // ── Commands ──
    let run = Callback::new(move |publish: bool| {
        if running.get_untracked() {
            return;
        }
        let Some(Ok(d)) = answer() else { return };
        let Some(fixed) = form.get_untracked() else {
            return;
        };
        let message = effective_message(&draft.message.get_untracked(), &fixed.placeholder);
        let metadata = draft.metadata.get_untracked();
        let workflow = fixed.intent(&draft.workflow.get_untracked());
        let (namespace, uri) = (d.namespace.clone(), d.uri.clone());
        running.set(true);
        said.set(None);
        leptos::task::spawn_local(async move {
            let ns = namespace.to_string();
            let result = if publish {
                commands::package_commit_and_push(ns, message, metadata, workflow, uri).await
            } else {
                commands::package_commit(ns, message, metadata, workflow, uri).await
            };
            match result {
                // The command's own notification reports it; arriving is the rest.
                Ok(_) => {
                    goto.try_set(Some(routes::package_page_href(&namespace)));
                }
                Err(detail) => {
                    running.try_set(false);
                    said.try_set(Some(Said {
                        lead: if publish {
                            "Could not publish this revision.".to_string()
                        } else {
                            "Could not save this revision.".to_string()
                        },
                        detail: Some(detail),
                    }));
                    // A publish can save the revision and then fail to send
                    // it; the page has to show what now exists.
                    reload.notify();
                }
            }
        });
    });
    let open_folder = Callback::new(move |()| {
        let Some(Ok(d)) = answer() else { return };
        leptos::task::spawn_local(async move {
            if let Err(detail) =
                commands::open_in_file_browser(d.namespace.to_string(), d.uri).await
            {
                said.try_set(Some(Said {
                    lead: "Could not open the folder.".to_string(),
                    detail: Some(detail),
                }));
            }
        });
    });

    // The ignore popup reports through a `Notification`; its failure is this
    // page's news, and its success re-reads the page, which is the report.
    let popup_said = RwSignal::new(None::<Notification>);
    Effect::new(move |_| {
        if let Some(Notification::Error(detail)) = popup_said.get() {
            said.set(Some(Said {
                lead: "Could not ignore this file.".to_string(),
                detail: Some(detail),
            }));
        }
    });

    let body = move || match answer() {
        None => view! { <CommitPageSkeleton /> }.into_any(),
        Some(Err(_)) => view! {
            <LoadFailure
                words="Could not load this package."
                on_retry=Callback::new(move |()| reload.notify())
            />
        }
        .into_any(),
        Some(Ok(d)) => {
            let Some(fixed) = form.get_untracked() else {
                return ().into_any();
            };
            untrack(|| draft.seed(&d, &fixed));
            column(
                &d,
                fixed,
                draft,
                violations,
                running,
                run,
                open_folder,
                ignoring,
                focus,
            )
        }
    };

    view! {
        {move || {
            ignoring
                .get()
                .map(|data| {
                    view! {
                        <IgnorePopup
                            data=data
                            notification=popup_said
                            refetch=reload
                            on_close=move || ignoring.set(None)
                        />
                    }
                })
        }}
        <PageLayout
            heading="New revision"
            banner=view! {
                {move || {
                    said.get()
                        .map(|news| {
                            view! {
                                <Banner
                                    variant=BannerVariant::Critical
                                    on_dismiss=Callback::new(move |_| said.set(None))
                                >
                                    {news.lead}
                                    {news.detail.map(|d| format!(" {d}"))}
                                </Banner>
                            }
                        })
                }}
            }
                .into_any()
            actions=appbar_actions(move || reload.notify(), in_flight.into())
        >
            {body}
        </PageLayout>
    }
}

/// The answered page: the views over one answer and the page's draft.
#[allow(
    clippy::too_many_arguments,
    clippy::needless_pass_by_value,
    reason = "the page's signals, each its own; the form is moved into the views"
)]
fn column(
    d: &CommitData,
    fixed: Form,
    draft: Draft,
    violations: Memo<Vec<CommitViolation>>,
    running: RwSignal<bool>,
    run: Callback<bool>,
    open_folder: Callback<()>,
    ignoring: RwSignal<Option<IgnorePopupData>>,
    focus: bool,
) -> AnyView {
    let has_bucket = d.uri.as_ref().is_some_and(|u| u.catalog.is_some());
    let namespace = d.namespace.to_string();
    let uri = d.uri.clone();

    let (no_access, no_session) = (d.no_access_reason.clone(), d.no_session);
    let host = d.no_session_host.clone();
    let blocked = Signal::derive(move || {
        let check = violations.with(|v| v.first().map(|v| v.message.clone()));
        blocked_reason(
            no_access.as_deref(),
            no_session,
            host.as_deref(),
            check.as_deref(),
        )
    });
    let w = PrimaryWiring {
        choice: RwSignal::new(0),
        on_publish: Callback::new(move |()| run.run(true)),
        on_save: Callback::new(move |()| run.run(false)),
        blocked,
        running: running.into(),
    };

    let problem = problem_banner(d, ignoring);

    let message_error =
        Signal::derive(move || violations.with(|v| caption(v, ViolationField::Message)));
    let message = view! {
        <div data-commit-message>
            <MessageField
                value=draft.message
                placeholder=fixed.placeholder.clone()
                error=message_error
                autofocus=focus
            />
        </div>
    }
    .into_any();

    let workflow = workflow_field(&fixed, draft, violations);

    let metadata = metadata_field(d, &fixed, draft, violations);

    let on_ignore = {
        let (namespace, uri) = (namespace.clone(), uri.clone());
        Callback::new(move |path: String| {
            ignoring.set(Some(IgnorePopupData {
                namespace: namespace.clone(),
                suggested_pattern: path.clone(),
                path,
                uri: uri.clone(),
            }));
        })
    };
    let included = view! {
        <IncludedList
            files=included_files(&d.entries)
            ignored=d.counts.ignored
            on_ignore=on_ignore
            totals=(d.counts.changed, d.changed_bytes)
        />
    }
    .into_any();

    let header = view! {
        <CommitHeader
            namespace=namespace.clone()
            package_href=routes::package_page_href(&d.namespace)
            primary=primary(has_bucket, d.counts.changed)
            w=w
            on_open_folder=open_folder
        />
    }
    .into_any();

    let focus_ref = NodeRef::<leptos::html::Div>::new();
    if focus {
        focus_message(focus_ref);
    }

    view! {
        <div node_ref=focus_ref>
            <CommitColumn
                header=header
                problem=problem.unwrap_or_else(|| ().into_any())
                message=message
                workflow=workflow
                metadata=metadata
                included=included
            />
        </div>
    }
    .into_any()
}

/// The metadata field: the preview or the editor, with the check's caption
/// and the settings line while the draft is still the Settings value.
fn metadata_field(
    d: &CommitData,
    fixed: &Form,
    draft: Draft,
    violations: Memo<Vec<CommitViolation>>,
) -> AnyView {
    let user_meta_error = d.user_meta_error.clone();
    let metadata_error = Signal::derive(move || {
        violations
            .with(|v| caption(v, ViolationField::Metadata))
            .or_else(|| user_meta_error.clone())
    });
    let settings_meta = fixed.settings_meta.clone();
    let metadata_from_settings = Signal::derive(move || {
        settings_meta
            .as_deref()
            .is_some_and(|m| draft.metadata.with(|now| now == m))
    });
    view! {
        <MetadataField
            metadata=draft.metadata
            editing=draft.editing
            error=metadata_error
            from_settings=metadata_from_settings
        />
    }
    .into_any()
}

/// Put the cursor in the message once the column is mounted.
fn focus_message(focus_ref: NodeRef<leptos::html::Div>) {
    // `autofocus` is honoured only once per document, and this page is
    // routed into one that already has focus, so it is asked for again.
    request_animation_frame(move || {
        let input = focus_ref
            .get_untracked()
            .and_then(|el| {
                el.query_selector("[data-commit-message] input")
                    .ok()
                    .flatten()
            })
            .and_then(|el| wasm_bindgen::JsCast::dyn_into::<web_sys::HtmlElement>(el).ok());
        if let Some(input) = input {
            drop(input.focus());
        }
    });
}

/// The problems banner, with *Ignore them* opening the ignore popup on the
/// first system file and its pattern.
fn problem_banner(d: &CommitData, ignoring: RwSignal<Option<IgnorePopupData>>) -> Option<AnyView> {
    let junk_to_ignore = junk(&d.entries);
    let (namespace, uri) = (d.namespace.to_string(), d.uri.clone());
    problem(
        d.no_access_reason.as_deref(),
        d.no_session,
        d.no_session_host.as_deref(),
        &d.entries,
    )
    .map(|problem| {
        let ignore = {
            let (namespace, uri) = (namespace.clone(), uri.clone());
            Callback::new(move |()| {
                if let Some((path, pattern, _, _)) = junk_to_ignore.clone() {
                    ignoring.set(Some(IgnorePopupData {
                        namespace: namespace.clone(),
                        path,
                        suggested_pattern: pattern,
                        uri: uri.clone(),
                    }));
                }
            })
        };
        match &problem {
            Problem::SignedOut { host: Some(host) } => {
                let href = routes::sign_in_href(host);
                view! { <ProblemBanner problem=problem on_ignore_junk=ignore sign_in_href=href /> }
                    .into_any()
            }
            _ => view! { <ProblemBanner problem=problem on_ignore_junk=ignore /> }.into_any(),
        }
    })
}

/// The workflow field: the select, or its words, with the check's caption,
/// the settings line and the missing-workflow warning.
fn workflow_field(fixed: &Form, draft: Draft, violations: Memo<Vec<CommitViolation>>) -> AnyView {
    // The package name is fixed here; a workflow that rejects it says so under
    // the workflow, the field that brought the rule.
    let workflow_error =
        Signal::derive(move || violations.with(|v| caption(v, ViolationField::Name)));
    let note = fixed
        .missing_settings_workflow
        .as_deref()
        .map(missing_workflow_note);
    {
        let hinted = fixed.clone();
        let from_settings = Signal::derive(move || {
            let intent = hinted.intent(&draft.workflow.get());
            hinted.choosable
                && shows_settings_workflow_hint(Some(&intent), hinted.settings_workflow.as_deref())
        });
        let choice = (fixed.choosable && fixed.options.len() > 1).then(|| WorkflowChoice {
            options: fixed.options.iter().map(|o| o.label.clone()).collect(),
            selected: draft.workflow,
        });
        let words = if fixed.choosable {
            fixed
                .options
                .first()
                .map(|o| o.label.clone())
                .unwrap_or_default()
        } else {
            fixed.words.clone()
        };
        match note {
            Some(note) => view! {
                <WorkflowField
                    workflow=choice
                    no_workflow=words
                    error=workflow_error
                    from_settings=from_settings
                    note=note
                />
            }
            .into_any(),
            None => view! {
                <WorkflowField
                    workflow=choice
                    no_workflow=words
                    error=workflow_error
                    from_settings=from_settings
                />
            }
            .into_any(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{mount, sleep_ms, unmount_earlier};
    use leptos_router::components::{Route, Router, Routes};
    use leptos_router::path;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::*;

    /// A package past the list's cap: three rows sent, 1203 changes counted.
    fn commit_data() -> CommitData {
        let mut rows = vec![
            entry("a.csv", "added"),
            entry("b.csv", "modified"),
            entry("c.csv", "deleted"),
        ];
        rows.push(entry("d.csv", "pristine"));
        CommitData {
            namespace: "org/pkg".try_into().unwrap(),
            uri: Some(
                quilt_uri::S3PackageUri::try_from(
                    "quilt+s3://team-bucket#package=org/pkg&catalog=quilt.test",
                )
                .unwrap(),
            ),
            status: "up_to_date".to_string(),
            message: "Updated 1203 files".to_string(),
            user_meta: "{\"plate\": 7}".to_string(),
            user_meta_error: None,
            settings_user_meta: None,
            settings_workflow: None,
            has_message_template: false,
            workflow: None,
            workflows: CommitWorkflows::NotConfigured,
            no_access_reason: None,
            no_session: false,
            no_session_host: None,
            entries: rows,
            ignored_count: 2,
            unmodified_count: 1,
            counts: commands::EntryCounts {
                all: 1204,
                changed: 1203,
                not_downloaded: 0,
                ignored: 2,
                deleted: 1,
            },
            changed_bytes: 2_500_000,
        }
    }

    fn answered(_: String) -> Pin<Box<dyn Future<Output = Result<CommitData, String>>>> {
        Box::pin(async { Ok(commit_data()) })
    }

    async fn page_at(address: &str) -> web_sys::Element {
        unmount_earlier();
        web_sys::window()
            .unwrap()
            .history()
            .unwrap()
            .replace_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some(address))
            .unwrap();
        let el = mount(|| {
            view! {
                <Router>
                    <Routes fallback=|| view! { "no route" }>
                        <Route path=path!("/commit") view=|| view! { <CommitScreen read=answered /> } />
                    </Routes>
                </Router>
            }
        });
        sleep_ms(80).await;
        el
    }

    fn message_input(el: &web_sys::Element) -> web_sys::HtmlInputElement {
        el.query_selector("[data-commit-message] input")
            .unwrap()
            .expect("the message field")
            .dyn_into()
            .unwrap()
    }

    /// The primary and the list heading count the whole package, not the
    /// three rows the capped list sent.
    #[wasm_bindgen_test]
    async fn the_page_counts_every_change_not_the_rows() {
        let el = page_at("/commit?namespace=org%2Fpkg").await;
        let text = el.text_content().unwrap();
        assert!(
            text.contains("Publish 1203 files"),
            "markup was {}",
            el.inner_html()
        );
        assert!(
            text.contains("1203 files · 2.5\u{a0}MB"),
            "markup was {}",
            el.inner_html()
        );
        assert!(text.contains("2 ignored files not included"));
        // The message starts empty, over the publish message.
        let input = message_input(&el);
        assert_eq!(input.value(), "");
        assert_eq!(input.placeholder(), "Updated 1203 files");
    }

    /// *Create new revision* arrives with the message focused; *Review
    /// before publishing…* arrives with nothing focused.
    #[wasm_bindgen_test]
    async fn focus_follows_how_the_reader_arrived() {
        let el = page_at("/commit?namespace=org%2Fpkg&focus=message").await;
        let active = web_sys::window()
            .unwrap()
            .document()
            .unwrap()
            .active_element();
        assert_eq!(
            active,
            Some(message_input(&el).unchecked_into()),
            "the message is focused"
        );

        let el = page_at("/commit?namespace=org%2Fpkg").await;
        let active = web_sys::window()
            .unwrap()
            .document()
            .unwrap()
            .active_element();
        assert_ne!(active, Some(message_input(&el).unchecked_into()));
    }

    fn entry(path: &str, status: &str) -> EntryData {
        EntryData {
            filename: path.to_string(),
            size: 10,
            status: status.to_string(),
            junky_pattern: None,
            ignored_by: None,
            namespace: quilt_uri::Namespace::try_from("org/pkg").unwrap(),
        }
    }

    #[test]
    fn the_list_is_the_changed_files_in_the_page_s_words() {
        let mut ignored = entry("tmp/x.tmp", "pristine");
        ignored.ignored_by = Some("*.tmp".to_string());
        let files = included_files(&[
            entry("a.csv", "added"),
            entry("b.csv", "modified"),
            entry("c.csv", "deleted"),
            entry("d.csv", "pristine"),
            entry("e.csv", "remote"),
            ignored,
        ]);
        let shown: Vec<_> = files.iter().map(|f| (f.path.as_str(), f.change)).collect();
        assert_eq!(
            shown,
            [
                ("a.csv", Change::New),
                ("b.csv", Change::Changed),
                ("c.csv", Change::Deleted)
            ]
        );
    }

    #[test]
    fn the_primary_says_what_the_revision_does() {
        assert_eq!(primary(true, 1203), Primary::Files(1203));
        assert_eq!(primary(true, 0), Primary::MetadataOnly);
        assert_eq!(primary(false, 4), Primary::LocalOnly);
        assert_eq!(primary(false, 0), Primary::LocalOnly);
    }

    #[test]
    fn an_empty_message_submits_the_placeholder() {
        assert_eq!(effective_message("", "Updated 3 files"), "Updated 3 files");
        assert_eq!(
            effective_message("   ", "Updated 3 files"),
            "Updated 3 files"
        );
        assert_eq!(effective_message("Plate 7", "Updated 3 files"), "Plate 7");
    }

    #[test]
    fn only_create_new_revision_focuses_the_message() {
        assert!(focuses_message(Some("message")));
        assert!(!focuses_message(None));
        assert!(!focuses_message(Some("other")));
    }

    #[test]
    fn the_banner_reports_the_worst_problem() {
        let mut junky = entry("raw/.DS_Store", "added");
        junky.junky_pattern = Some(".DS_Store".to_string());
        let mut junky2 = entry("notes/.DS_Store", "added");
        junky2.junky_pattern = Some(".DS_Store".to_string());
        let entries = [junky, junky2, entry("a.csv", "added")];
        assert_eq!(
            problem(None, false, None, &entries),
            Some(Problem::Junk {
                count: 2,
                names: vec![".DS_Store".to_string()],
            })
        );
        assert_eq!(
            problem(None, true, Some("quilt.test"), &entries),
            Some(Problem::SignedOut {
                host: Some("quilt.test".to_string())
            })
        );
        assert_eq!(
            problem(Some("Your role cannot write here"), true, None, &entries),
            Some(Problem::NoAccess {
                reason: "Your role cannot write here".to_string()
            })
        );
        assert_eq!(problem(None, false, None, &[entry("a.csv", "added")]), None);
        assert_eq!(
            junk(&entries).map(|(path, pattern, ..)| (path, pattern)),
            Some(("raw/.DS_Store".to_string(), ".DS_Store".to_string()))
        );
    }

    #[test]
    fn a_blocked_primary_says_why_and_a_failed_check_blocks_it() {
        assert_eq!(
            blocked_reason(Some("Role analyst has no access"), false, None, None).as_deref(),
            Some("Role analyst has no access. Switch role to publish.")
        );
        assert_eq!(
            blocked_reason(None, true, Some("quilt.test"), None).as_deref(),
            Some("Sign in to quilt.test to publish.")
        );
        assert_eq!(
            blocked_reason(None, false, None, Some("Missing operator")).as_deref(),
            Some("Missing operator")
        );
        assert_eq!(blocked_reason(None, false, None, None), None);
    }

    /// `commit_v2`'s banned words, over every string this page adds.
    #[test]
    fn no_page_label_uses_a_banned_word() {
        let mut all = vec![
            "Could not publish this revision.".to_string(),
            "Could not save this revision.".to_string(),
            "Could not open the folder.".to_string(),
            "Could not ignore this file.".to_string(),
            "Could not load this package.".to_string(),
            missing_workflow_note("wrong-workflow"),
            blocked_reason(Some("No access"), false, None, None).unwrap(),
            blocked_reason(None, true, Some("quilt.test"), None).unwrap(),
            blocked_reason(None, true, None, None).unwrap(),
        ];
        for kind in [
            WorkflowViewKind::NotConfigured,
            WorkflowViewKind::Unavailable,
            WorkflowViewKind::Invalid {
                reason: "bad yaml".to_string(),
            },
        ] {
            all.push(workflow_words(&kind, true));
        }
        all.push(workflow_words(&WorkflowViewKind::NotConfigured, false));
        for words in &all {
            assert_eq!(super::super::tests::banned_in(words), None, "{words:?}");
        }
    }
}
