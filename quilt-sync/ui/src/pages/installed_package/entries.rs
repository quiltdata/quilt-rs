use leptos::prelude::*;

use quilt_uri::S3PackageUri;

use crate::commands::{self, EntryData};
use crate::components::buttons;
use crate::components::{IgnorePopupData, Notification, UnignorePopupData};
use crate::util;
use crate::util::format_size;

// ── Entries toolbar ──

#[component]
pub(super) fn EntriesToolbar(
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
                    ().into_any()
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
            </div>
        </Show>
    }
}

// ── Entry row ──

#[component]
pub(super) fn EntryRow(
    index: usize,
    entry: EntryData,
    pkg_uri: Option<S3PackageUri>,
    checked_indices: RwSignal<Vec<usize>>,
    notification: RwSignal<Option<Notification>>,
    show_ignore_popup: RwSignal<Option<IgnorePopupData>>,
    show_unignore_popup: RwSignal<Option<UnignorePopupData>>,
) -> impl IntoView {
    let EntryData {
        filename,
        size,
        status,
        junky_pattern,
        ignored_by,
        namespace,
    } = entry;

    let is_remote = status == "remote";
    let is_deleted = status == "deleted";
    let is_ignored = ignored_by.is_some();
    let is_junky = junky_pattern.is_some();

    let class_mods = {
        let mut classes = vec![status.as_str()];
        if is_junky {
            classes.push("junky");
        }
        if is_ignored {
            classes.push("ignored");
        }
        format!("qui-entry {}", classes.join(" "))
    };

    let status_display = match status.as_str() {
        "added" => "New",
        "deleted" => "Deleted",
        "modified" => "Modified",
        "pristine" => "Downloaded",
        "remote" => "Remote",
        _ => "",
    };

    let size_display = format_size(size);
    let status_text = format!("{status_display}, {size_display}");

    let filename_display = filename.clone();
    let filename_title = filename.clone();

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
    let show_catalog = (is_remote || status == "pristine")
        && pkg_uri.as_ref().is_some_and(|u| u.catalog.is_some());

    let ns_for_open = namespace.clone();
    let path_for_open = filename.clone();
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

    let ns_for_reveal = namespace.clone();
    let path_for_reveal = filename.clone();
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

    let path_for_catalog = filename.clone();
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

    let ns_for_ignore = namespace.clone();
    let path_for_ignore = filename;
    let on_ignore = move |_| {
        if let Some(pattern) = junky_pattern.clone() {
            show_ignore_popup.set(Some(IgnorePopupData {
                namespace: ns_for_ignore.clone(),
                path: path_for_ignore.clone(),
                suggested_pattern: pattern,
            }));
        }
    };

    let ns_for_unignore = namespace;
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
                                <buttons::Open on_click=on_open small=true />
                            </li>
                            <li class="menu-item">
                                <buttons::Reveal on_click=on_reveal small=true />
                            </li>
                        }.into_any()
                    } else {
                        ().into_any()
                    }}
                    {if show_catalog {
                        view! {
                            <li class="menu-item">
                                <buttons::OpenInCatalog on_click=on_open_catalog small=true />
                            </li>
                        }.into_any()
                    } else {
                        ().into_any()
                    }}
                    {if is_junky {
                        view! {
                            <li class="menu-item">
                                <buttons::Ignore on_click=on_ignore small=true />
                            </li>
                        }.into_any()
                    } else {
                        ().into_any()
                    }}
                    {if is_ignored {
                        view! {
                            <li class="menu-item">
                                <buttons::Unignore on_click=on_unignore small=true />
                            </li>
                        }.into_any()
                    } else {
                        ().into_any()
                    }}
                </ul>
            </div>
        </div>
    }
}
