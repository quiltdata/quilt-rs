use std::collections::BTreeSet;

use leptos::prelude::*;

use super::entries::{EntriesToolbar, EntryRow};
use super::selection::{
    RemoteSelection, all_selected, partially_selected, resolve_for_scope, toggled_all,
};
use super::status_banner::StatusBanner;
use super::sync_scope::{ALL_DOWNLOADED_LINE, STANDING_SCOPE_LINE, SyncScopeBand};
use crate::commands::{self, InstalledPackageData, PausedEvent, PullCheck};
use crate::components::buttons;
use crate::components::{
    IgnorePopup, IgnorePopupData, Notification, SetRemotePopup, UnignorePopup, UnignorePopupData,
    with_popover,
};
use crate::util;
use crate::util::make_action;

// ── Main content ──

#[component]
#[allow(
    clippy::too_many_lines,
    reason = "declarative Leptos view; length is markup, not logic complexity"
)]
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
    /// Which remote entries are ticked for download. Owned by the page component
    /// above the resource boundary so it survives a refresh — see
    /// [`super::selection`].
    selection: RwSignal<RemoteSelection>,
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
    // Why the active role cannot reach this bucket, when it cannot. Outranks
    // the status-driven banner — see `remote_state_banner`.
    let no_access_reason = data.no_access_reason.clone();
    let banner_no_access_reason = no_access_reason.clone();
    let entries = data.entries;
    let has_remote_entries = data.has_remote_entries;
    let ignored_count = data.ignored_count;
    let unmodified_count = data.unmodified_count;

    let has_changes = entries
        .iter()
        .any(|e| matches!(e.status.as_str(), "added" | "modified" | "deleted"));

    // The remote entries this package currently offers, by path. Every read of
    // the selection is resolved against this set, which is what lets a preserved
    // selection stay honest with no cleanup pass: a name that has since been
    // installed or dropped is simply not in here any more. Held in a
    // `StoredValue` so the rows share one copy rather than cloning it per row.
    let remote_paths = StoredValue::new(
        entries
            .iter()
            .filter(|e| e.status == "remote")
            .map(|e| e.filename.clone())
            .collect::<BTreeSet<String>>(),
    );

    // The package's sync scope, needed above because the derived selection
    // below is scope-aware: whole-package scope has no per-file choice to
    // honour, so a subset left over from individual-file mode must not narrow
    // what the download control sends, enables on, or draws ticked.
    let scope_gate = data.entire_package_sync_enabled;
    let syncs_entire_package = data.syncs_entire_package;
    let whole_package = scope_gate && syncs_entire_package;

    // The one derived selection. The header checkbox and every row read *this*
    // and store nothing of their own, so they cannot disagree with each other —
    // before, they were two independent derivations over one index vector, and
    // their agreement rested on both being rebuilt at once.
    let selected = Memo::new(move |_| {
        remote_paths
            .with_value(|remote| selection.with(|s| resolve_for_scope(s, remote, whole_package)))
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

    // Count checked remote entries
    let checked_count = Memo::new(move |_| selected.with(BTreeSet::len));

    let show_toolbar = has_remote_entries || ignored_count > 0 || unmodified_count > 0;
    // The scope band renders regardless of `has_remote_entries`: once a package
    // has caught up there is nothing left to list, and the band is then the only
    // thing on screen stating a scope that keeps applying — and the only way to
    // turn it back off.
    let namespace_for_scope = data.namespace.clone();
    // One value, both sticky bands. They stack by deriving their `top` from
    // it, so a disagreement here is the two overlapping.
    let with_status = matches!(
        data.status.as_str(),
        "ahead" | "behind" | "diverged" | "error"
    ) || no_access_reason.is_some();
    let pending_count = remote_paths.with_value(std::collections::BTreeSet::len);
    // What fills the toolbar's left slot once there is nothing to download.
    // `None` off the gate: an un-gated package's page stays exactly as it was.
    let slot_caption = match (scope_gate, whole_package) {
        (false, _) => None,
        (true, true) => Some(STANDING_SCOPE_LINE),
        (true, false) => Some(ALL_DOWNLOADED_LINE),
    };

    // Install selected paths. The resolved selection is already remote-only and
    // already narrowed to what the package still offers, so it is the path list
    // verbatim — no index lookup left to mis-resolve.
    let uri_for_install = uri.clone();
    let on_install_paths = move |_| {
        let Some(uri) = uri_for_install
            .as_ref()
            .map(std::string::ToString::to_string)
        else {
            return;
        };
        let paths: Vec<String> = selected.get_untracked().into_iter().collect();
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

    // Select all — clears a full selection, otherwise takes everything.
    let on_select_all = move |_: leptos::ev::Event| {
        let current = selection.get_untracked();
        selection.set(remote_paths.with_value(|remote| toggled_all(&current, remote)));
    };

    let all_remote_selected = Memo::new(move |_| {
        remote_paths.with_value(|remote| selected.with(|s| all_selected(s, remote)))
    });
    // Drives the header checkbox's indeterminate state: a partial selection now
    // outlives a refresh, so drawing it as an empty box would be a standing claim
    // that nothing is selected.
    let some_remote_selected = Memo::new(move |_| {
        remote_paths.with_value(|remote| selected.with(|s| partially_selected(s, remote)))
    });

    // Commit button: primary when no remote entries are checked
    let commit_href = format!("/commit?namespace={namespace}");
    let commit_href_clone = commit_href.clone();

    let ns_for_status = namespace.clone();
    let uri_for_status = uri.clone();
    let uri_for_actions = uri.clone();
    let status_clone = status.clone();

    // Two-phase Pull affordance: the banner renders immediately from `status`;
    // when the package is `behind`, the dry-run pull outcome is fetched
    // asynchronously and fills in the Pull button's enabled state and copy. The
    // resource yields a `PullCheck`: `Loading` until the outcome resolves
    // (genuine in-flight state → "Checking for updates…"), `Failed` on a fetch
    // error (→ "Couldn't check for updates." with a retry), or `Ready`. The
    // `pull_retry` trigger re-runs the dry-run so one network blip no longer
    // strands the button on "Checking…" forever. A non-behind status never
    // queries: the outcome only gates the Pull button.
    let ns_for_outcome = namespace.clone();
    let status_for_outcome = status.clone();
    let pull_retry = Trigger::new();
    let pull_outcome_res = LocalResource::new(move || {
        pull_retry.track();
        let ns = ns_for_outcome.clone();
        let is_behind = status_for_outcome == "behind";
        async move {
            if is_behind {
                match commands::package_pull_outcome(ns).await {
                    Ok(outcome) => PullCheck::Ready(outcome),
                    Err(_) => PullCheck::Failed,
                }
            } else {
                PullCheck::Loading
            }
        }
    });
    let pull_check = Signal::derive(move || pull_outcome_res.get().unwrap_or(PullCheck::Loading));
    let show_commit = status != "error";
    let has_origin = origin_host.is_some();
    // Mirror the Publish gating from the Installed Packages List: Commit and
    // Push is offered only when there's a remote and something to ship.
    let is_publishable = has_origin
        && (status == "ahead" || (status == "up_to_date" && has_changes) || status == "local");
    // A role denial disables the action bar's *action* — Commit and Push —
    // and gives it a tooltip. Disabled, not hidden: the user opened this page
    // deliberately, and a button that simply vanishes explains nothing.
    // "Create new revision" is navigation and stays live (see
    // `commit_affordance_disabled`).
    let commit_denied = commit_affordance_disabled(no_access_reason.as_deref());
    let commit_hint = util::commit_denied_hint(no_access_reason.as_deref());

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
                            revision_label(installed_message.as_deref(), installed_hash.as_deref());
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
                                                            revision_label(msg.as_deref(), Some(requested.as_str()))
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
                    uri=uri_for_status
                    no_access_reason=banner_no_access_reason
                    pull_check=pull_check
                    pull_retry=pull_retry
                    paused_event=paused_event
                    notification=notification
                    ui_locked=ui_locked
                    refetch=refetch
                />

                // ── Entries form ──
                <div class="form" data-testid="installed-package-entries">
                    // ── Entries toolbar ──
                    <Show when=move || scope_gate>
                        <SyncScopeBand
                            namespace=namespace_for_scope.clone()
                            with_status=with_status
                            pending=pending_count
                            entire_package=syncs_entire_package
                            notification=notification
                            ui_locked=ui_locked
                            refetch=refetch
                        />
                    </Show>

                    <Show when=move || show_toolbar>
                        <EntriesToolbar
                            below_sync_scope=scope_gate
                            whole_package=whole_package
                            caption=slot_caption
                            has_remote_entries=has_remote_entries
                            on_select_all=on_select_all
                            all_selected=all_remote_selected
                            partially_selected=some_remote_selected
                            checked_count=checked_count
                            on_install_paths=on_install_paths.clone()
                            filter_unmodified=filter_unmodified
                            filter_ignored=filter_ignored
                            ignored_count=ignored_count
                            unmodified_count=unmodified_count
                            with_status=with_status
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
                                whole_package=whole_package
                                entry=item.1
                                pkg_uri=uri.clone()
                                selection=selection
                                selected=selected
                                remote_paths=remote_paths
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
                let commit_hint = commit_hint.clone();
                // When Commit and Push is present it takes the primary slot.
                // Otherwise fall back to the original heuristic: primary when
                // there are changes and no remote entries are queued for install.
                let revision_primary = Memo::new(move |_| {
                    !is_publishable && has_changes && checked_count.get() == 0
                });
                let ns_for_publish = namespace.clone();
                let uri_for_publish = uri_for_actions.clone();
                let (publish_busy, on_publish) = make_action(
                    move || {
                        let ns = ns_for_publish.clone();
                        let uri = uri_for_publish.clone();
                        async move { commands::package_publish(ns, uri).await }
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
                            {with_popover(
                                commit_hint,
                                view! {
                                    <buttons::CommitAndPush
                                        on_click=on_publish
                                        busy=publish_busy
                                        disabled=commit_denied
                                    />
                                }
                                    .into_any(),
                            )}
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

/// Whether this page's commit *action* — "Commit and Push" — must be inert.
///
/// Committing looks like offline work but is not: the workflow quality gate
/// reads the bucket's `.quilt/workflows/config.yml` before any manifest is
/// written, so under a role that cannot read the bucket every commit is a 403.
/// A page the user navigated to disables and explains rather than hiding,
/// which is what [`crate::util::commit_denied_hint`] supplies. (The packages
/// *list* hides its affordance instead: a list row carries its own visible
/// denial reason, and an inert button there would be noise.)
///
/// Deliberately narrow: this gates actions that cannot succeed, never
/// navigation. "Create new revision" is a link to the commit page, and that
/// page opens fine under a denial — it shows the banner and disables its own
/// commit buttons — so the user can still read their changes and the
/// explanation lands at the point of action instead of one screen earlier.
///
/// A pure seam so the contrast (denied disables, readable does not) is
/// testable without a DOM, matching `publish_affordance` on the list page.
fn commit_affordance_disabled(no_access_reason: Option<&str>) -> bool {
    no_access_reason.is_some()
}

/// A revision's display label: its manifest message with the full top-hash as
/// a hover tooltip, falling back to the 8-char short hash when the message is
/// empty. Returns `None` only when neither a message nor a hash is available.
fn revision_label(message: Option<&str>, hash: Option<&str>) -> Option<AnyView> {
    let title = hash.unwrap_or_default().to_string();
    match message {
        Some(m) if !m.trim().is_empty() => {
            Some(view! { <span title=title>{m.to_string()}</span> }.into_any())
        }
        _ => hash.map(|h| {
            let short: String = h.chars().take(8).collect();
            view! { <span title=h.to_string()>{short}</span> }.into_any()
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::commit_affordance_disabled;
    use crate::util::commit_denied_hint;

    const DENIED: &str = "Current role ReadOnly has no access to this bucket";

    /// The page's own source, so the markup rule below can be checked without
    /// a DOM: these tests run on the host target, where the Leptos view is
    /// never rendered.
    const SOURCE: &str = include_str!("content.rs");

    /// A denied bucket makes the page's commit *action* inert, and — the
    /// contrast that keeps this test honest — an otherwise identical package
    /// on a readable bucket leaves it live.
    #[test]
    fn a_denied_bucket_disables_the_commit_action_a_readable_one_does_not() {
        assert!(commit_affordance_disabled(Some(DENIED)));
        assert!(!commit_affordance_disabled(None));
    }

    /// The rule is "disable actions that cannot succeed, never navigation".
    /// "Create new revision" is a link to the commit page, which opens fine
    /// under a denial and explains itself there, so it must stay a plain,
    /// followable link — no `disabled`, no tooltip wrapper pre-empting it one
    /// screen early. Pinned against the markup because that is where someone
    /// would undo it while "fixing the inconsistency" with Commit and Push.
    #[test]
    fn the_link_to_the_commit_page_is_never_disabled() {
        let bar = action_bar_markup();
        let start = bar
            .find("<buttons::CreateNewRevision")
            .expect("the action bar still renders the commit-page link");
        let (before, from_link) = bar.split_at(start);
        let element = &from_link[..from_link
            .find("/>")
            .expect("the link element is self-closing")];

        assert!(
            !element.contains("disabled"),
            "the commit-page link must stay followable, got: {element}"
        );
        assert!(
            !before.contains("with_popover"),
            "the commit-page link must not be wrapped in a denial tooltip"
        );
        // The contrast: the action *after* it does carry one, so this test
        // cannot pass merely because every tooltip was deleted.
        assert!(
            from_link.contains("with_popover"),
            "Commit and Push still explains its denial"
        );
    }

    /// The disabled action is not silent: the same denial supplies the
    /// tooltip naming the role and the fix, and a readable bucket has none.
    #[test]
    fn the_disabled_commit_action_explains_itself() {
        assert_eq!(
            commit_denied_hint(Some(DENIED)).as_deref(),
            Some("Current role ReadOnly has no access to this bucket. Switch role to commit.")
        );
        assert_eq!(commit_denied_hint(None), None);
    }

    /// The action bar's markup, sliced out of [`SOURCE`] so the assertions
    /// above never read this test module's own text.
    fn action_bar_markup() -> &'static str {
        let start = SOURCE
            .find(r#"<div class="qui-actionbar">"#)
            .expect("the page still renders an action bar");
        let bar = &SOURCE[start..];
        &bar[..bar.find("</div>").expect("the action bar is closed")]
    }
}
