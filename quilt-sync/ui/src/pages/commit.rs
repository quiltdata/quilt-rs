use std::time::Duration;

use leptos::prelude::*;
use leptos_router::hooks::{use_navigate, use_query_map};

use quilt_uri::S3PackageUri;

use crate::commands::{
    self, CommitData, CommitViolation, CommitWorkflows, EntryData, ViolationField, WorkflowInfo,
    WorkflowIntent,
};
use crate::components::buttons;
use crate::components::layout::{BreadcrumbItem, BreadcrumbLink};
use crate::components::{
    IgnorePopup, IgnorePopupData, Layout, Notification, PreviousWorkflow, Spinner, ToolbarActions,
    UnignorePopup, UnignorePopupData, WorkflowSection, build_workflow_view, previous_workflow_note,
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
    let previous_workflow = PreviousWorkflow::from_stamp(data.workflow.as_ref());
    let wf_view = build_workflow_view(&data.workflows, previous_workflow.preselect_id());
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

    // Neutral override note: recomputed as the selection changes, it surfaces
    // when the current pick diverges from what the previous revision stamped
    // (the bucket default winning over the previous pick in `preselected_index`
    // can silently override the user's earlier choice). Only the `Available`
    // state declares workflows to name; the other states carry an empty list
    // and never render the note.
    let note_workflows: Vec<WorkflowInfo> = match &data.workflows {
        CommitWorkflows::Available { workflows, .. } => workflows.clone(),
        CommitWorkflows::NotConfigured
        | CommitWorkflows::Unavailable
        | CommitWorkflows::Invalid { .. } => Vec::new(),
    };
    let note_intents = workflow_intents.clone();
    let workflow_note = Memo::new(move |_| {
        let current = note_intents
            .get(selected_workflow.get())
            .cloned()
            .unwrap_or(WorkflowIntent::BucketDefault);
        previous_workflow_note(&previous_workflow, &current, &note_workflows)
    });

    // ── Live (advisory) workflow validation ──
    // As the user edits the message / metadata, validate against the selected
    // workflow's rules and show inline violations before the commit attempt.
    // Advisory only: the buttons stay enabled and the commit-time gate remains
    // the authority. The package name is read-only here, so `handle_pattern`
    // is validated once the rules load (it can't change) rather than per keystroke.
    let validation_ns = data.namespace.clone();
    let validation_name = data.namespace.clone();
    // Reactive mirror of the metadata editor's current text: the JSON editor
    // writes edits into the hidden `#metadata` textarea and dispatches an
    // `input` event (see json-editor-glue.js), so this tracks edits from either
    // the editor or the textarea fallback.
    let metadata_text = RwSignal::new(data.user_meta.clone());
    // The concretely-selected workflow id, or `None` for the `None` /
    // bucket-default selections — which the commit gate does not enforce a
    // named workflow's rules against, so there is nothing to validate live.
    let id_intents = workflow_intents.clone();
    let selected_workflow_id = Memo::new(move |_| match id_intents.get(selected_workflow.get()) {
        Some(WorkflowIntent::Named(id)) => Some(id.clone()),
        _ => None,
    });

    // The current validation input; the debounced mirror keys the fetch so it
    // fires once per settled edit (~400ms) instead of on every keystroke.
    let live_key = Memo::new(move |_| {
        (
            message.get(),
            metadata_text.get(),
            selected_workflow_id.get(),
        )
    });
    let debounced_key = RwSignal::new(live_key.get_untracked());
    let debounce_timer: StoredValue<Option<TimeoutHandle>> = StoredValue::new(None);
    Effect::new(move |_| {
        let key = live_key.get();
        // Skip arming the timer when the input already equals the debounced
        // mirror. The Effect runs once on mount with `key` equal to the value
        // `debounced_key` was seeded with; `RwSignal::set` notifies
        // unconditionally (no `PartialEq` dedupe), so scheduling that set would
        // re-run the validation resource with identical input — a redundant IPC
        // round-trip. Only settled *edits* should (re)arm the timer.
        if !should_debounce(&key, &debounced_key.get_untracked()) {
            return;
        }
        if let Some(handle) = debounce_timer.get_value() {
            handle.clear();
        }
        if let Ok(handle) =
            set_timeout_with_handle(move || debounced_key.set(key), Duration::from_millis(400))
        {
            debounce_timer.set_value(Some(handle));
        }
    });
    on_cleanup(move || {
        if let Some(Some(handle)) = debounce_timer.try_get_value() {
            handle.clear();
        }
    });

    // Validate the debounced input. `load_workflow_rules` fetches + caches the
    // rules on the backend once per workflow id (a no-op after the first call,
    // so no network on later keystrokes); `validate_commit_candidate` then reads
    // the cache with no I/O. A `None` workflow id short-circuits to no
    // violations. The result carries the input key it was computed from.
    let validation = LocalResource::new(move || {
        let (message, metadata, workflow_id) = debounced_key.get();
        let ns = validation_ns.clone();
        let name = validation_name.clone();
        async move {
            let key = (message.clone(), metadata.clone(), workflow_id.clone());
            let Some(id) = workflow_id else {
                return (key, Vec::new());
            };
            let _ = commands::load_workflow_rules(ns.clone(), id.clone()).await;
            let violations = commands::validate_commit_candidate(ns, id, message, metadata, name)
                .await
                .unwrap_or_default();
            (key, violations)
        }
    });

    // Self-keyed: only surface a response matching the CURRENT input and
    // workflow selection, so a slow in-flight response never paints over newer
    // input (the stale-response discipline from the workflow selector).
    let live_violations = Memo::new(move |_| match validation.get() {
        Some((key, violations)) if key == live_key.get() => violations,
        _ => Vec::new(),
    });

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
                        note=workflow_note
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
                    {move || field_violation_view(&live_violations.get(), ViolationField::Name)}

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
                    {move || field_violation_view(&live_violations.get(), ViolationField::Message)}

                    // ── Metadata (textarea + JSON editor) ──
                    <p class="field">
                        <label class="label" for="metadata">"User metadata"</label>
                        <textarea
                            class="textarea"
                            id="metadata"
                            name="metadata"
                            placeholder="{ \"key\": \"value\" }"
                            on:input=move |ev| metadata_text.set(event_target_value(&ev))
                        >
                            {user_meta}
                        </textarea>
                        {user_meta_error.map(|err| view! {
                            <span class="error">{err}</span>
                        })}
                    </p>
                    {move || field_violation_view(&live_violations.get(), ViolationField::Metadata)}
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

// ── Live-validation view model ──

/// Whether a settled edit should (re)arm the debounce timer: only when the
/// current input key differs from the value the debounced mirror already holds.
/// Guards the mount-time Effect run, which observes the initial key equal to the
/// seeded mirror — scheduling a `set` there would re-run the validation resource
/// with identical input (`RwSignal::set` has no `PartialEq` dedupe).
fn should_debounce<T: PartialEq>(key: &T, debounced: &T) -> bool {
    key != debounced
}

/// The messages of the violations that belong under `field`, in order. Pure
/// mapping from the backend's per-field violation list to the strings one field
/// renders, so the routing is unit-testable without a DOM.
fn field_violations(violations: &[CommitViolation], field: ViolationField) -> Vec<String> {
    violations
        .iter()
        .filter(|violation| violation.field == field)
        .map(|violation| violation.message.clone())
        .collect()
}

/// Render a field's advisory violations as a modest error list, or nothing when
/// the field is clean.
fn field_violation_view(
    violations: &[CommitViolation],
    field: ViolationField,
) -> Option<impl IntoView + use<>> {
    let messages = field_violations(violations, field);
    (!messages.is_empty()).then(|| {
        let items = messages
            .into_iter()
            .map(|message| view! { <li>{message}</li> })
            .collect::<Vec<_>>();
        view! { <ul class="qui-field-violations">{items}</ul> }
    })
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
    use super::{field_violations, should_debounce};
    use crate::commands::{CommitViolation, ViolationField};

    fn violation(field: ViolationField, message: &str) -> CommitViolation {
        CommitViolation {
            field,
            message: message.to_string(),
        }
    }

    #[test]
    fn should_debounce_skips_equal_key() {
        // Mount-time: the input equals the seeded debounced mirror, so no timer.
        let key = ("msg".to_string(), "{}".to_string(), Some("wf".to_string()));
        assert!(!should_debounce(&key, &key.clone()));
        // A settled edit differs, so the timer arms.
        let edited = ("msg2".to_string(), "{}".to_string(), Some("wf".to_string()));
        assert!(should_debounce(&edited, &key));
    }

    #[test]
    fn field_violations_routes_messages_to_their_field() {
        let violations = vec![
            violation(ViolationField::Message, "message required"),
            violation(ViolationField::Metadata, "missing owner"),
            violation(ViolationField::Name, "handle mismatch"),
            violation(ViolationField::Metadata, "bad json"),
        ];
        assert_eq!(
            field_violations(&violations, ViolationField::Message),
            vec!["message required".to_string()]
        );
        // Two metadata violations preserve their order.
        assert_eq!(
            field_violations(&violations, ViolationField::Metadata),
            vec!["missing owner".to_string(), "bad json".to_string()]
        );
        assert_eq!(
            field_violations(&violations, ViolationField::Name),
            vec!["handle mismatch".to_string()]
        );
    }

    #[test]
    fn field_violations_empty_when_field_clean() {
        let violations = vec![violation(ViolationField::Message, "x")];
        assert!(field_violations(&violations, ViolationField::Name).is_empty());
        assert!(field_violations(&[], ViolationField::Message).is_empty());
    }
}
