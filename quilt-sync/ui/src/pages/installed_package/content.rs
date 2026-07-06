use leptos::prelude::*;

use super::entries::{EntriesToolbar, EntryRow};
use super::status_banner::StatusBanner;
use crate::commands::{self, InstalledPackageData, PausedEvent};
use crate::components::buttons;
use crate::components::{
    IgnorePopup, IgnorePopupData, Notification, SetRemotePopup, UnignorePopup, UnignorePopupData,
};
use crate::util;
use crate::util::make_action;

// ── Main content ──

#[component]
pub(super) fn InstalledPackageContent(
    data: InstalledPackageData,
    notification: RwSignal<Option<Notification>>,
    ui_locked: RwSignal<bool>,
    refetch: Trigger,
    page_warning: Option<String>,
    show_set_remote_popup: RwSignal<bool>,
    paused_event: RwSignal<Option<PausedEvent>>,
) -> impl IntoView {
    let filter_unmodified = RwSignal::new(data.filter_unmodified);
    let filter_ignored = RwSignal::new(data.filter_ignored);
    let show_ignore_popup = RwSignal::new(None::<IgnorePopupData>);
    let show_unignore_popup = RwSignal::new(None::<UnignorePopupData>);

    let namespace = data.namespace.clone();
    let uri = data.uri.clone();
    let status = data.status.clone();
    let origin_host = uri.as_ref().and_then(util::host_str);
    let current_host = origin_host.clone();
    let current_bucket = uri.as_ref().and_then(util::bucket_str);
    let remote_locked = data.remote_locked;
    let entries = data.entries;
    let has_remote_entries = data.has_remote_entries;
    let ignored_count = data.ignored_count;
    let unmodified_count = data.unmodified_count;

    let has_changes = entries
        .iter()
        .any(|e| matches!(e.status.as_str(), "added" | "modified" | "deleted"));

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
        let Some(uri) = uri_for_install
            .as_ref()
            .map(std::string::ToString::to_string)
        else {
            return;
        };
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
            match commands::package_install_paths(uri, paths).await {
                Ok(msg) => {
                    ui_locked.set(false);
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
    let commit_href = format!("/commit?namespace={namespace}");
    let commit_href_clone = commit_href.clone();

    let ns_for_status = namespace.clone();
    let origin_host_for_status = origin_host.clone();
    let status_clone = status.clone();
    let show_commit = status != "error";
    let has_origin = origin_host.is_some();
    // Mirror the Publish gating from the Installed Packages List: Commit and
    // Push is offered only when there's a remote and something to ship.
    let is_publishable = has_origin
        && (status == "ahead" || (status == "up_to_date" && has_changes) || status == "local");

    view! {
        <div class="qui-page-installed-package">
            <div class="container">
                // ── Persistent page warning (e.g. version mismatch from deep link) ──
                {page_warning.map(|msg| view! {
                    <div class="qui-status">
                        <div class="root">
                            <h2 class="description">{msg}</h2>
                        </div>
                    </div>
                })}

                // ── Status banner ──
                <StatusBanner
                    namespace=ns_for_status
                    status=status_clone
                    origin_host=origin_host_for_status
                    has_changes=has_changes
                    paused_event=paused_event
                    notification=notification
                    ui_locked=ui_locked
                    refetch=refetch
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
                                pkg_uri=uri.clone()
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

        // ── Action bar: Commit (and optionally Commit and Push) ──
        <Show when=move || show_commit>
            {
                let href = commit_href_clone.clone();
                // When Commit and Push is present it takes the primary slot.
                // Otherwise fall back to the original heuristic: primary when
                // there are changes and no remote entries are queued for install.
                let revision_primary = Memo::new(move |_| {
                    !is_publishable && has_changes && checked_count.get() == 0
                });
                let ns_for_publish = namespace.clone();
                let (publish_busy, on_publish) = make_action(
                    move || {
                        let ns = ns_for_publish.clone();
                        async move { commands::package_publish(ns).await }
                    },
                    notification,
                    Some(ui_locked),
                    move || refetch.notify(),
                );
                view! {
                    <div class="qui-actionbar">
                        <buttons::CreateNewRevision href=href primary=revision_primary />
                        {is_publishable.then(|| view! {
                            <span class="actions-divider">"or"</span>
                            <buttons::CommitAndPush
                                on_click=on_publish
                                busy=publish_busy
                            />
                        })}
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

        <Show when=move || show_set_remote_popup.get()>
            <SetRemotePopup
                namespace=data.namespace.clone()
                current_host=current_host.clone()
                current_bucket=current_bucket.clone()
                locked=remote_locked
                notification=notification
                refetch=refetch
                on_close=move || show_set_remote_popup.set(false)
            />
        </Show>
    }
}
