use leptos::prelude::*;
use leptos_router::hooks::{use_navigate, use_query_map};

use crate::commands::{self, CommitData, EntryData, WorkflowData};
use crate::components::layout::{BreadcrumbItem, BreadcrumbLink};
use crate::components::buttons;
use crate::components::{
    IgnorePopup, IgnorePopupData, Layout, Notification, Spinner, ToolbarActions, UnignorePopup,
    UnignorePopupData,
};
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

    // Workflow state
    let workflow = data.workflow.clone();
    let has_workflow = workflow.is_some();
    let workflow_id = RwSignal::new(
        workflow
            .as_ref()
            .and_then(|w| w.id.clone())
            .unwrap_or_default(),
    );
    let workflow_null = RwSignal::new(workflow.as_ref().map(|w| w.id.is_none()).unwrap_or(true));

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

    // Commit action
    let ns_for_commit = namespace.clone();
    let committing = RwSignal::new(false);
    let navigate_for_commit = navigate.clone();
    let on_commit = move |_| {
        let navigate = navigate_for_commit.clone();
        if committing.get_untracked() {
            return;
        }
        let msg = message.get_untracked();
        if msg.trim().is_empty() {
            return;
        }
        committing.set(true);
        ui_locked.set(true);
        let ns = ns_for_commit.clone();
        let meta = get_json_editor_value("metadata-editor");
        let wf = if has_workflow && !workflow_null.get_untracked() {
            Some(workflow_id.get_untracked())
        } else {
            None
        };
        leptos::task::spawn_local(async move {
            match commands::package_commit(ns.clone(), msg, meta, wf).await {
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
    };

    view! {
        <div class="qui-page-commit container container-wide">
            // ── Left column: form ──
            <div class="main">
                <div class="form">
                    // ── Workflow ──
                    <WorkflowSection
                        workflow=data.workflow.clone()
                        workflow_id=workflow_id
                        workflow_null=workflow_null
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
                                            {format!("({})", unmodified_count)}
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
                                            {format!("({})", ignored_count)}
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
            <button
                class="qui-button primary large"
                type="button"
                prop:disabled=move || committing.get()
                on:click=on_commit
            >
                <span>{move || if committing.get() { "Committing\u{2026}" } else { "Commit" }}</span>
                <img class="qui-icon" src="/assets/img/icons/done.svg" />
            </button>
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

// ── Workflow section ──

#[component]
fn WorkflowSection(
    workflow: Option<WorkflowData>,
    workflow_id: RwSignal<String>,
    workflow_null: RwSignal<bool>,
) -> impl IntoView {
    match workflow {
        Some(w) => {
            let has_id = w.id.is_some();
            let url = w.url.clone();

            view! {
                <div class="workflow">
                    <p class="field">
                        <label class="label" for="workflow">"Workflow ID"</label>
                        <input
                            class="input"
                            id="workflow"
                            name="workflow"
                            prop:value=move || workflow_id.get()
                            prop:disabled=move || workflow_null.get()
                            on:input=move |ev| workflow_id.set(event_target_value(&ev))
                        />
                    </p>
                    <div class="workflow-null">
                        <input
                            id="workflow-null"
                            type="checkbox"
                            prop:checked=move || workflow_null.get()
                            on:change=move |_| {
                                workflow_null.set(!workflow_null.get_untracked());
                            }
                        />
                        <label class="workflow-null-label" for="workflow-null">
                            "No workflow"
                        </label>
                    </div>
                    {url.map(|url_str| {
                        let url_for_click = url_str.clone();
                        view! {
                            <p class="field-description">
                                {if has_id { "Use the workflow ID from " } else { "" }}
                                <a
                                    class="link"
                                    on:click=move |_| {
                                        let url = url_for_click.clone();
                                        leptos::task::spawn_local(async move {
                                            let _ = commands::open_in_web_browser(url).await;
                                        });
                                    }
                                >
                                    ".quilt/workflows/config.yaml"
                                </a>
                            </p>
                        }
                    })}
                </div>
            }
            .into_any()
        }
        None => view! {
            <div class="workflow">
                <p class="field">
                    <label class="label" for="workflow">"Workflow ID"</label>
                    <input class="input" disabled prop:value="Workflow not available" />
                </p>
                <div class="workflow-null">
                    <input id="workflow-null" type="checkbox" checked disabled />
                    <label class="workflow-null-label" for="workflow-null">
                        "No workflow"
                    </label>
                </div>
            </div>
        }
        .into_any(),
    }
}

// ── Toolbar actions ──

fn build_toolbar_actions(
    data: &CommitData,
    notification: RwSignal<Option<Notification>>,
    ui_locked: RwSignal<bool>,
) -> ToolbarActions {
    let namespace = data.namespace.clone();
    let origin_url = data.origin_url.clone();
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
    let show_catalog =
        (entry.status == "remote" || entry.status == "pristine") && entry.origin_url.is_some();

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

    let catalog_url = entry.origin_url.clone();
    let on_open_catalog = move |_| {
        if let Some(url) = catalog_url.clone() {
            leptos::task::spawn_local(async move {
                let _ = commands::open_in_web_browser(url).await;
            });
        }
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

use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsCast;

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
                .map(|ta| ta.value())
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
