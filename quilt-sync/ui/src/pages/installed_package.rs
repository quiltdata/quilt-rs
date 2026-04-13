use leptos::prelude::*;
use leptos_router::hooks::{use_navigate, use_query_map};

use crate::commands::{self, EntryData, InstalledPackageData};
use crate::components::layout::{BreadcrumbItem, BreadcrumbLink};
use crate::components::{
    IgnorePopup, IgnorePopupData, Layout, Notification, Spinner, ToolbarActions, UnignorePopup,
    UnignorePopupData,
};
use crate::util::{format_size, is_valid_hostname, urlencoding};

// ── Installed Package page ──

#[component]
pub fn InstalledPackage() -> impl IntoView {
    let notification = RwSignal::new(None);
    let ui_locked = RwSignal::new(false);
    let refetch = Trigger::new();

    let query = use_query_map();
    let data = LocalResource::new(move || {
        refetch.track();
        let namespace = query.read().get("namespace").unwrap_or_default();
        let filter = query.read().get("filter");
        async move {
            commands::get_installed_package_data(namespace, filter).await
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
                        let breadcrumbs = vec![
                            BreadcrumbItem::Link(BreadcrumbLink {
                                href: "/installed-packages-list".to_string(),
                                title: String::new(),
                            }),
                            BreadcrumbItem::Current(ns),
                        ];
                        let actions = build_toolbar_actions(&d, notification, ui_locked);
                        view! {
                            <Layout breadcrumbs=breadcrumbs notification=notification actions=actions ui_locked=ui_locked>
                                <InstalledPackageContent data=d notification=notification ui_locked=ui_locked refetch=refetch />
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
fn InstalledPackageContent(
    data: InstalledPackageData,
    notification: RwSignal<Option<Notification>>,
    ui_locked: RwSignal<bool>,
    refetch: Trigger,
) -> impl IntoView {
    let filter_unmodified = RwSignal::new(data.filter_unmodified);
    let filter_ignored = RwSignal::new(data.filter_ignored);
    let show_ignore_popup = RwSignal::new(None::<IgnorePopupData>);
    let show_unignore_popup = RwSignal::new(None::<UnignorePopupData>);
    let show_origin_popup = RwSignal::new(false);

    let namespace = data.namespace.clone();
    let uri = data.uri.clone();
    let status = data.status.clone();
    let origin_host = data.origin_host.clone();
    let entries = data.entries;
    let has_remote_entries = data.has_remote_entries;
    let ignored_count = data.ignored_count;
    let unmodified_count = data.unmodified_count;

    // Track which remote entries are checked (by index) — all selected by default
    let initial_checked: Vec<usize> = entries
        .iter()
        .enumerate()
        .filter(|(_, e)| e.status == "remote")
        .map(|(i, _)| i)
        .collect();
    let checked_indices = RwSignal::new(initial_checked);

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

    // Count checked remote entries
    let checked_count = Memo::new(move |_| checked_indices.get().len());

    let show_toolbar = has_remote_entries || ignored_count > 0 || unmodified_count > 0;

    // Install selected paths
    let uri_for_install = uri.clone();
    let entries_for_install = entries.clone();
    let on_install_paths = move |_| {
        let uri = uri_for_install.clone();
        let indices = checked_indices.get_untracked();
        let paths: Vec<String> = indices
            .iter()
            .filter_map(|&i| entries_for_install.get(i))
            .filter(|e| e.status == "remote")
            .map(|e| e.filename.clone())
            .collect();
        if paths.is_empty() {
            return;
        }
        let notification = notification;
        ui_locked.set(true);
        leptos::task::spawn_local(async move {
            match commands::package_install_paths(uri, paths).await
            {
                Ok(msg) => {
                    notification.set(Some(Notification::Success(msg)));
                    refetch.notify();
                }
                Err(e) => {
                    ui_locked.set(false);
                    notification.set(Some(Notification::Error(e)));
                }
            }
        });
    };

    // Select all
    let entries_for_select = entries.clone();
    let on_select_all = move |_: leptos::ev::Event| {
        let current = checked_indices.get_untracked();
        let remote_indices: Vec<usize> = entries_for_select
            .iter()
            .enumerate()
            .filter(|(_, e)| e.status == "remote")
            .map(|(i, _)| i)
            .collect();
        if current.len() == remote_indices.len() {
            checked_indices.set(Vec::new());
        } else {
            checked_indices.set(remote_indices);
        }
    };

    let entries_for_all_check = entries.clone();
    let all_remote_selected = Memo::new(move |_| {
        let checked = checked_indices.get();
        let remote_count = entries_for_all_check
            .iter()
            .filter(|e| e.status == "remote")
            .count();
        remote_count > 0 && checked.len() == remote_count
    });

    // Commit button: primary when no remote entries are checked
    let commit_href = format!(
        "/commit?namespace={}",
        namespace
    );
    let commit_href_clone = commit_href.clone();

    let ns_for_status = namespace.clone();
    let origin_host_for_status = origin_host.clone();
    let status_clone = status.clone();
    let show_commit = status != "error";

    view! {
        <div class="qui-page-installed-package">
            <div class="container">
                // ── Status banner ──
                <StatusBanner
                    namespace=ns_for_status
                    status=status_clone
                    origin_host=origin_host_for_status
                    notification=notification
                    ui_locked=ui_locked
                    refetch=refetch
                    show_origin_popup=show_origin_popup
                />

                // ── Entries form ──
                <div class="form" data-testid="installed-package-entries">
                    // ── Entries toolbar ──
                    <Show when=move || show_toolbar>
                        <EntriesToolbar
                            has_remote_entries=has_remote_entries
                            on_select_all=on_select_all.clone()
                            all_selected=all_remote_selected
                            checked_count=checked_count
                            on_install_paths=on_install_paths.clone()
                            filter_unmodified=filter_unmodified
                            filter_ignored=filter_ignored
                            ignored_count=ignored_count
                            unmodified_count=unmodified_count
                            with_status=matches!(data.status.as_str(), "ahead" | "behind" | "diverged" | "error")
                        />
                    </Show>

                    // ── Entry list ──
                    <div class="list">
                        <For
                            each=move || filtered_entries.get()
                            key=|(i, _)| *i
                            let:item
                        >
                            <EntryRow
                                index=item.0
                                entry=item.1
                                checked_indices=checked_indices
                                notification=notification
                                show_ignore_popup=show_ignore_popup
                                show_unignore_popup=show_unignore_popup
                            />
                        </For>
                    </div>
                </div>
            </div>
        </div>

        // ── Toolbar actions (rendered into Layout's toolbar area) ──
        // These are placed here as toolbar buttons via the header, but since
        // Layout doesn't have an actions prop, we use absolute positioning
        // or integrate into breadcrumbs. Actually, looking at the pattern,
        // the actions go in the toolbar above content.
        // Let me use a toolbar actions div instead.

        // ── Action bar: Commit ──
        <Show when=move || show_commit>
            {
                let has_changes = entries.iter().any(|e| {
                    matches!(e.status.as_str(), "added" | "modified" | "deleted")
                });
                let commit_btn_class = Memo::new(move |_| {
                    if has_changes && checked_count.get() == 0 {
                        "qui-button primary large"
                    } else {
                        "qui-button large"
                    }
                });
                let href = commit_href_clone.clone();
                view! {
                    <div class="qui-actionbar">
                        <a href=href>
                            <button class=move || commit_btn_class.get() type="button">
                                <span>"Create new revision"</span>
                                <img class="qui-icon" src="/assets/img/icons/arrow_forward.svg" />
                            </button>
                        </a>
                    </div>
                }
            }
        </Show>

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

        <Show when=move || show_origin_popup.get()>
            <SetOriginPopup
                namespace=data.namespace.clone()
                current_origin=data.origin_host.clone().unwrap_or_default()
                notification=notification
                refetch=refetch
                on_close=move || show_origin_popup.set(false)
            />
        </Show>
    }
}

// ── Toolbar actions (rendered into Layout's toolbar) ──

fn build_toolbar_actions(
    data: &InstalledPackageData,
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
                }.into_any()
            } else {
                view! {}.into_any()
            }}
            <li>
                <button class="qui-button" type="button" on:click=on_uninstall>
                    <img class="qui-icon" src="/assets/img/icons/block.svg" />
                    <span>"Remove"</span>
                </button>
            </li>
        }.into_any()
    })
}

// ── Status banner ──

#[component]
fn StatusBanner(
    namespace: String,
    status: String,
    origin_host: Option<String>,
    notification: RwSignal<Option<Notification>>,
    ui_locked: RwSignal<bool>,
    refetch: Trigger,
    show_origin_popup: RwSignal<bool>,
) -> impl IntoView {
    let ns = namespace.clone();
    let host = origin_host.clone();

    let content = match status.as_str() {
        "ahead" => {
            let ns = ns.clone();
            Some(view! {
                <StatusBannerInner description="Your commits are ahead of the remote">
                    <PushButton namespace=ns notification=notification ui_locked=ui_locked refetch=refetch />
                </StatusBannerInner>
            }.into_any())
        }
        "behind" => {
            let ns = ns.clone();
            Some(view! {
                <StatusBannerInner description="Your commits are behind the remote">
                    <PullButton namespace=ns notification=notification ui_locked=ui_locked refetch=refetch />
                </StatusBannerInner>
            }.into_any())
        }
        "diverged" => {
            let merge_href = format!("/merge?namespace={ns}");
            Some(view! {
                <StatusBannerInner description="Your commits are detached from the remote">
                    <a href=merge_href>
                        <button class="qui-button primary" type="button">
                            <span>"Merge"</span>
                        </button>
                    </a>
                </StatusBannerInner>
            }.into_any())
        }
        "error" => {
            match host {
                Some(ref h) => {
                    let back = format!("/installed-package?namespace={}&filter=unmodified", urlencoding(&ns));
                    let login_href = format!(
                        "/login?host={}&back={}",
                        h,
                        urlencoding(&back)
                    );
                    Some(view! {
                        <StatusBannerInner description="Unable to check remote status">
                            <a href=login_href>
                                <button class="qui-button warning" type="button">
                                    <img class="qui-icon" src="/assets/img/icons/warning.svg" />
                                    <span>"Login"</span>
                                </button>
                            </a>
                            <button
                                class="qui-button"
                                type="button"
                                on:click=move |_| show_origin_popup.set(true)
                            >
                                <span>"Change origin"</span>
                            </button>
                        </StatusBannerInner>
                    }.into_any())
                }
                None => {
                    Some(view! {
                        <StatusBannerInner description="No catalog origin configured">
                            <button
                                class="qui-button warning"
                                type="button"
                                on:click=move |_| show_origin_popup.set(true)
                            >
                                <img class="qui-icon" src="/assets/img/icons/warning.svg" />
                                <span>"Set origin"</span>
                            </button>
                        </StatusBannerInner>
                    }.into_any())
                }
            }
        }
        "local" if origin_host.is_some() => {
            let ns = ns.clone();
            Some(view! {
                <StatusBannerInner description="Push to remote">
                    <PushButton namespace=ns notification=notification ui_locked=ui_locked refetch=refetch />
                </StatusBannerInner>
            }.into_any())
        }
        _ => None,
    };

    view! {
        {content}
    }
}

#[component]
fn StatusBannerInner(
    description: &'static str,
    children: Children,
) -> impl IntoView {
    view! {
        <div class="qui-status">
            <div class="root">
                <h2 class="description">{description}</h2>
                <div class="action">
                    {children()}
                </div>
            </div>
        </div>
    }
}

#[component]
fn PushButton(
    namespace: String,
    notification: RwSignal<Option<Notification>>,
    ui_locked: RwSignal<bool>,
    refetch: Trigger,
) -> impl IntoView {
    let pushing = RwSignal::new(false);
    view! {
        <button
            class="qui-button primary"
            type="button"
            prop:disabled=move || pushing.get()
            on:click=move |_| {
                if pushing.get_untracked() { return; }
                pushing.set(true);
                ui_locked.set(true);
                let ns = namespace.clone();
                leptos::task::spawn_local(async move {
                    match commands::package_push(ns).await {
                        Ok(msg) => {
                            notification.set(Some(Notification::Success(msg)));
                            refetch.notify();
                        }
                        Err(e) => {
                            ui_locked.set(false);
                            notification.set(Some(Notification::Error(e)));
                            pushing.set(false);
                        }
                    }
                });
            }
        >
            <span>{move || if pushing.get() { "Pushing\u{2026}" } else { "Push" }}</span>
        </button>
    }
}

#[component]
fn PullButton(
    namespace: String,
    notification: RwSignal<Option<Notification>>,
    ui_locked: RwSignal<bool>,
    refetch: Trigger,
) -> impl IntoView {
    let pulling = RwSignal::new(false);
    view! {
        <button
            class="qui-button primary"
            type="button"
            prop:disabled=move || pulling.get()
            on:click=move |_| {
                if pulling.get_untracked() { return; }
                pulling.set(true);
                ui_locked.set(true);
                let ns = namespace.clone();
                leptos::task::spawn_local(async move {
                    match commands::package_pull(ns).await {
                        Ok(msg) => {
                            notification.set(Some(Notification::Success(msg)));
                            refetch.notify();
                        }
                        Err(e) => {
                            ui_locked.set(false);
                            notification.set(Some(Notification::Error(e)));
                            pulling.set(false);
                        }
                    }
                });
            }
        >
            <span>{move || if pulling.get() { "Pulling\u{2026}" } else { "Pull" }}</span>
        </button>
    }
}

// ── Entries toolbar ──

#[component]
fn EntriesToolbar(
    has_remote_entries: bool,
    on_select_all: impl Fn(leptos::ev::Event) + 'static,
    all_selected: Memo<bool>,
    checked_count: Memo<usize>,
    on_install_paths: impl Fn(leptos::ev::MouseEvent) + 'static,
    filter_unmodified: RwSignal<bool>,
    filter_ignored: RwSignal<bool>,
    ignored_count: usize,
    unmodified_count: usize,
    with_status: bool,
) -> impl IntoView {
    let toolbar_class = if with_status {
        "qui-entries-toolbar with-status"
    } else {
        "qui-entries-toolbar"
    };

    view! {
        <div class=toolbar_class>
            <div class="container">
                {if has_remote_entries {
                    {
                        let install_btn_class = Memo::new(move |_| {
                            if checked_count.get() > 0 {
                                "qui-button primary"
                            } else {
                                "qui-button"
                            }
                        });
                        view! {
                            <label class="select-all">
                                <input
                                    type="checkbox"
                                    prop:checked=move || all_selected.get()
                                    on:change=on_select_all
                                />
                                "Select all"
                            </label>
                            <button
                                class=move || install_btn_class.get()
                                type="button"
                                prop:disabled=move || checked_count.get() == 0
                                on:click=on_install_paths
                            >
                                <span>"Download selected paths"</span>
                            </button>
                        }.into_any()
                    }
                } else {
                    view! {}.into_any()
                }}
                <EntriesFilter
                    filter_unmodified=filter_unmodified
                    filter_ignored=filter_ignored
                    ignored_count=ignored_count
                    unmodified_count=unmodified_count
                />
            </div>
        </div>
    }
}

// ── Entries filter ──

#[component]
fn EntriesFilter(
    filter_unmodified: RwSignal<bool>,
    filter_ignored: RwSignal<bool>,
    ignored_count: usize,
    unmodified_count: usize,
) -> impl IntoView {
    let show_filter = ignored_count > 0 || unmodified_count > 0;

    view! {
        <Show when=move || show_filter>
            <div class="filter">
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
            </div>
        </Show>
    }
}

// ── Entry row ──

#[component]
fn EntryRow(
    index: usize,
    entry: EntryData,
    checked_indices: RwSignal<Vec<usize>>,
    notification: RwSignal<Option<Notification>>,
    show_ignore_popup: RwSignal<Option<IgnorePopupData>>,
    show_unignore_popup: RwSignal<Option<UnignorePopupData>>,
) -> impl IntoView {
    let is_remote = entry.status == "remote";
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

    // Checkbox state for remote entries
    let is_checked = Memo::new(move |_| {
        if !is_remote {
            return true; // non-remote always show as checked (disabled)
        }
        checked_indices.get().contains(&index)
    });

    let on_checkbox_change = move |_| {
        if !is_remote {
            return;
        }
        let mut indices = checked_indices.get_untracked();
        if let Some(pos) = indices.iter().position(|&i| i == index) {
            indices.remove(pos);
        } else {
            indices.push(index);
        }
        checked_indices.set(indices);
    };

    // Action buttons
    let show_open_reveal = !is_remote && !is_deleted && !is_ignored;
    let show_catalog = (is_remote || entry.status == "pristine") && entry.origin_url.is_some();

    let ns_for_open = entry.namespace.clone();
    let path_for_open = entry.filename.clone();
    let on_open = move |_| {
        let ns = ns_for_open.clone();
        let path = path_for_open.clone();
        let notification = notification;
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
        let notification = notification;
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
            <label class="avatar">
                <input
                    type="checkbox"
                    prop:checked=move || is_checked.get()
                    prop:disabled=!is_remote
                    on:change=on_checkbox_change
                />
            </label>

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
                        }.into_any()
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
                        }.into_any()
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
                        }.into_any()
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
                        }.into_any()
                    } else {
                        view! {}.into_any()
                    }}
                </ul>
            </div>
        </div>
    }
}


// ── Set origin popup ──

#[component]
fn SetOriginPopup(
    namespace: String,
    current_origin: String,
    notification: RwSignal<Option<Notification>>,
    refetch: Trigger,
    on_close: impl Fn() + Clone + 'static,
) -> impl IntoView {
    let origin = RwSignal::new(current_origin);
    let show_error = RwSignal::new(false);
    let submitting = RwSignal::new(false);

    let ns = namespace.clone();

    let on_close_submit = on_close.clone();
    let on_submit = move || {
        let value = origin.get_untracked().trim().to_string();
        if value.is_empty() || submitting.get_untracked() {
            return;
        }
        if !is_valid_hostname(&value) {
            show_error.set(true);
            return;
        }
        submitting.set(true);
        let ns = ns.clone();
        let on_close = on_close_submit.clone();
        leptos::task::spawn_local(async move {
            match commands::set_origin(ns, value).await
            {
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
                <div class="origin-form">
                    <label>"Catalog origin"</label>
                    <div class="origin-input-group">
                        <input
                            class="origin-input"
                            class:error=move || show_error.get()
                            type="text"
                            placeholder="open.quilt.bio"
                            prop:value=move || origin.get()
                            on:input=move |ev| {
                                origin.set(event_target_value(&ev));
                                show_error.set(false);
                            }
                            on:keydown=on_keydown
                        />
                        <span
                            class="origin-hint"
                            class:visible=move || show_error.get()
                        >
                            "Enter a valid hostname, e.g. open.quilt.bio"
                        </span>
                    </div>
                    <div class="origin-form-actions">
                        <button
                            class="qui-button primary"
                            type="button"
                            prop:disabled=move || submitting.get()
                            on:click=on_submit_click
                        >
                            <span>"Submit"</span>
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


