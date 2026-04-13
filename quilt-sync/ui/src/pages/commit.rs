use leptos::prelude::*;
use leptos_router::hooks::{use_navigate, use_query_map};

use crate::commands::{self, CommitData, EntryData, WorkflowData};
use crate::components::layout::{BreadcrumbItem, BreadcrumbLink};
use crate::components::{Layout, Notification, Spinner, ToolbarActions};

// ── Commit page ──

#[component]
pub fn Commit() -> impl IntoView {
    let notification = RwSignal::new(None);
    let ui_locked = RwSignal::new(false);

    let query = use_query_map();
    let data = LocalResource::new(move || {
        let namespace = query.read().get("namespace").unwrap_or_default();
        async move {
            commands::get_commit_data(namespace).await
        }
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
                                <CommitContent data=d notification=notification ui_locked=ui_locked />
                            </Layout>
                        }
                            .into_any()
                    }
                    Err(e) => {
                        let msg = format!("Failed to load commit page: {e}");
                        view! {
                            <Layout breadcrumbs=vec![] notification=notification ui_locked=ui_locked>
                                <div class="qui-page-commit container">
                                    <p>{msg}</p>
                                </div>
                            </Layout>
                        }
                            .into_any()
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
    let workflow_null = RwSignal::new(
        workflow
            .as_ref()
            .map(|w| w.id.is_none())
            .unwrap_or(true),
    );

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
            match commands::package_commit(ns.clone(), msg, meta, wf).await
            {
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
        None => {
            view! {
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
            .into_any()
        }
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
        let ns_for_folder = namespace.clone();
        let on_open_folder = move |_| {
            let ns = ns_for_folder.clone();
            leptos::task::spawn_local(async move {
                match commands::open_in_file_browser(ns).await {
                    Ok(msg) => notification.set(Some(Notification::Success(msg))),
                    Err(e) => notification.set(Some(Notification::Error(e))),
                }
            });
        };

        let origin_for_catalog = origin_url.clone();
        let on_open_catalog = move |_| {
            if let Some(url) = origin_for_catalog.clone() {
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
                <button class="qui-button" type="button" on:click=on_open_folder>
                    <img class="qui-icon" src="/assets/img/icons/folder_open.svg" />
                    <span>"Open"</span>
                </button>
            </li>
            {if has_catalog {
                view! {
                    <li>
                        <button
                            class="qui-button"
                            type="button"
                            prop:disabled=catalog_disabled
                            on:click=on_open_catalog
                        >
                            <img class="qui-icon" src="/assets/img/icons/open_in_browser.svg" />
                            <span>"Open in Catalog"</span>
                        </button>
                    </li>
                }
                .into_any()
            } else {
                view! {}.into_any()
            }}
            <li>
                <button class="qui-button" type="button" on:click=on_uninstall>
                    <img class="qui-icon" src="/assets/img/icons/block.svg" />
                    <span>"Remove"</span>
                </button>
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
    let show_catalog = (entry.status == "remote" || entry.status == "pristine")
        && entry.origin_url.is_some();

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

    let origin_for_catalog = entry.origin_url.clone();
    let on_catalog = move |_| {
        if let Some(url) = origin_for_catalog.clone() {
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
                                <button class="qui-button small" type="button" on:click=on_open>
                                    <img class="qui-icon" src="/assets/img/icons/open_in_new.svg" />
                                    <span>"Open"</span>
                                </button>
                            </li>
                            <li class="menu-item">
                                <button class="qui-button small" type="button" on:click=on_reveal>
                                    <img class="qui-icon" src="/assets/img/icons/folder_open.svg" />
                                    <span>"Reveal"</span>
                                </button>
                            </li>
                        }
                        .into_any()
                    } else {
                        view! {}.into_any()
                    }}
                    {if show_catalog {
                        view! {
                            <li class="menu-item">
                                <button class="qui-button small" type="button" on:click=on_catalog>
                                    <img class="qui-icon" src="/assets/img/icons/open_in_browser.svg" />
                                    <span>"Open in Catalog"</span>
                                </button>
                            </li>
                        }
                        .into_any()
                    } else {
                        view! {}.into_any()
                    }}
                    {if is_junky {
                        view! {
                            <li class="menu-item">
                                <button class="qui-button small" type="button" on:click=on_ignore>
                                    <img class="qui-icon" src="/assets/img/icons/visibility_off.svg" />
                                    <span>"Ignore"</span>
                                </button>
                            </li>
                        }
                        .into_any()
                    } else {
                        view! {}.into_any()
                    }}
                    {if is_ignored {
                        view! {
                            <li class="menu-item">
                                <button class="qui-button small" type="button" on:click=on_unignore>
                                    <img class="qui-icon" src="/assets/img/icons/visibility.svg" />
                                    <span>"Ignored"</span>
                                </button>
                            </li>
                        }
                        .into_any()
                    } else {
                        view! {}.into_any()
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
    let cb = Closure::<dyn Fn()>::new(move || {
        create_json_editor_js(id, &init_value);
    });
    // Schedule after the current frame so Leptos has committed the DOM.
    let _ = web_sys::window()
        .unwrap()
        .request_animation_frame(cb.as_ref().unchecked_ref());
    cb.forget();

    on_cleanup(move || {
        destroy_json_editor_js(id);
    });

    view! {
        <div class="metadata" id=id></div>
    }
}

// ── Ignore popup ──

#[derive(Clone, Debug)]
struct IgnorePopupData {
    namespace: String,
    path: String,
    suggested_pattern: String,
}

#[derive(Clone)]
enum IgnoreHint {
    WillBeIgnored(String),
    OnlyExact(String),
    NoMatch(String),
}

#[component]
fn IgnorePopup(
    data: IgnorePopupData,
    notification: RwSignal<Option<Notification>>,
    on_close: impl Fn() + Clone + 'static,
) -> impl IntoView {
    let pattern = RwSignal::new(data.suggested_pattern.clone());
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
    let on_close_for_submit = on_close.clone();
    let on_submit = move || {
        let p = pattern.get_untracked();
        if p.trim().is_empty() || submitting.get_untracked() {
            return;
        }
        submitting.set(true);
        let ns = ns_for_submit.clone();
        let on_close = on_close_for_submit.clone();
        leptos::task::spawn_local(async move {
            match commands::add_to_quiltignore(ns, p).await
            {
                Ok(msg) => {
                    notification.set(Some(Notification::Success(msg)));
                    on_close();
                    let _ = web_sys::window().and_then(|w| w.location().reload().ok());
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
                        <button
                            class="qui-button primary"
                            type="button"
                            prop:disabled=move || submitting.get()
                            on:click=on_submit_click
                        >
                            <span>"Add to .quiltignore"</span>
                        </button>
                        <button class="qui-button" type="button" on:click=on_cancel>
                            <span>"Cancel"</span>
                        </button>
                    </div>
                </div>
            </div>
        </div>
    }
}

// ── Unignore popup ──

#[derive(Clone, Debug)]
struct UnignorePopupData {
    namespace: String,
    pattern: String,
}

#[component]
fn UnignorePopup(
    data: UnignorePopupData,
    notification: RwSignal<Option<Notification>>,
    on_close: impl Fn() + Clone + 'static,
) -> impl IntoView {
    let ns = data.namespace.clone();
    let pattern_display = data.pattern.clone();

    let on_close_for_edit = on_close.clone();
    let on_edit = move |_| {
        let ns = ns.clone();
        let on_close = on_close_for_edit.clone();
        leptos::task::spawn_local(async move {
            match commands::open_in_default_application(ns, ".quiltignore".to_string()).await
            {
                Ok(msg) => notification.set(Some(Notification::Success(msg))),
                Err(e) => notification.set(Some(Notification::Error(e))),
            }
            on_close();
        });
    };

    view! {
        <div class="popup-overlay" on:click={
            let on_close = on_close.clone();
            move |_| on_close()
        }>
            <div class="popup-content" on:click=|ev| ev.stop_propagation()>
                <div class="unignore-popup">
                    <span>"Ignored by: "<span class="pattern-display">{pattern_display}</span></span>
                    <div>
                        <button class="qui-button primary" type="button" on:click=on_edit>
                            <span>"Edit .quiltignore"</span>
                        </button>
                    </div>
                </div>
            </div>
        </div>
    }
}

// ── Helpers ──

fn format_size(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "kB", "MB", "GB", "TB", "PB", "EB"];
    if bytes == 0 {
        return "0 B".to_string();
    }
    let mut value = bytes as f64;
    for unit in UNITS {
        if value < 1000.0 {
            if *unit == "B" {
                return format!("{value} {unit}");
            }
            return format!("{value:.2} {unit}");
        }
        value /= 1000.0;
    }
    format!("{value:.2} EB")
}


