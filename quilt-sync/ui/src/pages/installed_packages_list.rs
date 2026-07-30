use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use leptos::prelude::*;

use crate::commands::{
    self, AUTOSYNC_PAUSED_EVENT, AUTOSYNC_PUBLISHED_EVENT, PACKAGE_STATUS_EVENT, PackageItemData,
    PackageStatusEvent, PausedEvent, PublishedEvent, PullCheck, PullOutcome,
};
use crate::components::buttons;
use crate::components::layout::BreadcrumbItem;
use crate::components::{
    Layout, Notification, SetRemotePopup, SetRemotePopupData, Spinner, ToolbarActions,
};
use crate::tauri as tauri_bridge;
use crate::util;
use crate::util::make_action;

/// Latest `package-status-changed` event received from the backend.
///
/// One listener is registered per page mount and writes to this signal;
/// each `PackageItem` row reads it and applies updates whose namespace
/// matches its own. Stored in a `RwSignal<Option<...>>` — `None` is the
/// initial "no event yet" state.
type StatusEventSignal = RwSignal<Option<PackageStatusEvent>>;

/// Derive the third-line hint for a package row from its current status,
/// whether it has a catalog host configured, and any autosync pause
/// message. Returns `None` for a healthy row (no third line, not red).
///
/// The guidance line shown above a paused row's reason: paused rows stay
/// paused until the user acts, so it names the resume action rather than
/// leaving them to wonder why autosync stopped.
const PAUSED_GUIDANCE: &str = "Autosync paused. Resolve the issue, then push manually to resume.";

/// The pull-conflict guidance line: names the conflicting files and points at
/// the commit → merge-page remediation, matching the manual-pull `Blocked`
/// copy so both paths read the same. "Push manually to resume" (the generic
/// paused guidance) is the wrong fix for a pull conflict, so this replaces it
/// whenever the pause reason is `pullConflict`. Falls back to a file-less
/// phrasing when the reason message is empty.
fn pull_conflict_hint(files: Option<&str>) -> String {
    match files {
        Some(f) if !f.is_empty() => {
            format!("Conflicts in {f}. Commit your changes to resolve them on the merge page.")
        }
        _ => "Pull conflict. Commit your changes to resolve it on the merge page.".to_string(),
    }
}

/// The one or two hint lines shown under a row's URI, error-coloured; empty
/// when the row is healthy (which is also the not-red condition).
///
/// Pure so it can be unit-tested. A `pullConflict` pause shows the
/// conflict-specific guidance (files + merge page); any other paused row shows
/// the generic guidance line followed by the raw refusal reason (when one is
/// known); an `error` row shows a sign-in or no-remote hint depending on
/// whether a remote host exists.
///
/// `no_access_reason` outranks the error hint. A row the active role cannot
/// reach is not a broken session, and telling the user to sign in again puts
/// them in a loop that cannot end: the re-vend returns the same denied role.
fn hint_lines(
    status: &str,
    has_host: bool,
    paused_kind: Option<&str>,
    paused_message: Option<&str>,
    no_access_reason: Option<&str>,
) -> Vec<String> {
    if let Some(reason) = no_access_reason {
        return vec![reason.to_string()];
    }
    if status == "paused" || paused_message.is_some() {
        if paused_kind == Some("pullConflict") {
            return vec![pull_conflict_hint(paused_message)];
        }
        // Same reasoning as the pull conflict: the generic guidance names an
        // action that cannot work. A manual push is signed by the role that
        // was just refused.
        if paused_kind == Some("roleDenied") {
            return vec![util::role_denied_hint(paused_message)];
        }
        let mut lines = vec![PAUSED_GUIDANCE.to_string()];
        lines.extend(paused_message.map(str::to_string));
        return lines;
    }
    if status == "error" {
        return vec![if has_host {
            "Unable to check remote status — sign in again".to_string()
        } else {
            "No remote configured".to_string()
        }];
    }
    Vec::new()
}

/// The autosync-paused toast for a namespace, or `None` when the pause needs
/// no toast. The classic refusals (pendingChanges/pendingCommit/diverged) are
/// already legible from the row's status string, so they stay silent; `other`,
/// `pullConflict` and `roleDenied` carry information the status string drops,
/// so all three toast — the pull conflict with its file names and merge-page
/// remediation, the denial with the role and the switch.
fn paused_toast(reason: &str, namespace: &str, message: Option<&str>) -> Option<String> {
    match reason {
        "pullConflict" => Some(format!(
            "Autosync paused {namespace} — {}",
            pull_conflict_hint(message)
        )),
        "roleDenied" => Some(format!(
            "Autosync paused {namespace} — {}",
            util::role_denied_hint(message)
        )),
        "other" => {
            let msg = message.unwrap_or("Autosync paused");
            Some(format!("Autosync paused {namespace} — {msg}"))
        }
        _ => None,
    }
}

/// Hover-popover text for the list-row Pull button: shown for a `Blocked`
/// outcome (naming the conflicting files and the commit → merge resolution
/// path) and for a `Failed` dry-run (an honest error, paired with the retry
/// affordance). `None` for every other (or still-loading) check, so no popover
/// renders and Pull is simply enabled or, while loading, disabled.
fn pull_popover(check: &PullCheck) -> Option<String> {
    match check {
        PullCheck::Failed => Some("Couldn't check for updates.".to_string()),
        PullCheck::Ready(PullOutcome::Blocked { conflicts }) => Some(format!(
            "Resolve conflicts in {} via commit \u{2192} merge",
            conflicts.join(", ")
        )),
        _ => None,
    }
}

/// Whether the row's Commit and Push affordance should render.
///
/// `has_changes` stays honest on a denied row — it is real local state, and
/// hiding it misrepresents the package. But the *action* is a different
/// question: a push always starts with an S3 read (the push gate's workflow
/// config check), so a role the row cannot read for can never complete one.
/// The state flag (`has_changes`) and the affordance gate (`no_access`) are
/// kept as separate terms for exactly this reason — deriving one from the
/// other collapses a case where they must disagree.
fn publish_affordance(status: &str, has_changes: bool, has_origin: bool, no_access: bool) -> bool {
    if no_access {
        return false;
    }
    let publishable_status = status == "ahead" || (status == "local" && has_origin);
    let up_to_date_with_changes = status == "up_to_date" && has_changes && has_origin;
    publishable_status || up_to_date_with_changes
}

// ── Installed Packages List page ──

#[component]
pub fn InstalledPackagesList() -> impl IntoView {
    let notification = RwSignal::new(None);
    let ui_locked = RwSignal::new(false);
    let refetch = Trigger::new();

    // Page-scoped status-changed bus: the autosync watcher emits
    // `package-status-changed` events; each row's Effect picks the
    // matching namespace and updates its local signals in place.
    let status_event: StatusEventSignal = RwSignal::new(None);

    // Namespaces last seen in the paused-relevant status (`"paused"`).
    // This drives *refetch decisions only* — never rendering — so the red
    // state can never go stale from it. It lets us fire `refetch` exactly
    // on transitions to/from paused (not on every routine status tick, of
    // which the watcher emits many). The list's `paused_reason` is always
    // re-read from the backend's authoritative paused map on that refetch.
    let paused_seen: RwSignal<HashSet<String>> = RwSignal::new(HashSet::new());

    // Status-changed listener. Updates the per-row status bus, and triggers
    // a list refetch when a namespace transitions to/from the paused status
    // so each row re-reads its `paused_reason` from the backend.
    let listener = tauri_bridge::listen::<PackageStatusEvent>(PACKAGE_STATUS_EVENT, move |ev| {
        let is_paused = ev.status == "paused";
        let was_paused = paused_seen.with_untracked(|seen| seen.contains(&ev.namespace));
        if is_paused != was_paused {
            let ns = ev.namespace.clone();
            paused_seen.update(|seen| {
                if is_paused {
                    seen.insert(ns);
                } else {
                    seen.remove(&ns);
                }
            });
            refetch.notify();
        }
        status_event.set(Some(ev));
    });
    on_cleanup(move || drop(listener));

    // Autosync publish events — emit a toast mirroring the manual
    // Commit & Push success notification, and refetch so the published
    // namespace's now-cleared pause is re-read from the backend and its
    // row stops rendering red.
    let publish_listener =
        tauri_bridge::listen::<PublishedEvent>(AUTOSYNC_PUBLISHED_EVENT, move |ev| {
            notification.set(Some(Notification::Success(format!(
                "Autosync published {} — {}",
                ev.namespace, ev.message,
            ))));
            refetch.notify();
        });
    on_cleanup(move || drop(publish_listener));

    // Autosync pause events — surface as a warning toast carrying the
    // reason. The detail page reads the same event to drive its
    // persistent banner, so the user sees both the immediate toast
    // (here) and a stable indicator when they open the package. The
    // refetch re-reads the backend paused map so the row's durable red
    // state reflects authoritative data (the toast is transient feedback).
    let paused_listener = tauri_bridge::listen::<PausedEvent>(AUTOSYNC_PAUSED_EVENT, move |ev| {
        // The classic refusal kinds (pendingChanges, pendingCommit,
        // diverged) are already legible from the per-row status string;
        // `other` and `pullConflict` carry information it drops, so only
        // those toast (see `paused_toast`).
        let Some(msg) = paused_toast(&ev.reason, &ev.namespace, ev.message.as_deref()) else {
            return;
        };
        notification.set(Some(Notification::Error(msg)));
        refetch.notify();
    });
    on_cleanup(move || drop(paused_listener));

    let data = LocalResource::new(move || {
        refetch.track();
        async { commands::get_installed_packages_list_data().await }
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
                        let breadcrumbs = vec![
                            BreadcrumbItem::Current("Packages".to_string()),
                        ];
                        let show_create_popup = RwSignal::new(false);
                        let show_create_popup_for_action = show_create_popup;
                        let actions = ToolbarActions::new(move || {
                            view! {
                                <li>
                                    <buttons::CreateLocalPackage
                                        on_click=move |_| show_create_popup_for_action.set(true)
                                        small=true
                                    />
                                </li>
                            }.into_any()
                        });
                        view! {
                            <Layout breadcrumbs=breadcrumbs notification=notification actions=actions ui_locked=ui_locked>
                                <PackagesListContent
                                    packages=d.packages
                                    notification=notification
                                    ui_locked=ui_locked
                                    refetch=refetch
                                    show_create_popup=show_create_popup
                                    status_event=status_event
                                />
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

// ── List content ──

#[component]
fn PackagesListContent(
    packages: Vec<PackageItemData>,
    notification: RwSignal<Option<Notification>>,
    ui_locked: RwSignal<bool>,
    refetch: Trigger,
    show_create_popup: RwSignal<bool>,
    status_event: StatusEventSignal,
) -> impl IntoView {
    let show_set_remote_popup = RwSignal::new(None::<SetRemotePopupData>);

    let is_empty = packages.is_empty();

    view! {
        <div class="qui-page-installed-packages-list">
            {if is_empty {
                view! {
                    <section class="empty">
                        <h1 class="empty-title">"You don't have any packages"</h1>
                        <p class="empty-title">"You can navigate to the file in Quilt Catalog and click on GET FILE and then OPEN IN QUILTSYNC buttons to install that package with file"</p>
                        <img class="empty-img" src="/assets/img/how-to-deep-link.png" />
                    </section>
                }.into_any()
            } else {
                view! {
                    <ul class="list">
                        {packages.into_iter().map(|pkg| {
                            view! {
                                <PackageItem
                                    data=pkg
                                    notification=notification
                                    ui_locked=ui_locked
                                    refetch=refetch
                                    show_set_remote_popup=show_set_remote_popup
                                    status_event=status_event
                                />
                            }
                        }).collect_view()}
                    </ul>
                }.into_any()
            }}
        </div>

        // ── Popups ──
        <Show when=move || show_create_popup.get()>
            <CreatePackagePopup
                notification=notification
                refetch=refetch
                on_close=move || show_create_popup.set(false)
            />
        </Show>

        <Show when=move || show_set_remote_popup.get().is_some()>
            {move || show_set_remote_popup.get().map(|data| {
                view! {
                    <SetRemotePopup
                        namespace=data.namespace
                        current_host=data.current_host
                        current_bucket=data.current_bucket
                        has_local_commit=data.has_local_commit
                        notification=notification
                        refetch=refetch
                        on_close=move || show_set_remote_popup.set(None)
                    />
                }
            })}
        </Show>
    }
}

// ── Package item row ──

#[component]
fn PackageItem(
    data: PackageItemData,
    notification: RwSignal<Option<Notification>>,
    ui_locked: RwSignal<bool>,
    refetch: Trigger,
    show_set_remote_popup: RwSignal<Option<SetRemotePopupData>>,
    status_event: StatusEventSignal,
) -> impl IntoView {
    let status = RwSignal::new(data.status.clone());
    let has_changes = RwSignal::new(data.has_changes);
    let refreshing = RwSignal::new(true);
    let refresh_error = RwSignal::new(None::<String>);
    // Seeded from the light phase's readable-bucket pre-filter, then
    // replaced wholesale by the heavy phase — see `AccessSignals::apply`.
    let access = AccessSignals {
        no_access: RwSignal::new(data.no_access),
        reason: RwSignal::new(data.no_access_reason.clone()),
        role_switch_host: RwSignal::new(data.role_switch_host.clone()),
    };
    let no_access_reason = access.reason;

    let cancelled = Arc::new(AtomicBool::new(false));
    let cancelled_flag = cancelled.clone();
    on_cleanup(move || cancelled.store(true, Ordering::Relaxed));

    let ns = data.namespace.clone();
    leptos::task::spawn_local(async move {
        let result = commands::refresh_package_status(ns).await;
        if cancelled_flag.load(Ordering::Relaxed) {
            return;
        }
        match result {
            Ok(fresh) => {
                status.set(fresh.status);
                has_changes.set(fresh.has_changes);
                access.apply(
                    fresh.no_access,
                    fresh.no_access_reason,
                    fresh.role_switch_host,
                );
            }
            Err(err) => refresh_error.set(Some(err)),
        }
        refreshing.set(false);
    });

    // Mirror autopull watcher events into this row's local signals.
    let ns_for_listener = data.namespace.clone();
    Effect::new(move |_| {
        if let Some(ev) = status_event.get()
            && ev.namespace == ns_for_listener
        {
            status.set(ev.status);
            has_changes.set(ev.has_changes);
            refresh_error.set(None);
        }
    });

    let pkg_href = format!(
        "/installed-package?namespace={}&filter=unmodified",
        data.namespace
    );

    let namespace_display = data.namespace.clone();
    let remote_display = data.remote_display.clone();

    // Attention hint lines under the URI. Red state and the lines are both
    // driven by `hint`: it is non-empty exactly when the row needs attention
    // (autosync-paused, or a remote error), empty for a healthy row.
    //
    // The pause reason comes straight from the fetched row data — the
    // backend's authoritative paused map — so there is no frontend cache to
    // go stale. A resume clears the reason at the source; the next refetch
    // drops it here. `status` stays reactive for the in-place status update.
    let has_host = data.uri.as_ref().and_then(util::host_str).is_some();
    let paused_reason = data.paused_reason.clone();
    let paused_kind = data.paused_kind.clone();
    let hint = Signal::derive(move || {
        no_access_reason.with(|reason| {
            status.with(|s| {
                hint_lines(
                    s,
                    has_host,
                    paused_kind.as_deref(),
                    paused_reason.as_deref(),
                    reason.as_deref(),
                )
            })
        })
    });

    // Two-phase Pull affordance: only the `behind` row action gates on it.
    // When the row is behind, the dry-run pull outcome is fetched and drives
    // the Pull button's enabled state and its conflict popover. The resource
    // re-runs when `status` changes (and when `pull_retry` fires), so it
    // clears/refetches as the row's status moves; the yielded `PullCheck`
    // distinguishes `Loading` (disabled, no popover) from `Failed` (disabled,
    // with a retry), so one network blip no longer strands the button.
    let ns_for_outcome = data.namespace.clone();
    let pull_retry = Trigger::new();
    let pull_outcome_res = LocalResource::new(move || {
        pull_retry.track();
        let ns = ns_for_outcome.clone();
        let is_behind = status.get() == "behind";
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

    // Build menu buttons
    let menu = build_package_menu(
        &data,
        status,
        has_changes,
        access,
        pull_check,
        pull_retry,
        refreshing,
        notification,
        ui_locked,
        refetch,
        show_set_remote_popup,
    );

    view! {
        <li class=move || if hint.with(Vec::is_empty) {
            "qui-installed-package-item"
        } else {
            "qui-installed-package-item error"
        }>
            <a class="link" href=pkg_href>
                <span class="item-primary">{namespace_display}</span>
                {remote_display.map(|uri| view! {
                    <span class="item-secondary">
                        <strong>"URI: "</strong>
                        {uri}
                    </span>
                })}
                {move || hint.get().into_iter().map(|line| view! {
                    <span class="item-error-hint">{line}</span>
                }).collect::<Vec<_>>()}
            </a>
            <Show when=move || refreshing.get()>
                <div class="q-spinner-inline" />
            </Show>
            <Show when=move || refresh_error.get().is_some()>
                <img class="refresh-warning-icon" src="/assets/img/icons/warning.svg" />
            </Show>
            <div class=move || if refreshing.get() { "menu refreshing" } else { "menu" }>
                <ul class="menu-list">
                    {menu}
                </ul>
            </div>
            <Show when=move || refreshing.get()>
                <div class="status-tooltip-wrapper">
                    <div class="status-tooltip">
                        "Syncing with remote and scanning local files for changes\u{2026}"
                    </div>
                </div>
            </Show>
            <Show when=move || refresh_error.get().is_some()>
                <div class="status-tooltip-wrapper">
                    <div class="status-tooltip error">
                        {move || refresh_error.get().unwrap_or_default()}
                    </div>
                </div>
            </Show>
        </li>
    }
}

/// The row's live access state, threaded into the menu so the error
/// affordances can tell a denied role from a broken session.
#[derive(Clone, Copy)]
struct AccessSignals {
    no_access: RwSignal<bool>,
    reason: RwSignal<Option<String>>,
    role_switch_host: RwSignal<Option<String>>,
}

impl AccessSignals {
    /// Replace the light phase's guess with the heavy phase's answer — in
    /// both directions.
    ///
    /// The pre-filter that seeds these signals is an optimistic hint drawn
    /// from the registry's readable-bucket list, and that list only knows
    /// the buckets registered with the stack while `set_remote` accepts any
    /// S3 bucket. Only ever *adding* the mark made such a false positive
    /// permanent for the life of the page — and since the mark suppresses
    /// the Login affordance, it also left a genuinely broken row with no
    /// remedy at all. The refresh is the real call, so it gets the last
    /// word either way.
    fn apply(self, no_access: bool, reason: Option<String>, role_switch_host: Option<String>) {
        self.no_access.set(no_access);
        self.reason.set(reason);
        self.role_switch_host.set(role_switch_host);
    }
}

#[allow(clippy::too_many_arguments)]
#[allow(
    clippy::too_many_lines,
    reason = "declarative Leptos view; length is markup, not logic complexity"
)]
fn build_package_menu(
    data: &PackageItemData,
    status: RwSignal<String>,
    has_changes: RwSignal<bool>,
    access: AccessSignals,
    pull_check: Signal<PullCheck>,
    pull_retry: Trigger,
    refreshing: RwSignal<bool>,
    notification: RwSignal<Option<Notification>>,
    ui_locked: RwSignal<bool>,
    refetch: Trigger,
    show_set_remote_popup: RwSignal<Option<SetRemotePopupData>>,
) -> impl IntoView + use<> {
    let namespace = data.namespace.clone();
    let uri = data.uri.clone();
    let origin_url = data.uri.as_ref().and_then(util::catalog_url);
    let origin_host = data.uri.as_ref().and_then(util::host_str);
    let current_host = origin_host.clone();
    let current_bucket = data.uri.as_ref().and_then(util::bucket_str);
    let has_origin = origin_url.is_some();
    let remote_configured = current_host.is_some() && current_bucket.is_some();

    // ── Open in file browser ──
    let ns_for_open = namespace.clone();
    let uri_for_open = uri.clone();
    let on_open_file_browser = move |_| {
        let ns = ns_for_open.clone();
        let uri = uri_for_open.clone();
        leptos::task::spawn_local(async move {
            match commands::open_in_file_browser(ns, uri).await {
                Ok(msg) => notification.set(Some(Notification::Success(msg))),
                Err(e) => notification.set(Some(Notification::Error(e))),
            }
        });
    };

    // ── Open in catalog ──
    let url_for_catalog = origin_url.clone();
    let on_open_catalog = move |_| {
        if let Some(url) = url_for_catalog.clone() {
            leptos::task::spawn_local(async move {
                let _ = commands::open_in_web_browser(url).await;
            });
        }
    };

    // ── Uninstall ──
    let ns_for_uninstall = namespace.clone();
    let uri_for_uninstall = uri.clone();
    let on_uninstall = move |_| {
        let ns = ns_for_uninstall.clone();
        let uri = uri_for_uninstall.clone();
        ui_locked.set(true);
        leptos::task::spawn_local(async move {
            match commands::package_uninstall(ns, uri).await {
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

    // ── Sync actions (Publish/Pull) ──
    // Stored in StoredValue so they can be used inside Show children (which are Fn).
    let ns_for_publish = namespace.clone();
    let uri_for_publish = uri.clone();
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
    let on_publish = StoredValue::new(on_publish);

    let ns_for_pull = namespace.clone();
    let uri_for_pull = uri.clone();
    let (pull_busy, on_pull) = make_action(
        move || {
            let ns = ns_for_pull.clone();
            let uri = uri_for_pull.clone();
            async move { commands::package_pull(ns, uri).await }
        },
        notification,
        Some(ui_locked),
        move || refetch.notify(),
    );
    let on_pull = StoredValue::new(on_pull);

    let ns_for_merge = namespace.clone();

    // ── Error action (static views, shown/hidden reactively) ──
    let ns_for_set_remote = namespace.clone();
    let current_host_for_popup = current_host.clone();
    let current_bucket_for_popup = current_bucket.clone();
    let has_local_commit_for_popup = data.has_local_commit;
    let login_href = origin_host.as_ref().map(|host| {
        let back_encoded = urlencoding::encode("/installed-packages-list");
        format!("/login?host={host}&back={back_encoded}")
    });

    view! {
        // Open local
        <li class="menu-item">
            <buttons::OpenInFileBrowser on_click=on_open_file_browser small=true />
        </li>
        // Open remote
        {has_origin.then(|| view! {
            <li class="menu-item">
                <buttons::OpenInCatalog
                    on_click=on_open_catalog
                    small=true
                    disabled=Signal::derive(move || status.get() == "local")
                />
            </li>
        })}

        <li class="menu-item menu-divider"></li>

        // Publish: commit (if needed) + push in one click.
        // Gated on having a remote origin, and on there being something to ship
        // (either uncommitted changes or a pending commit).
        <Show when=move || {
            publish_affordance(&status.get(), has_changes.get(), has_origin, access.no_access.get())
        }>
            <li class="menu-item">
                <buttons::Publish
                    on_click=move |ev| on_publish.with_value(|f| f(ev))
                    small=true
                    busy=publish_busy
                    disabled=refreshing
                />
            </li>
        </Show>

        // Pull (behind). Gated on access too: a denied role can never fetch
        // the objects a pull would write, so the row falls back to the
        // Switch-role remedy instead of an action that cannot succeed.
        <Show when=move || status.get() == "behind" && !access.no_access.get()>
            <li class="menu-item menu-divider"></li>
            <li class="menu-item">
                <div class="qui-popover">
                    <buttons::Pull
                        on_click=move |ev| on_pull.with_value(|f| f(ev))
                        small=true
                        busy=pull_busy
                        disabled=Signal::derive(move || !pull_check.get().pull_enabled())
                    />
                    <Show when=move || pull_check.get().is_failed()>
                        <buttons::Refresh on_click=move |_| pull_retry.notify() />
                    </Show>
                    <Show when=move || pull_popover(&pull_check.get()).is_some()>
                        <div class="popover-wrapper">
                            <div class="popover">
                                {move || pull_popover(&pull_check.get()).unwrap_or_default()}
                            </div>
                        </div>
                    </Show>
                </div>
            </li>
        </Show>

        // Merge (diverged). Same access gate: the merge page's own actions
        // (certify, reset-to-remote) both start with an S3 read, so sending
        // a denied role there just relocates the failure.
        <Show when=move || status.get() == "diverged" && !access.no_access.get()>
            <li class="menu-item menu-divider"></li>
            <li class="menu-item">
                <buttons::Merge namespace=ns_for_merge.clone() small=true />
            </li>
        </Show>

        // Error actions
        // Host or bucket missing → Set Remote (warning palette, always visible)
        {(!remote_configured).then(|| view! {
            <li class="menu-item menu-divider"></li>
            <li class="menu-item">
                <buttons::SetRemote
                    on_click=move |_| show_set_remote_popup.set(Some(SetRemotePopupData {
                        namespace: ns_for_set_remote.clone(),
                        current_host: current_host_for_popup.clone(),
                        current_bucket: current_bucket_for_popup.clone(),
                        has_local_commit: has_local_commit_for_popup,
                    }))
                    small=true
                    warning=true
                />
            </li>
        })}
        // Has origin but error → Login (reactive on status).
        // Suppressed for a role denial: the session is healthy, so signing
        // in again re-vends the same denied role — the loop the original
        // bug report was stuck in.
        {login_href.map(|href| view! {
            <Show when=move || status.get() == "error" && !access.no_access.get()>
                <li class="menu-item menu-divider"></li>
                <li class="menu-item">
                    <buttons::Login href=href.clone() small=true />
                </li>
            </Show>
        })}
        // Denied by the active role → point at the role switcher, but only
        // when another role is actually held. A single-role user gets the
        // reason line and no dead-end button.
        <Show when=move || access.no_access.get() && access.role_switch_host.with(Option::is_some)>
            <li class="menu-item menu-divider"></li>
            <li class="menu-item">
                <buttons::SwitchRole small=true />
            </li>
        </Show>

        <li class="menu-item menu-divider"></li>

        // Uninstall
        <li class="menu-item">
            <buttons::Remove on_click=on_uninstall small=true />
        </li>
    }
}

// ── Create Package popup ──

#[component]
fn CreatePackagePopup(
    notification: RwSignal<Option<Notification>>,
    refetch: Trigger,
    on_close: impl Fn() + Clone + 'static,
) -> impl IntoView {
    let namespace = RwSignal::new(String::new());
    let source = RwSignal::new(None::<String>);
    let submitting = RwSignal::new(false);

    let on_close_submit = on_close.clone();
    let on_submit = move || {
        let ns = namespace.get_untracked().trim().to_string();
        if ns.is_empty() || submitting.get_untracked() {
            return;
        }
        submitting.set(true);
        let src = source.get_untracked();
        let on_close = on_close_submit.clone();
        leptos::task::spawn_local(async move {
            match commands::package_create(ns, src, None).await {
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

    let on_browse = move |_| {
        leptos::task::spawn_local(async move {
            if let Ok(path) = commands::open_directory_picker().await {
                source.set(Some(path));
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
                <div class="create-package-form">
                    <label>"Namespace"</label>
                    <input
                        class="create-package-input"
                        type="text"
                        placeholder="owner/package-name"
                        prop:value=move || namespace.get()
                        on:input=move |ev| namespace.set(event_target_value(&ev))
                        on:keydown=on_keydown
                    />

                    <label>"Source directory (optional)"</label>
                    <div class="create-package-source">
                        <span class="source-path">{move || {
                            source.get().unwrap_or_else(|| "No directory selected".to_string())
                        }}</span>
                        <buttons::Browse on_click=on_browse small=true />
                    </div>

                    <div class="create-package-actions">
                        <buttons::FormPrimary on_click=on_submit_click disabled=submitting>
                            "Create"
                        </buttons::FormPrimary>
                        <buttons::FormSecondary on_click=on_cancel />
                    </div>
                </div>
            </div>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use leptos::prelude::*;

    use super::{
        AccessSignals, PAUSED_GUIDANCE, PullCheck, PullOutcome, hint_lines, paused_toast,
        publish_affordance, pull_conflict_hint, pull_popover,
    };

    /// A row seeded by the pre-filter as denied.
    fn pre_marked_row() -> AccessSignals {
        AccessSignals {
            no_access: RwSignal::new(true),
            reason: RwSignal::new(Some(
                "Current role ReadOnly has no access to this bucket".to_string(),
            )),
            role_switch_host: RwSignal::new(Some("acme.quilt.dev".to_string())),
        }
    }

    /// The pre-filter reads the registry's readable-bucket list, which does
    /// not know about buckets outside the stack — but `set_remote` accepts
    /// them. When the real call then succeeds, the mark must go: left in
    /// place it greys a working row for the life of the page, and it takes
    /// the row's Login remedy with it.
    #[test]
    fn a_refresh_that_does_not_deny_clears_the_pre_filter_mark() {
        let access = pre_marked_row();

        access.apply(false, None, None);

        assert!(!access.no_access.get_untracked());
        assert!(access.reason.get_untracked().is_none());
        assert!(
            access.role_switch_host.get_untracked().is_none(),
            "an unmarked row must get its error affordances back"
        );
    }

    /// The other direction still works: the real call is authoritative when
    /// it denies a bucket the pre-filter listed as readable.
    #[test]
    fn a_refresh_that_denies_marks_a_row_the_pre_filter_cleared() {
        let access = AccessSignals {
            no_access: RwSignal::new(false),
            reason: RwSignal::new(None),
            role_switch_host: RwSignal::new(None),
        };

        access.apply(
            true,
            Some("Current role ReadOnly has no access to this bucket".to_string()),
            Some("acme.quilt.dev".to_string()),
        );

        assert!(access.no_access.get_untracked());
        assert_eq!(
            access.reason.get_untracked().as_deref(),
            Some("Current role ReadOnly has no access to this bucket")
        );
    }

    /// A denied row keeps `has_changes` honest but must not offer Commit and
    /// Push: the push gate's first S3 call reads the workflow config, and a
    /// role that cannot read the bucket can never get there. A readable row
    /// with the identical local changes is the contrast that proves the gate
    /// is on access, not on the changes themselves.
    #[test]
    fn denied_row_hides_publish_that_a_readable_row_with_the_same_changes_offers() {
        assert!(
            !publish_affordance("up_to_date", true, true, true),
            "a denied row must not offer Commit and Push"
        );
        assert!(
            publish_affordance("up_to_date", true, true, false),
            "a readable row with the same local changes must still offer it"
        );
    }

    #[test]
    fn blocked_popover_names_conflicts_and_resolution_path() {
        let check = PullCheck::Ready(PullOutcome::Blocked {
            conflicts: vec!["a.txt".to_string(), "b.txt".to_string()],
        });
        assert_eq!(
            pull_popover(&check),
            Some("Resolve conflicts in a.txt, b.txt via commit \u{2192} merge".to_string())
        );
    }

    #[test]
    fn failed_check_popover_reports_honest_error_and_disables_pull() {
        assert_eq!(
            pull_popover(&PullCheck::Failed),
            Some("Couldn't check for updates.".to_string())
        );
        assert!(!PullCheck::Failed.pull_enabled());
        assert!(PullCheck::Failed.is_failed());
    }

    #[test]
    fn non_blocking_checks_have_no_popover() {
        assert_eq!(pull_popover(&PullCheck::Loading), None);
        assert_eq!(
            pull_popover(&PullCheck::Ready(PullOutcome::CleanUpdate)),
            None
        );
        assert_eq!(
            pull_popover(&PullCheck::Ready(PullOutcome::KeepsLocalChanges {
                added: vec!["a.txt".to_string()],
                modified: vec![],
                removed: vec![],
            })),
            None
        );
    }

    #[test]
    fn paused_with_reason_shows_guidance_then_reason() {
        assert_eq!(
            hint_lines(
                "paused",
                true,
                Some("other"),
                Some("workflow rejected metadata"),
                None
            ),
            vec![
                PAUSED_GUIDANCE.to_string(),
                "workflow rejected metadata".to_string(),
            ]
        );
        // A snapshot-seeded pause carries its reason even when the row's
        // own status string was refreshed to something else on mount.
        assert_eq!(
            hint_lines(
                "up_to_date",
                false,
                Some("other"),
                Some("hash mismatch"),
                None
            ),
            vec![PAUSED_GUIDANCE.to_string(), "hash mismatch".to_string()]
        );
    }

    #[test]
    fn pull_conflict_paused_row_points_at_merge_page() {
        // Keyed on the `pullConflict` reason, the row drops the generic
        // "push manually to resume" guidance for the merge-page remediation,
        // naming the conflicting files (which arrive as the pause message).
        assert_eq!(
            hint_lines(
                "paused",
                true,
                Some("pullConflict"),
                Some("a.txt, b.txt"),
                None
            ),
            vec![
                "Conflicts in a.txt, b.txt. Commit your changes to resolve them on the merge page."
                    .to_string()
            ]
        );
    }

    #[test]
    fn pull_conflict_hint_falls_back_without_files() {
        assert_eq!(
            pull_conflict_hint(None),
            "Pull conflict. Commit your changes to resolve it on the merge page."
        );
        assert_eq!(
            pull_conflict_hint(Some("")),
            "Pull conflict. Commit your changes to resolve it on the merge page."
        );
    }

    #[test]
    fn paused_without_reason_shows_guidance_only() {
        assert_eq!(
            hint_lines("paused", true, None, None, None),
            vec![PAUSED_GUIDANCE.to_string()]
        );
    }

    #[test]
    fn paused_takes_precedence_over_error_status() {
        // A row that is both `error` and has a pause reason shows the pause
        // guidance + reason — the more specific, actionable message wins.
        assert_eq!(
            hint_lines(
                "error",
                true,
                Some("other"),
                Some("workflow rejected metadata"),
                None
            ),
            vec![
                PAUSED_GUIDANCE.to_string(),
                "workflow rejected metadata".to_string(),
            ]
        );
    }

    #[test]
    fn error_with_host_prompts_sign_in() {
        assert_eq!(
            hint_lines("error", true, None, None, None),
            vec!["Unable to check remote status — sign in again".to_string()]
        );
    }

    /// The bug this whole change exists to fix: a row the active role
    /// cannot read used to arrive as `error`, which renders "sign in
    /// again" — advice that cannot work, because the re-vend returns the
    /// same denied role. The access reason must replace it.
    #[test]
    fn a_denied_row_never_tells_the_user_to_sign_in_again() {
        let reason = "Current role ReadOnly has no access to this bucket";

        assert_eq!(
            hint_lines("error", true, None, None, Some(reason)),
            vec![reason.to_string()],
            "a denial must not be dressed up as a broken session"
        );
    }

    /// The mark travels with the row whatever the status string says, so a
    /// pre-greyed row explains itself before any per-row call lands.
    #[test]
    fn a_denied_row_explains_itself_whatever_its_status() {
        let reason = "Current role ReadOnly has no access to this bucket";

        for status in ["up_to_date", "behind", "ahead", "error"] {
            assert_eq!(
                hint_lines(status, true, None, None, Some(reason)),
                vec![reason.to_string()],
                "status {status} must not shadow the access reason"
            );
        }
    }

    /// A denied row that is *also* paused shows the denial, including over
    /// a pull conflict's merge-page remediation. Deliberate, and pinned
    /// here: the pause is downstream of the denial — no amount of merging
    /// makes a bucket readable — so leading with the conflict would send
    /// the user to a page that cannot help. With autosync now pausing *for*
    /// a denial too, the two say the same thing, so the precedence only ever
    /// picks between agreeing messages.
    #[test]
    fn a_denial_outranks_a_pause_even_a_pull_conflict() {
        let reason = "Current role ReadOnly has no access to this bucket";

        assert_eq!(
            hint_lines(
                "paused",
                true,
                Some("pullConflict"),
                Some("a.txt, b.txt"),
                Some(reason)
            ),
            vec![reason.to_string()],
        );
        assert_eq!(
            hint_lines(
                "paused",
                true,
                Some("other"),
                Some("workflow rejected metadata"),
                Some(reason)
            ),
            vec![reason.to_string()],
        );
        assert_eq!(
            hint_lines(
                "paused",
                true,
                Some("roleDenied"),
                Some("ReadOnly"),
                Some(reason)
            ),
            vec![reason.to_string()],
        );
    }

    /// A role-denied pause on a row the roster did *not* pre-mark — a
    /// write-only denial, which the read-presence bucket list cannot see.
    /// The generic "push manually to resume" guidance would be a dead end
    /// here, so the row names the role and the switch instead.
    #[test]
    fn role_denied_paused_row_points_at_the_role_switch() {
        assert_eq!(
            hint_lines("paused", true, Some("roleDenied"), Some("ReadOnly"), None),
            vec![
                "Current role ReadOnly has no access to this bucket. \
                 Switch role to resume autosync."
                    .to_string()
            ]
        );
        assert_eq!(
            hint_lines("paused", true, Some("roleDenied"), None, None),
            vec![
                "The active role has no access to this bucket. \
                 Switch role to resume autosync."
                    .to_string()
            ]
        );
    }

    #[test]
    fn error_without_host_reports_no_remote() {
        assert_eq!(
            hint_lines("error", false, None, None, None),
            vec!["No remote configured".to_string()]
        );
    }

    #[test]
    fn healthy_row_has_no_hint() {
        assert!(hint_lines("up_to_date", true, None, None, None).is_empty());
        assert!(hint_lines("ahead", false, None, None, None).is_empty());
    }

    #[test]
    fn paused_toast_carries_the_role_denial() {
        assert_eq!(
            paused_toast("roleDenied", "acme/demo", Some("ReadOnly")),
            Some(
                "Autosync paused acme/demo — Current role ReadOnly has no access to this bucket. \
                 Switch role to resume autosync."
                    .to_string()
            )
        );
    }

    #[test]
    fn paused_toast_fires_for_other_and_pull_conflict_only() {
        assert_eq!(
            paused_toast("other", "acme/demo", Some("workflow rejected metadata")),
            Some("Autosync paused acme/demo — workflow rejected metadata".to_string())
        );
        assert_eq!(
            paused_toast("pullConflict", "acme/demo", Some("a.txt, b.txt")),
            Some(
                "Autosync paused acme/demo — Conflicts in a.txt, b.txt. Commit your changes to resolve them on the merge page."
                    .to_string()
            )
        );
        // Status-legible refusals stay silent.
        assert_eq!(paused_toast("pendingChanges", "acme/demo", None), None);
        assert_eq!(paused_toast("diverged", "acme/demo", None), None);
    }
}
