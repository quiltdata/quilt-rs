use leptos::prelude::*;
use leptos_router::hooks::{use_navigate, use_query_map};

use quilt_uri::S3PackageUri;

use crate::commands::{self, CommitData, EntryData, WorkflowInfo, WorkflowIntent};
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

    // Workflow state: a single dropdown whose selected index is the whole
    // state. Each option carries the `WorkflowIntent` it submits.
    let default_workflow = data.default_workflow.clone();
    let is_workflow_required = data.is_workflow_required;
    let wf_options = workflow_options(
        &data.workflows,
        default_workflow.as_deref(),
        is_workflow_required,
    );
    let initial_workflow = preselected_index(
        &data.workflows,
        data.workflow.as_ref().and_then(|w| w.id.as_deref()),
        default_workflow.as_deref(),
    );
    // `initial_workflow` is the single source of truth for the starting
    // selection: it both seeds this signal (which submit reads) and is passed
    // to `WorkflowSection` to render the `selected` attribute, so display and
    // submit start from the same index into the same option list.
    let selected_workflow = RwSignal::new(initial_workflow);
    // Intents mirrored for the submit path — the selected option's intent is
    // passed straight through, so `Named("")` can never be constructed.
    let workflow_intents: Vec<WorkflowIntent> =
        wf_options.iter().map(|o| o.intent.clone()).collect();

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
                        options=wf_options
                        selected=selected_workflow
                        initial=initial_workflow
                        is_workflow_required=is_workflow_required
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

/// Build the dropdown's option list.
///
/// Normal path (workflows non-empty), matching the web catalog's order:
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
/// There is deliberately no `Bucket default` item on the normal path — the
/// catalog offers only `None` plus the concrete workflows.
///
/// Degraded path (workflows empty — a config-less bucket or a config-fetch
/// failure): a minimal two-item control
/// `[Bucket default → BucketDefault, No workflow → NoWorkflow]`. Keeping
/// `BucketDefault` here means a no-touch commit re-resolves the bucket's real
/// default at commit time instead of forcing `NoWorkflow` on a transient
/// failure.
fn workflow_options(
    workflows: &[WorkflowInfo],
    default_workflow: Option<&str>,
    is_workflow_required: bool,
) -> Vec<WorkflowOption> {
    if workflows.is_empty() {
        return vec![
            WorkflowOption {
                label: "Bucket default".to_string(),
                intent: WorkflowIntent::BucketDefault,
                disabled: false,
            },
            WorkflowOption {
                label: "No workflow".to_string(),
                intent: WorkflowIntent::NoWorkflow,
                disabled: false,
            },
        ];
    }

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

/// Index (into [`workflow_options`]'s output) that should start selected.
///
/// Normal path: the previous revision's `workflow.id` if it appears in the
/// list, else the bucket default's id if present, else index 0 (the `None`
/// item). Options are laid out as `[None, workflows..]`, so a workflow at
/// position `p` maps to option index `p + 1`.
///
/// Degraded path (empty `workflows`): index 0 — the `Bucket default` item, so
/// a transient config-fetch failure never silently forces `No workflow`.
fn preselected_index(
    workflows: &[WorkflowInfo],
    previous_workflow_id: Option<&str>,
    default_workflow: Option<&str>,
) -> usize {
    if workflows.is_empty() {
        return 0;
    }
    if let Some(prev) = previous_workflow_id
        && let Some(pos) = workflows.iter().position(|w| w.id == prev)
    {
        return pos + 1;
    }
    if let Some(def) = default_workflow
        && let Some(pos) = workflows.iter().position(|w| w.id == def)
    {
        return pos + 1;
    }
    // The `None` item sits at the head of the normal-path list.
    0
}

// ── Workflow section ──

#[component]
fn WorkflowSection(
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
    use super::{WorkflowOption, preselected_index, workflow_options};
    use crate::commands::{WorkflowInfo, WorkflowIntent};

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

    // ── Degraded path (workflows empty) ──

    #[test]
    fn options_degraded_is_bucket_default_then_no_workflow() {
        // Flags are irrelevant on the degraded path.
        let opts = workflow_options(&[], Some("x"), true);
        assert_eq!(
            opts,
            vec![
                WorkflowOption {
                    label: "Bucket default".to_string(),
                    intent: WorkflowIntent::BucketDefault,
                    disabled: false,
                },
                WorkflowOption {
                    label: "No workflow".to_string(),
                    intent: WorkflowIntent::NoWorkflow,
                    disabled: false,
                },
            ]
        );
    }

    // ── Preselection ──

    #[test]
    fn preselect_previous_id_when_in_list() {
        let wfs = vec![wf("alpha", None), wf("beta", None)];
        // Options: [None=0, alpha=1, beta=2].
        assert_eq!(preselected_index(&wfs, Some("beta"), Some("alpha")), 2);
    }

    #[test]
    fn preselect_prev_not_in_list_uses_default() {
        let wfs = vec![wf("alpha", None), wf("beta", None)];
        // Previous id absent, default present → the default's index.
        assert_eq!(preselected_index(&wfs, Some("ghost"), Some("beta")), 2);
        // No previous id, default present → the default's index.
        assert_eq!(preselected_index(&wfs, None, Some("alpha")), 1);
    }

    #[test]
    fn preselect_no_usable_default_is_none_head() {
        let wfs = vec![wf("alpha", None)];
        // No previous id and no default → the `None` head (0).
        assert_eq!(preselected_index(&wfs, None, None), 0);
        // Default set but absent from the list → the `None` head (0).
        assert_eq!(preselected_index(&wfs, Some("ghost"), Some("ghost")), 0);
    }

    #[test]
    fn preselect_degraded_is_bucket_default() {
        // Empty list → the `Bucket default` item at index 0, regardless of flags.
        assert_eq!(preselected_index(&[], None, None), 0);
        assert_eq!(preselected_index(&[], Some("x"), Some("y")), 0);
    }

    // ── Display == submit invariant, guarded at the pure-fn level ──

    #[test]
    fn preselected_index_maps_to_submitted_intent_normal() {
        let wfs = vec![wf("alpha", Some("Alpha WF")), wf("beta", None)];
        let opts = workflow_options(&wfs, Some("alpha"), false);
        // Previous revision picked "beta": the rendered+submitted option is beta.
        let idx = preselected_index(&wfs, Some("beta"), Some("alpha"));
        assert_eq!(opts[idx].intent, WorkflowIntent::Named("beta".to_string()));
        assert_eq!(opts[idx].label, "beta");
        // No previous id: preselection lands on the labeled default.
        let idx = preselected_index(&wfs, None, Some("alpha"));
        assert_eq!(opts[idx].intent, WorkflowIntent::Named("alpha".to_string()));
        assert_eq!(opts[idx].label, "Alpha WF (default)");
    }

    #[test]
    fn preselected_index_maps_to_submitted_intent_degraded() {
        let opts = workflow_options(&[], None, false);
        let idx = preselected_index(&[], None, None);
        assert_eq!(opts[idx].intent, WorkflowIntent::BucketDefault);
    }
}
