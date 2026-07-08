use leptos::prelude::*;
use leptos_router::hooks::{use_navigate, use_query_map};

use quilt_uri::S3PackageUri;

use crate::commands::{self, CommitData, CommitWorkflows, EntryData, WorkflowInfo, WorkflowIntent};
use crate::components::buttons;
use crate::components::layout::{BreadcrumbItem, BreadcrumbLink};
use crate::components::{
    IgnorePopup, IgnorePopupData, Layout, Notification, Spinner, ToolbarActions, UnignorePopup,
    UnignorePopupData,
};
use crate::util;
use crate::util::format_size;

// ── Commit page ──

#[component]
pub fn Commit() -> impl IntoView {
    let notification = RwSignal::new(None);
    let ui_locked = RwSignal::new(false);
    let refetch = Trigger::new();

    let query = use_query_map();
    let data = LocalResource::new(move || {
        refetch.track();
        let namespace = query.read().get("namespace").unwrap_or_default();
        async move { commands::get_commit_data(namespace).await }
    });

    view! {
        <Suspense fallback=move || {
            view! {
                <Layout breadcrumbs=vec![] notification=notification ui_locked=ui_locked>
                    <Spinner />
                </Layout>
            }
        }>
            {move || Suspend::new(async move {
                match data.await {
                    Ok(d) => {
                        let ns = d.namespace.clone();
                        let pkg_href = format!(
                            "/installed-package?namespace={}&filter=unmodified",
                            d.namespace
                        );
                        let breadcrumbs = vec![
                            BreadcrumbItem::Link(BreadcrumbLink {
                                href: "/installed-packages-list".to_string(),
                                title: String::new(),
                            }),
                            BreadcrumbItem::Link(BreadcrumbLink {
                                href: pkg_href,
                                title: ns.clone(),
                            }),
                            BreadcrumbItem::Current("Commit".to_string()),
                        ];
                        let actions = build_toolbar_actions(&d, notification, ui_locked);
                        view! {
                            <Layout breadcrumbs=breadcrumbs notification=notification actions=actions ui_locked=ui_locked>
                                <CommitContent data=d notification=notification ui_locked=ui_locked refetch=refetch />
                            </Layout>
                        }
                            .into_any()
                    }
                    Err(e) => {
                        crate::error_handler::handle_or_display(&e, notification)
                    }
                }
            })}
        </Suspense>
    }
}

// ── Main content ──

#[component]
fn CommitContent(
    data: CommitData,
    notification: RwSignal<Option<Notification>>,
    ui_locked: RwSignal<bool>,
    refetch: Trigger,
) -> impl IntoView {
    let navigate = use_navigate();
    let filter_unmodified = RwSignal::new(false);
    let filter_ignored = RwSignal::new(false);
    let show_ignore_popup = RwSignal::new(None::<IgnorePopupData>);
    let show_unignore_popup = RwSignal::new(None::<UnignorePopupData>);

    let namespace = data.namespace.clone();
    let message = RwSignal::new(data.message.clone());
    let user_meta = data.user_meta.clone();
    let user_meta_for_editor = data.user_meta.clone();
    let user_meta_error = data.user_meta_error.clone();
    let entries = data.entries;
    let ignored_count = data.ignored_count;
    let unmodified_count = data.unmodified_count;

    // Workflow state: the backend's `CommitWorkflows` state maps to a render
    // model whose option list carries, per entry, the `WorkflowIntent` it
    // submits. The selected index into that list is the whole client-side
    // state.
    let previous_workflow_id = data.workflow.as_ref().and_then(|w| w.id.clone());
    let wf_view = build_workflow_view(&data.workflows, previous_workflow_id.as_deref());
    // `wf_view.initial` is the single source of truth for the starting
    // selection: it both seeds this signal (which submit reads) and is passed
    // to `WorkflowSection` to render the `selected` attribute, so display and
    // submit start from the same index into the same option list.
    let initial_workflow = wf_view.initial;
    let selected_workflow = RwSignal::new(initial_workflow);
    // Intents mirrored for the submit path — the selected option's intent is
    // passed straight through, so `Named("")` can never be constructed. On the
    // NotConfigured/Unavailable states this is a single `BucketDefault` entry.
    let workflow_intents: Vec<WorkflowIntent> =
        wf_view.options.iter().map(|o| o.intent.clone()).collect();

    // Filtered entries
    let entries_for_view = entries.clone();
    let filtered_entries = Memo::new(move |_| {
        entries_for_view
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                if e.ignored_by.is_some() {
                    return filter_ignored.get();
                }
                if e.status == "pristine" || e.status == "remote" {
                    return filter_unmodified.get();
                }
                true
            })
            .map(|(i, e)| (i, e.clone()))
            .collect::<Vec<_>>()
    });

    let show_filter = ignored_count > 0 || unmodified_count > 0;

    // Whether this package has a remote we can publish to. When there is no
    // resolvable origin, the `[Commit and Push]` primary is hidden and only
    // `[Commit]` is shown.
    let has_remote = data.uri.as_ref().is_some_and(|u| u.catalog.is_some());

    let ns_for_action = namespace.clone();
    let committing = RwSignal::new(false);
    let navigate_for_action = navigate.clone();
    // `action` decides whether we do a plain commit or a commit-and-push.
    let run_action = std::rc::Rc::new(move |push: bool| {
        let navigate = navigate_for_action.clone();
        if committing.get_untracked() {
            return;
        }
        let msg = message.get_untracked();
        if msg.trim().is_empty() {
            return;
        }
        committing.set(true);
        ui_locked.set(true);
        let ns = ns_for_action.clone();
        let meta = get_json_editor_value("metadata-editor");
        let wf = workflow_intents
            .get(selected_workflow.get_untracked())
            .cloned()
            .unwrap_or(WorkflowIntent::BucketDefault);
        leptos::task::spawn_local(async move {
            let result = if push {
                commands::package_commit_and_push(ns.clone(), msg, meta, wf).await
            } else {
                commands::package_commit(ns.clone(), msg, meta, wf).await
            };
            match result {
                Ok(msg) => {
                    notification.set(Some(Notification::Success(msg)));
                    navigate(
                        &format!("/installed-package?namespace={ns}&filter=unmodified"),
                        Default::default(),
                    );
                }
                Err(e) => {
                    ui_locked.set(false);
                    notification.set(Some(Notification::Error(e)));
                    committing.set(false);
                }
            }
        });
    });

    let run_commit_only = {
        let run = run_action.clone();
        move |_| run(false)
    };
    let run_commit_and_push = {
        let run = run_action.clone();
        move |_| run(true)
    };

    view! {
        <div class="qui-page-commit container container-wide">
            // ── Left column: form ──
            <div class="main">
                <div class="form">
                    // ── Workflow ──
                    <WorkflowSection
                        view=wf_view
                        selected=selected_workflow
                    />

                    // ── Namespace (readonly) ──
                    <p class="field">
                        <label class="label" for="namespace">"Name"</label>
                        <input
                            class="input"
                            id="namespace"
                            name="namespace"
                            readonly
                            prop:value=namespace.clone()
                        />
                    </p>

                    // ── Message ──
                    <p class="field">
                        <label class="label" for="message">"Message"</label>
                        <input
                            autofocus
                            class="input"
                            id="message"
                            name="message"
                            required
                            prop:value=move || message.get()
                            on:input=move |ev| message.set(event_target_value(&ev))
                        />
                    </p>

                    // ── Metadata (textarea + JSON editor) ──
                    <p class="field">
                        <label class="label" for="metadata">"User metadata"</label>
                        <textarea
                            class="textarea"
                            id="metadata"
                            name="metadata"
                            placeholder="{ \"key\": \"value\" }"
                        >
                            {user_meta}
                        </textarea>
                        {user_meta_error.map(|err| view! {
                            <span class="error">{err}</span>
                        })}
                    </p>
                    <JsonEditor id="metadata-editor" initial_value=user_meta_for_editor />
                </div>
            </div>

            // ── Right column: file list ──
            <div class="files">
                <div class="list">
                    <div>
                        <Show when=move || show_filter>
                            <div class="qui-entries-filter">
                                <span>"Show"</span>
                                <label>
                                    <input
                                        type="checkbox"
                                        prop:checked=move || filter_unmodified.get()
                                        on:change=move |_| {
                                            filter_unmodified.set(!filter_unmodified.get_untracked());
                                        }
                                    />
                                    "unmodified"
                                    <Show when=move || !filter_unmodified.get() && (unmodified_count > 0)>
                                        <span class="qui-filter-count">
                                            {format!("({unmodified_count})")}
                                        </span>
                                    </Show>
                                </label>
                                <label>
                                    <input
                                        type="checkbox"
                                        prop:checked=move || filter_ignored.get()
                                        on:change=move |_| {
                                            filter_ignored.set(!filter_ignored.get_untracked());
                                        }
                                    />
                                    "ignored"
                                    <Show when=move || !filter_ignored.get() && (ignored_count > 0)>
                                        <span class="qui-filter-count">
                                            {format!("({ignored_count})")}
                                        </span>
                                    </Show>
                                </label>
                            </div>
                            {(ignored_count > 0 || unmodified_count > 0).then(|| view! {
                                <div class="list-separator"></div>
                            })}
                        </Show>

                        <For
                            each=move || filtered_entries.get()
                            key=|(i, _)| *i
                            let:item
                        >
                            <CommitEntryRow
                                entry=item.1
                                pkg_uri=data.uri.clone()
                                notification=notification
                                show_ignore_popup=show_ignore_popup
                                show_unignore_popup=show_unignore_popup
                            />
                        </For>
                    </div>
                </div>
            </div>
        </div>

        // ── Action bar ──
        <div class="qui-actionbar">
            <buttons::CommitRevision
                on_click=run_commit_only
                busy=committing
                disabled=Signal::derive(move || message.get().trim().is_empty())
                primary=!has_remote
            />
            {has_remote.then(|| view! {
                <span class="actions-divider">"or"</span>
                <buttons::CommitAndPush
                    on_click=run_commit_and_push
                    busy=committing
                    disabled=Signal::derive(move || message.get().trim().is_empty())
                />
            })}
        </div>

        // ── Popups ──
        <Show when=move || show_ignore_popup.get().is_some()>
            {move || show_ignore_popup.get().map(|data| {
                view! {
                    <IgnorePopup
                        data=data
                        notification=notification
                        refetch=refetch
                        on_close=move || show_ignore_popup.set(None)
                    />
                }
            })}
        </Show>

        <Show when=move || show_unignore_popup.get().is_some()>
            {move || show_unignore_popup.get().map(|data| {
                view! {
                    <UnignorePopup
                        data=data
                        notification=notification
                        on_close=move || show_unignore_popup.set(None)
                    />
                }
            })}
        </Show>
    }
}

// ── Workflow dropdown model ──

/// One entry in the workflow dropdown. Carries its own display label and the
/// [`WorkflowIntent`] it submits, so the commit handler passes the selected
/// option's intent straight through — no re-derivation, never `Named("")`.
#[derive(Clone, Debug, PartialEq, Eq)]
struct WorkflowOption {
    label: String,
    intent: WorkflowIntent,
    disabled: bool,
}

/// Display label for a single declared workflow: its name, or its id when the
/// config gave no name.
fn workflow_label(w: &WorkflowInfo) -> String {
    w.name.clone().unwrap_or_else(|| w.id.clone())
}

/// Build the dropdown's option list for the [`CommitWorkflows::Available`]
/// state, matching the web catalog's order:
/// - index 0: `None` → [`WorkflowIntent::NoWorkflow`], disabled when a workflow
///   is required. Its label gains `" (default)"` when the config names no
///   default and no workflow is required — then `None` is itself the bucket
///   default (the resolver treats `BucketDefault` and `NoWorkflow` alike);
/// - one entry per declared workflow (label = name or id, with `" (default)"`
///   appended to the bucket default) → [`WorkflowIntent::Named`].
///
/// Exactly one option ever bears `" (default)"`: whichever is the bucket's
/// actual default. A required-but-defaultless bucket has none.
///
/// There is deliberately no `Bucket default` item — the catalog offers only
/// `None` plus the concrete workflows. An Available-but-empty workflow list
/// therefore yields just `[None]`, which is fine.
fn workflow_options(
    workflows: &[WorkflowInfo],
    default_workflow: Option<&str>,
    is_workflow_required: bool,
) -> Vec<WorkflowOption> {
    // When the config names no default and doesn't require a workflow, `None`
    // IS the bucket default (the resolver treats `BucketDefault` and
    // `NoWorkflow` identically then), so it bears the `(default)` marker.
    // A required-but-defaultless bucket leaves `None` disabled with the
    // required error, so it must not be labeled default.
    let none_is_default = default_workflow.is_none() && !is_workflow_required;
    let none_label = if none_is_default {
        "None (default)".to_string()
    } else {
        "None".to_string()
    };
    let mut options = vec![WorkflowOption {
        label: none_label,
        intent: WorkflowIntent::NoWorkflow,
        disabled: is_workflow_required,
    }];

    options.extend(workflows.iter().map(|w| {
        let is_default = default_workflow.is_some_and(|id| id == w.id);
        let label = if is_default {
            format!("{} (default)", workflow_label(w))
        } else {
            workflow_label(w)
        };
        WorkflowOption {
            label,
            intent: WorkflowIntent::Named(w.id.clone()),
            disabled: false,
        }
    }));

    options
}

/// Index (into [`workflow_options`]'s output) that should start selected in the
/// [`CommitWorkflows::Available`] state.
///
/// The bucket default always wins over the previous revision's pick — that is
/// what makes it the default. This intentionally diverges from the web catalog,
/// which preselects the previous revision's workflow first. That rule dates to
/// the 2020 package-update dialog (quilt#1856), which chose to reuse the
/// previous revision's workflow even though `default_workflow` already existed —
/// a deliberate but lightly-justified UX choice (a one-line "reuse previous"
/// comment, no recorded rationale). We prefer the bucket default here instead —
/// it also heals packages whose workflow was set through the earlier error-prone
/// free-text field: the next commit adopts what the bucket intends rather than
/// carrying a likely-wrong prior stamp forward. Options are laid out as
/// `[None, workflows..]`, so
/// a workflow at position `p` maps to option index `p + 1`. The rule mirrors the
/// `(default)` marker that [`workflow_options`] applies:
///
/// 1. A named `default_workflow` present in the list → its index (ignoring the
///    previous pick).
/// 2. No named default and the bucket doesn't require a workflow → index 0, the
///    `None` head, which is itself labeled `None (default)` and so is the
///    effective default.
/// 3. Required bucket with no usable default (genuinely no default): the
///    previous revision's workflow if present, else index 0 (`None`, disabled).
fn preselected_index(
    workflows: &[WorkflowInfo],
    previous_workflow_id: Option<&str>,
    default_workflow: Option<&str>,
    is_workflow_required: bool,
) -> usize {
    // 1. A named default in the list wins outright.
    if let Some(def) = default_workflow
        && let Some(pos) = workflows.iter().position(|w| w.id == def)
    {
        return pos + 1;
    }
    // 2. No named default and no requirement: `None` is itself the default
    //    (labeled "None (default)"), so it wins too — keeping "the default
    //    always wins" uniform.
    if !is_workflow_required {
        return 0;
    }
    // 3. Required with genuinely no default: fall back to the previous pick.
    if let Some(prev) = previous_workflow_id
        && let Some(pos) = workflows.iter().position(|w| w.id == prev)
    {
        return pos + 1;
    }
    // The `None` item sits at the head of the option list.
    0
}

/// Which of the three [`CommitWorkflows`] states the section renders, plus the
/// per-state chrome the render needs.
#[derive(Clone, Debug, PartialEq, Eq)]
enum WorkflowViewKind {
    /// The bucket has a config: render the dropdown. Carries whether the bucket
    /// requires a workflow (drives the "required" hint).
    Available { is_workflow_required: bool },
    /// The bucket is ungoverned: a single disabled `None`, no choice to make.
    NotConfigured,
    /// The config couldn't be loaded: an inline notice, no dropdown.
    Unavailable,
}

/// The commit dialog's workflow section, resolved from the backend state.
///
/// `options` and `initial` drive both display and submit uniformly across all
/// three states: submit reads `options[selected].intent`. On the non-`Available`
/// states this is a single [`WorkflowIntent::BucketDefault`] entry at index 0,
/// so a config-less bucket resolves to no-workflow while a transient failure
/// re-resolves the real default at commit time.
#[derive(Clone, Debug, PartialEq, Eq)]
struct WorkflowView {
    kind: WorkflowViewKind,
    options: Vec<WorkflowOption>,
    initial: usize,
}

/// Map the backend [`CommitWorkflows`] state to the section's render model.
fn build_workflow_view(
    workflows: &CommitWorkflows,
    previous_workflow_id: Option<&str>,
) -> WorkflowView {
    match workflows {
        CommitWorkflows::Available {
            workflows,
            default_workflow,
            is_workflow_required,
        } => WorkflowView {
            kind: WorkflowViewKind::Available {
                is_workflow_required: *is_workflow_required,
            },
            options: workflow_options(
                workflows,
                default_workflow.as_deref(),
                *is_workflow_required,
            ),
            initial: preselected_index(
                workflows,
                previous_workflow_id,
                default_workflow.as_deref(),
                *is_workflow_required,
            ),
        },
        // Ungoverned bucket: a single disabled `None`. Submit sends
        // `BucketDefault`, which on a config-less bucket resolves to
        // no-workflow — the honest "let the bucket decide".
        CommitWorkflows::NotConfigured => WorkflowView {
            kind: WorkflowViewKind::NotConfigured,
            options: vec![WorkflowOption {
                label: "None".to_string(),
                intent: WorkflowIntent::BucketDefault,
                disabled: true,
            }],
            initial: 0,
        },
        // Config load failed: no dropdown, just a notice. Submit sends
        // `BucketDefault` so the real default re-resolves at commit time
        // (the network may have recovered) rather than forcing no-workflow.
        CommitWorkflows::Unavailable => WorkflowView {
            kind: WorkflowViewKind::Unavailable,
            options: vec![WorkflowOption {
                label: "Bucket default".to_string(),
                intent: WorkflowIntent::BucketDefault,
                disabled: true,
            }],
            initial: 0,
        },
    }
}

// ── Workflow section ──

#[component]
fn WorkflowSection(view: WorkflowView, selected: RwSignal<usize>) -> impl IntoView {
    let WorkflowView {
        kind,
        options,
        initial,
    } = view;

    match kind {
        WorkflowViewKind::Available {
            is_workflow_required,
        } => workflow_dropdown(options, selected, initial, is_workflow_required).into_any(),
        // Ungoverned bucket: a single disabled `None`, plus a hint explaining
        // why there is no choice to make. Submit already carries `BucketDefault`
        // via `options[0]`.
        WorkflowViewKind::NotConfigured => view! {
            <div class="workflow">
                <p class="field">
                    <label class="label" for="workflow">"Workflow"</label>
                    <select class="input" id="workflow" name="workflow" disabled>
                        <option selected>"None"</option>
                    </select>
                </p>
                <span class="hint">"This bucket has no workflow configuration."</span>
            </div>
        }
        .into_any(),
        // Config load failed: no dropdown, just an inline notice. Submit does
        // not block; it re-resolves the bucket default at commit time.
        WorkflowViewKind::Unavailable => view! {
            <div class="workflow">
                <p class="field">
                    <label class="label" for="workflow">"Workflow"</label>
                    <span class="hint">
                        "⚠ Couldn't load this bucket's workflows. Commit will use the bucket default."
                    </span>
                </p>
            </div>
        }
        .into_any(),
    }
}

/// The [`CommitWorkflows::Available`] dropdown.
fn workflow_dropdown(
    options: Vec<WorkflowOption>,
    selected: RwSignal<usize>,
    initial: usize,
    is_workflow_required: bool,
) -> impl IntoView {
    // Intents indexed by option position — used to decide whether the current
    // selection is the (disabled) `None` item, which drives the required hint.
    let intents: Vec<WorkflowIntent> = options.iter().map(|o| o.intent.clone()).collect();
    let show_required_hint = move || {
        is_workflow_required
            && intents
                .get(selected.get())
                .is_some_and(|i| *i == WorkflowIntent::NoWorkflow)
    };

    // The initial selection is rendered as the HTML boolean `selected`
    // attribute on the matching option. Unlike a `<select prop:value=…>`
    // (which tachys applies before the options mount, making it a no-op), the
    // attribute survives mount order and works even on the disabled `None`
    // option. `initial` is the same index that seeds `selected` in the parent,
    // so what the browser shows and what submit sends cannot diverge. The
    // `on:change` handler keeps the `selected` signal in step thereafter.
    let option_views = options
        .into_iter()
        .enumerate()
        .map(|(i, o)| {
            view! {
                <option value=i.to_string() selected=i == initial disabled=o.disabled>
                    {o.label}
                </option>
            }
        })
        .collect::<Vec<_>>();

    view! {
        <div class="workflow">
            <p class="field">
                <label class="label" for="workflow">"Workflow"</label>
                <select
                    class="input"
                    id="workflow"
                    name="workflow"
                    on:change=move |ev| {
                        if let Ok(i) = event_target_value(&ev).parse::<usize>() {
                            selected.set(i);
                        }
                    }
                >
                    {option_views}
                </select>
            </p>
            <Show when=show_required_hint>
                <span class="error">"Workflow is required for this bucket."</span>
            </Show>
        </div>
    }
}

// ── Toolbar actions ──

fn build_toolbar_actions(
    data: &CommitData,
    notification: RwSignal<Option<Notification>>,
    ui_locked: RwSignal<bool>,
) -> ToolbarActions {
    let namespace = data.namespace.clone();
    let origin_url = data.uri.as_ref().and_then(util::catalog_url);
    let has_catalog = origin_url.is_some();
    let catalog_disabled = data.status == "local";

    ToolbarActions::new(move || {
        let navigate = use_navigate();

        let ns_for_open = namespace.clone();
        let on_open_file_browser = move |_| {
            let ns = ns_for_open.clone();
            leptos::task::spawn_local(async move {
                match commands::open_in_file_browser(ns).await {
                    Ok(msg) => notification.set(Some(Notification::Success(msg))),
                    Err(e) => notification.set(Some(Notification::Error(e))),
                }
            });
        };

        let url_for_catalog = origin_url.clone();
        let on_open_catalog = move |_| {
            if let Some(url) = url_for_catalog.clone() {
                leptos::task::spawn_local(async move {
                    let _ = commands::open_in_web_browser(url).await;
                });
            }
        };

        let ns_for_uninstall = namespace.clone();
        let on_uninstall = move |_| {
            let ns = ns_for_uninstall.clone();
            let navigate = navigate.clone();
            ui_locked.set(true);
            leptos::task::spawn_local(async move {
                match commands::package_uninstall(ns).await {
                    Ok(msg) => {
                        notification.set(Some(Notification::Success(msg)));
                        navigate("/installed-packages-list", Default::default());
                    }
                    Err(e) => {
                        ui_locked.set(false);
                        notification.set(Some(Notification::Error(e)));
                    }
                }
            });
        };

        view! {
            <li>
                <buttons::OpenInFileBrowser on_click=on_open_file_browser />
            </li>
            {if has_catalog {
                view! {
                    <li>
                        <buttons::OpenInCatalog on_click=on_open_catalog disabled=catalog_disabled />
                    </li>
                }
                .into_any()
            } else {
                ().into_any()
            }}
            <li>
                <buttons::Remove on_click=on_uninstall />
            </li>
        }
        .into_any()
    })
}

// ── Entry row (commit variant — no checkboxes) ──

#[component]
fn CommitEntryRow(
    entry: EntryData,
    pkg_uri: Option<S3PackageUri>,
    notification: RwSignal<Option<Notification>>,
    show_ignore_popup: RwSignal<Option<IgnorePopupData>>,
    show_unignore_popup: RwSignal<Option<UnignorePopupData>>,
) -> impl IntoView {
    let is_deleted = entry.status == "deleted";
    let is_ignored = entry.ignored_by.is_some();
    let is_junky = entry.junky_pattern.is_some();

    let class_mods = {
        let mut classes = vec![entry.status.as_str()];
        if is_junky {
            classes.push("junky");
        }
        if is_ignored {
            classes.push("ignored");
        }
        format!("qui-entry {}", classes.join(" "))
    };

    let status_display = match entry.status.as_str() {
        "added" => "New",
        "deleted" => "Deleted",
        "modified" => "Modified",
        "pristine" => "Downloaded",
        "remote" => "Remote",
        _ => "",
    };

    let size_display = format_size(entry.size);
    let status_text = format!("{status_display}, {size_display}");

    let filename_display = entry.filename.clone();
    let filename_title = entry.filename.clone();

    // Action buttons
    let show_open_reveal = !is_deleted && !is_ignored && entry.status != "remote";
    let show_catalog = (entry.status == "remote" || entry.status == "pristine")
        && pkg_uri.as_ref().is_some_and(|u| u.catalog.is_some());

    let ns_for_open = entry.namespace.clone();
    let path_for_open = entry.filename.clone();
    let on_open = move |_| {
        let ns = ns_for_open.clone();
        let path = path_for_open.clone();
        leptos::task::spawn_local(async move {
            match commands::open_in_default_application(ns, path).await {
                Ok(msg) => notification.set(Some(Notification::Success(msg))),
                Err(e) => notification.set(Some(Notification::Error(e))),
            }
        });
    };

    let ns_for_reveal = entry.namespace.clone();
    let path_for_reveal = entry.filename.clone();
    let on_reveal = move |_| {
        let ns = ns_for_reveal.clone();
        let path = path_for_reveal.clone();
        leptos::task::spawn_local(async move {
            match commands::reveal_in_file_browser(ns, path).await {
                Ok(msg) => notification.set(Some(Notification::Success(msg))),
                Err(e) => notification.set(Some(Notification::Error(e))),
            }
        });
    };

    let path_for_catalog = entry.filename.clone();
    let on_open_catalog = move |_| {
        let Some(url) = pkg_uri
            .as_ref()
            .and_then(|u| util::entry_catalog_url(u, &path_for_catalog))
        else {
            return;
        };
        leptos::task::spawn_local(async move {
            let _ = commands::open_in_web_browser(url).await;
        });
    };

    let junky_pattern = entry.junky_pattern.clone();
    let ns_for_ignore = entry.namespace.clone();
    let path_for_ignore = entry.filename.clone();
    let on_ignore = move |_| {
        if let Some(pattern) = junky_pattern.clone() {
            show_ignore_popup.set(Some(IgnorePopupData {
                namespace: ns_for_ignore.clone(),
                path: path_for_ignore.clone(),
                suggested_pattern: pattern,
            }));
        }
    };

    let ignored_by = entry.ignored_by.clone();
    let ns_for_unignore = entry.namespace.clone();
    let on_unignore = move |_| {
        if let Some(pattern) = ignored_by.clone() {
            show_unignore_popup.set(Some(UnignorePopupData {
                namespace: ns_for_unignore.clone(),
                pattern,
            }));
        }
    };

    view! {
        <div class=class_mods>
            <div class="text">
                <p class="text-primary" title=filename_title data-testid="entry-name">
                    {filename_display}
                </p>
                <p class="text-secondary">{status_text}</p>
            </div>

            <div class="menu">
                <ul class="menu-list">
                    {if show_open_reveal {
                        view! {
                            <li class="menu-item">
                                <buttons::Open on_click=on_open small=true />
                            </li>
                            <li class="menu-item">
                                <buttons::Reveal on_click=on_reveal small=true />
                            </li>
                        }
                        .into_any()
                    } else {
                        ().into_any()
                    }}
                    {if show_catalog {
                        view! {
                            <li class="menu-item">
                                <buttons::OpenInCatalog on_click=on_open_catalog small=true />
                            </li>
                        }
                        .into_any()
                    } else {
                        ().into_any()
                    }}
                    {if is_junky {
                        view! {
                            <li class="menu-item">
                                <buttons::Ignore on_click=on_ignore small=true />
                            </li>
                        }
                        .into_any()
                    } else {
                        ().into_any()
                    }}
                    {if is_ignored {
                        view! {
                            <li class="menu-item">
                                <buttons::Unignore on_click=on_unignore small=true />
                            </li>
                        }
                        .into_any()
                    } else {
                        ().into_any()
                    }}
                </ul>
            </div>
        </div>
    }
}

// ── JSON editor integration ──

use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;

#[wasm_bindgen::prelude::wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["window"], js_name = "__getJsonEditorValue")]
    fn get_json_editor_value_js(target_id: &str) -> String;

    #[wasm_bindgen(js_namespace = ["window"], js_name = "__createJsonEditor")]
    fn create_json_editor_js(target_id: &str, initial_value: &str);

    #[wasm_bindgen(js_namespace = ["window"], js_name = "__destroyJsonEditor")]
    fn destroy_json_editor_js(target_id: &str);
}

fn get_json_editor_value(target_id: &str) -> String {
    // If the JS editor is available, use it; otherwise fall back to textarea
    let value = get_json_editor_value_js(target_id);
    if !value.is_empty() {
        return value;
    }
    // Fall back to the textarea value
    web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.get_element_by_id("metadata"))
        .and_then(|el| {
            el.dyn_ref::<web_sys::HtmlTextAreaElement>()
                .map(web_sys::HtmlTextAreaElement::value)
        })
        .unwrap_or_default()
}

#[component]
fn JsonEditor(id: &'static str, initial_value: String) -> impl IntoView {
    let init_value = initial_value.clone();
    // once_into_js creates a JS function that frees the Rust closure after
    // a single call, avoiding the permanent leak from Closure::forget().
    let cb = Closure::once_into_js(move || {
        create_json_editor_js(id, &init_value);
    });
    // Schedule after the current frame so Leptos has committed the DOM.
    if let Some(window) = web_sys::window() {
        let _ = window.request_animation_frame(cb.unchecked_ref());
    }

    on_cleanup(move || {
        destroy_json_editor_js(id);
    });

    view! {
        <div class="metadata" id=id></div>
    }
}

#[cfg(test)]
mod tests {
    use super::{
        WorkflowOption, WorkflowViewKind, build_workflow_view, preselected_index, workflow_options,
    };
    use crate::commands::{CommitWorkflows, WorkflowInfo, WorkflowIntent};

    fn wf(id: &str, name: Option<&str>) -> WorkflowInfo {
        WorkflowInfo {
            id: id.to_string(),
            name: name.map(str::to_string),
            description: None,
        }
    }

    // ── Normal path (workflows non-empty) ──

    #[test]
    fn options_normal_none_head_default_labeled_no_bucket_default() {
        let wfs = vec![wf("alpha", Some("Alpha WF")), wf("beta", None)];
        let opts = workflow_options(&wfs, Some("alpha"), false);
        assert_eq!(
            opts,
            vec![
                WorkflowOption {
                    label: "None".to_string(),
                    intent: WorkflowIntent::NoWorkflow,
                    disabled: false,
                },
                WorkflowOption {
                    // The bucket default gets the " (default)" suffix.
                    label: "Alpha WF (default)".to_string(),
                    intent: WorkflowIntent::Named("alpha".to_string()),
                    disabled: false,
                },
                WorkflowOption {
                    // No name → fall back to the id as the label; not the default.
                    label: "beta".to_string(),
                    intent: WorkflowIntent::Named("beta".to_string()),
                    disabled: false,
                },
            ]
        );
        // The normal path never offers a `Bucket default` item.
        assert!(
            opts.iter()
                .all(|o| o.intent != WorkflowIntent::BucketDefault)
        );
    }

    #[test]
    fn options_default_suffix_uses_label_or_id() {
        // Unnamed default → suffix appended to the id.
        let wfs = vec![wf("beta", None)];
        assert_eq!(
            workflow_options(&wfs, Some("beta"), false)[1].label,
            "beta (default)"
        );
        // Default id not present in the list → no option carries the suffix.
        let wfs = vec![wf("alpha", Some("Alpha WF"))];
        let opts = workflow_options(&wfs, Some("ghost"), false);
        assert_eq!(opts[1].label, "Alpha WF");
        assert!(opts.iter().all(|o| !o.label.contains("(default)")));
    }

    #[test]
    fn options_no_default_not_required_labels_none_as_default() {
        // No named default and not required → `None` IS the bucket default, so
        // it carries the suffix; the concrete workflow does not.
        let wfs = vec![wf("alpha", Some("Alpha WF"))];
        let opts = workflow_options(&wfs, None, false);
        assert_eq!(opts[0].label, "None (default)");
        assert_eq!(opts[1].label, "Alpha WF");
    }

    /// Number of labels ending in the `(default)` marker across the option list.
    fn default_marker_count(opts: &[WorkflowOption]) -> usize {
        opts.iter()
            .filter(|o| o.label.ends_with("(default)"))
            .count()
    }

    #[test]
    fn options_exactly_one_default_marker_when_named_default_present() {
        let wfs = vec![wf("alpha", Some("Alpha WF")), wf("beta", None)];
        let opts = workflow_options(&wfs, Some("alpha"), false);
        // The named default is the real workflow; `None` stays plain.
        assert_eq!(opts[0].label, "None");
        assert_eq!(default_marker_count(&opts), 1);
        assert_eq!(opts[1].label, "Alpha WF (default)");
    }

    #[test]
    fn options_exactly_one_default_marker_when_no_default_not_required() {
        // No named default and not required → `None` is the only default.
        let wfs = vec![wf("alpha", Some("Alpha WF")), wf("beta", None)];
        let opts = workflow_options(&wfs, None, false);
        assert_eq!(default_marker_count(&opts), 1);
        assert_eq!(opts[0].label, "None (default)");
    }

    #[test]
    fn options_no_default_marker_when_no_default_but_required() {
        // Required with no named default → `None` is disabled with the required
        // error, so it must NOT be labeled default; there genuinely is none.
        let wfs = vec![wf("alpha", Some("Alpha WF")), wf("beta", None)];
        let opts = workflow_options(&wfs, None, true);
        assert_eq!(default_marker_count(&opts), 0);
        assert_eq!(opts[0].label, "None");
        assert!(opts[0].disabled);
    }

    #[test]
    fn options_no_default_marker_when_named_default_absent_from_list() {
        // Stale/misconfigured default id not in the list → the real-workflow
        // branch adds no marker, and `None` gets none either (default is Some).
        let wfs = vec![wf("alpha", Some("Alpha WF")), wf("beta", None)];
        let opts = workflow_options(&wfs, Some("ghost"), false);
        assert_eq!(default_marker_count(&opts), 0);
        assert_eq!(opts[0].label, "None");
    }

    #[test]
    fn options_none_disabled_iff_required() {
        let wfs = vec![wf("alpha", None)];
        let required = workflow_options(&wfs, None, true);
        assert_eq!(required[0].intent, WorkflowIntent::NoWorkflow);
        assert!(required[0].disabled);
        assert!(!workflow_options(&wfs, None, false)[0].disabled);
    }

    #[test]
    fn options_available_empty_list_is_just_none() {
        // An Available-but-empty workflow list yields just `[None]` (labeled the
        // default, since nothing is required and no default is named).
        let opts = workflow_options(&[], None, false);
        assert_eq!(
            opts,
            vec![WorkflowOption {
                label: "None (default)".to_string(),
                intent: WorkflowIntent::NoWorkflow,
                disabled: false,
            }]
        );
    }

    // ── State → render model (`build_workflow_view`) ──

    #[test]
    fn view_available_builds_dropdown_options() {
        let workflows = CommitWorkflows::Available {
            workflows: vec![wf("alpha", Some("Alpha WF")), wf("beta", None)],
            default_workflow: Some("alpha".to_string()),
            is_workflow_required: false,
        };
        let view = build_workflow_view(&workflows, Some("beta"));
        assert_eq!(
            view.kind,
            WorkflowViewKind::Available {
                is_workflow_required: false
            }
        );
        // Options + preselection match the pure builders for the Available case.
        // Previous picked "beta", but the named default "alpha" wins.
        assert_eq!(view.options.len(), 3);
        assert_eq!(view.initial, 1);
        assert_eq!(
            view.options[view.initial].intent,
            WorkflowIntent::Named("alpha".to_string())
        );
    }

    #[test]
    fn view_not_configured_is_single_disabled_none_submitting_bucket_default() {
        let view = build_workflow_view(&CommitWorkflows::NotConfigured, None);
        assert_eq!(view.kind, WorkflowViewKind::NotConfigured);
        assert_eq!(view.initial, 0);
        assert_eq!(
            view.options,
            vec![WorkflowOption {
                label: "None".to_string(),
                intent: WorkflowIntent::BucketDefault,
                disabled: true,
            }]
        );
    }

    #[test]
    fn view_unavailable_submits_bucket_default() {
        let view = build_workflow_view(&CommitWorkflows::Unavailable, Some("ignored"));
        assert_eq!(view.kind, WorkflowViewKind::Unavailable);
        assert_eq!(view.initial, 0);
        // Both non-Available states submit `BucketDefault`, but they are
        // distinct render inputs (NotConfigured vs Unavailable).
        assert_eq!(view.options[0].intent, WorkflowIntent::BucketDefault);
    }

    // ── Preselection: the bucket default always wins over the previous pick ──

    #[test]
    fn preselect_named_default_wins_over_previous() {
        let wfs = vec![wf("alpha", None), wf("beta", None)];
        // Options: [None=0, alpha=1, beta=2]. Previous picked beta, but the
        // named default alpha wins — that's why it is the default.
        assert_eq!(
            preselected_index(&wfs, Some("beta"), Some("alpha"), false),
            1
        );
        // Requiredness doesn't change it: a named default still wins.
        assert_eq!(
            preselected_index(&wfs, Some("beta"), Some("alpha"), true),
            1
        );
    }

    #[test]
    fn preselect_none_default_wins_when_not_required() {
        let wfs = vec![wf("alpha", None), wf("beta", None)];
        // No named default, not required → `None` is the effective default
        // ("None (default)") and wins over the previous pick.
        assert_eq!(preselected_index(&wfs, Some("beta"), None, false), 0);
        // A named default absent from the list is no usable default; not
        // required → `None` still wins.
        assert_eq!(
            preselected_index(&wfs, Some("beta"), Some("ghost"), false),
            0
        );
    }

    #[test]
    fn preselect_required_no_default_falls_back_to_previous() {
        let wfs = vec![wf("alpha", None), wf("beta", None)];
        // Required with genuinely no default: the previous revision's workflow.
        assert_eq!(preselected_index(&wfs, Some("beta"), None, true), 2);
        // A named default absent from the list is still no default.
        assert_eq!(
            preselected_index(&wfs, Some("beta"), Some("ghost"), true),
            2
        );
        // Required, no default, no previous → the `None` head (disabled).
        assert_eq!(preselected_index(&wfs, None, None, true), 0);
        // Required, no default, previous absent from the list → `None` head.
        assert_eq!(preselected_index(&wfs, Some("ghost"), None, true), 0);
    }

    // ── Consistency invariant: preselection lands on the "(default)" option ──

    #[test]
    fn preselect_agrees_with_default_label() {
        let wfs = vec![wf("alpha", Some("Alpha WF")), wf("beta", None)];

        // Case 1: named default present → selected option bears "(default)".
        let opts = workflow_options(&wfs, Some("alpha"), false);
        let idx = preselected_index(&wfs, Some("beta"), Some("alpha"), false);
        assert!(opts[idx].label.ends_with("(default)"));
        assert_eq!(opts[idx].intent, WorkflowIntent::Named("alpha".to_string()));

        // Case 2: no named default, not required → the `None` head is
        // "None (default)" and is the selection.
        let opts = workflow_options(&wfs, None, false);
        let idx = preselected_index(&wfs, Some("beta"), None, false);
        assert_eq!(idx, 0);
        assert_eq!(opts[0].label, "None (default)");

        // Case 3: required, no default → no option bears "(default)"; the
        // selection is the previous pick (or `None`).
        let opts = workflow_options(&wfs, None, true);
        assert!(opts.iter().all(|o| !o.label.ends_with("(default)")));
        let idx = preselected_index(&wfs, Some("beta"), None, true);
        assert_eq!(idx, 2);
        assert_eq!(opts[idx].intent, WorkflowIntent::Named("beta".to_string()));
    }

    // ── Display == submit invariant, guarded at the pure-fn level ──

    #[test]
    fn preselected_index_maps_to_submitted_intent_normal() {
        let wfs = vec![wf("alpha", Some("Alpha WF")), wf("beta", None)];
        let opts = workflow_options(&wfs, Some("alpha"), false);
        // Previous revision picked "beta", but the named default "alpha" wins:
        // the rendered+submitted option is the labeled default.
        let idx = preselected_index(&wfs, Some("beta"), Some("alpha"), false);
        assert_eq!(opts[idx].intent, WorkflowIntent::Named("alpha".to_string()));
        assert_eq!(opts[idx].label, "Alpha WF (default)");
        // No previous id: preselection still lands on the labeled default.
        let idx = preselected_index(&wfs, None, Some("alpha"), false);
        assert_eq!(opts[idx].intent, WorkflowIntent::Named("alpha".to_string()));
        assert_eq!(opts[idx].label, "Alpha WF (default)");
    }

    #[test]
    fn view_non_available_states_submit_bucket_default() {
        // The submit path reads `options[selected].intent`; both non-Available
        // states must send `BucketDefault` from their single option at index 0.
        for state in [CommitWorkflows::NotConfigured, CommitWorkflows::Unavailable] {
            let view = build_workflow_view(&state, None);
            assert_eq!(
                view.options[view.initial].intent,
                WorkflowIntent::BucketDefault
            );
        }
    }
}
