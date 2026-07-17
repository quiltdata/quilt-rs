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
    /// Requested revision top-hash from a version-mismatch deep link.
    mismatch_requested: Option<String>,
    /// The requested revision's own bucket, so its message is fetched from the
    /// remote it actually lives on (not the installed package's remote).
    mismatch_bucket: Option<String>,
    /// The requested revision's catalog origin, if the deep link carried one.
    mismatch_catalog: Option<String>,
    /// True when the deep link resolved to a local-only package.
    local_only: bool,
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
                // ── Version-mismatch / local-only banner (deep link) ──
                {
                    let installed_hash = data.installed_hash.clone();
                    let installed_message = data.installed_message.clone();
                    let namespace_for_banner = namespace.clone();
                    let status_for_banner = status.clone();
                    move || {
                        if local_only {
                            return view! {
                                <div class="qui-status"><div class="root">
                                    <h2 class="description">
                                        "This package is installed locally without a remote origin. Showing the local version."
                                    </h2>
                                </div></div>
                            }.into_any();
                        }
                        let Some(requested) = mismatch_requested.clone() else {
                            return ().into_any();
                        };
                        // Phase 1: installed side, immediate.
                        let installed_label =
                            revision_label(&installed_message, &installed_hash);
                        // Phase 2: requested side, fetched lazily from the
                        // requested revision's own remote (bucket + catalog).
                        let requested_for_fetch = requested.clone();
                        let ns_for_fetch = namespace_for_banner.clone();
                        let bucket_for_fetch = mismatch_bucket.clone().unwrap_or_default();
                        let catalog_for_fetch = mismatch_catalog.clone();
                        let requested_msg = LocalResource::new(move || {
                            let ns = ns_for_fetch.clone();
                            let hash = requested_for_fetch.clone();
                            let bucket = bucket_for_fetch.clone();
                            let catalog = catalog_for_fetch.clone();
                            async move {
                                commands::get_revision_message(bucket, ns, hash, catalog).await
                            }
                        });
                        let requested_short: String = requested.chars().take(8).collect();
                        let requested_full = requested.clone();
                        // Reason line: always says the requested revision isn't
                        // installed; when there is no Pull button (any state but
                        // `behind`), it also says why. The `behind` StatusBanner
                        // below carries the Pull button and its own explanation.
                        let reason = match status_for_banner.as_str() {
                            "behind" => "The requested version isn't installed on this computer. You're seeing the version you have.",
                            "ahead" => "The requested version isn't installed on this computer. You have local changes that aren't on the remote yet.",
                            "diverged" => "The requested version isn't installed on this computer. Your local version has diverged from the remote — resolve that below.",
                            "up_to_date" => "The requested version isn't installed on this computer. You have the latest version installed, and that's what's shown.",
                            _ => "The requested version isn't installed on this computer, and the remote can't be checked right now.",
                        };
                        view! {
                            <div class="qui-status"><div class="root">
                                <div class="description">
                                    <div class="revision">
                                        <p class="revision-title">"Requested version"</p>
                                        <p class="revision-message">
                                            <Suspense fallback=move || view! {
                                                <span title=requested_full.clone()>{requested_short.clone()}</span>
                                            }>
                                                {
                                                    let requested = requested.clone();
                                                    move || {
                                                        let requested = requested.clone();
                                                        Suspend::new(async move {
                                                            let msg = requested_msg.await.ok().flatten();
                                                            let short: String = requested.chars().take(8).collect();
                                                            revision_label(&msg, &Some(requested.clone()))
                                                                .unwrap_or_else(|| view! {
                                                                    <span title=requested.clone()>{short}</span>
                                                                }.into_any())
                                                        })
                                                    }
                                                }
                                            </Suspense>
                                        </p>
                                    </div>
                                    <div class="revision">
                                        <p class="revision-title">"Installed version"</p>
                                        <p class="revision-message">{installed_label}</p>
                                    </div>
                                    <p class="detail">{reason}</p>
                                </div>
                            </div></div>
                        }.into_any()
                    }
                }

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
                has_local_commit=data.has_local_commit
                locked=remote_locked
                notification=notification
                refetch=refetch
                on_close=move || show_set_remote_popup.set(false)
            />
        </Show>
    }
}

/// A revision's display label: its manifest message with the full top-hash as
/// a hover tooltip, falling back to the 8-char short hash when the message is
/// empty. Returns `None` only when neither a message nor a hash is available.
fn revision_label(message: &Option<String>, hash: &Option<String>) -> Option<AnyView> {
    let title = hash.clone().unwrap_or_default();
    match message {
        Some(m) if !m.trim().is_empty() => {
            Some(view! { <span title=title>{m.clone()}</span> }.into_any())
        }
        _ => hash.as_ref().map(|h| {
            let short: String = h.chars().take(8).collect();
            view! { <span title=h.clone()>{short}</span> }.into_any()
        }),
    }
}
