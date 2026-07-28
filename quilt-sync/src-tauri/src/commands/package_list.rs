//! Installed-packages list (light phase) and per-package status refresh
//! (heavy phase) for the Leptos UI.

use std::collections::HashMap;
use std::collections::HashSet;
use std::time::Duration;

use serde::Serialize;
use tokio::time::timeout;

use quilt_rs::RoleInfo;
use quilt_uri::Host;

use crate::Error;
use crate::autopull::Watcher;
use crate::commands::RoleCache;
use crate::model;
use crate::quilt;

// ── Installed Packages List data for Leptos UI ──

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct InstalledPackagesListData {
    pub packages: Vec<InstalledPackageListItem>,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct InstalledPackageListItem {
    pub namespace: String,
    pub status: String,
    pub has_changes: bool,
    /// True when the package has a local commit. Setting a remote only
    /// re-commits (creating a new revision) when there is a commit to
    /// re-commit, so the UI gates the "creates a new revision" notice on this.
    pub has_local_commit: bool,
    pub uri: Option<quilt_uri::S3PackageUri>,
    /// Raw `lineage.remote_uri` rendering, kept separate from `uri` so
    /// the UI can still surface a misconfigured remote when origin
    /// resolution fails (status: "error" branch).
    pub remote_display: Option<String>,
    /// The autosync watcher's `Other` pause message for this namespace,
    /// if it is currently paused for a reason the status string cannot
    /// carry (workflow refusal, hash mismatch, etc.); `None` otherwise.
    ///
    /// Read straight from the watcher's paused map (the single source of
    /// truth) at fetch time, so the UI derives the red/hint state from
    /// authoritative data instead of a reconciled frontend cache.
    pub paused_reason: Option<String>,
    /// The stable reason discriminant for the pause (`"pullConflict"`,
    /// `"other"`, …), paired with `paused_reason`. `Some` exactly when
    /// `paused_reason` is; lets the UI pick conflict- vs. generic guidance
    /// without re-parsing the message.
    pub paused_kind: Option<String>,
    /// True when the active role cannot reach this row's bucket.
    ///
    /// A GUI-only mark, which is why it rides here as its own boolean
    /// rather than as an [`quilt::lineage::UpstreamState`] variant: the
    /// scriptable commands must not grow an access dimension. Set
    /// proactively from the role's readable-bucket list, and reactively
    /// from an `AccessDenied` on the status call. It is *not* an auth
    /// failure — signing in again re-vends the same denied role — so the
    /// row must not offer the Login route.
    pub no_access: bool,
    /// Why the row is marked, naming the active role. `Some` exactly when
    /// `no_access` is.
    pub no_access_reason: Option<String>,
    /// The host whose role selector the row's switch affordance opens.
    /// `Some` only when the user holds more than one role there, so the
    /// affordance is never a dead end: a single-role user gets the reason
    /// and no button.
    pub role_switch_host: Option<String>,
}

/// Whether the active role can reach a row's bucket, and how to say so.
///
/// The default is unmarked: no evidence of denial, so the row renders as
/// it always has. Shared by both detection paths so they word the mark
/// identically.
#[derive(Clone, Debug, Default)]
pub(super) struct AccessMark {
    no_access: bool,
    pub(super) reason: Option<String>,
    role_switch_host: Option<String>,
}

impl AccessMark {
    /// A denial, stated as a neutral fact — it is equally true when the
    /// user deliberately switched to a narrow role, so the wording never
    /// implies the role is wrong.
    ///
    /// `roles` is `None` when the role query itself failed. The mark still
    /// stands (the denial is known either way); it just cannot name the
    /// role, and it offers no switch affordance because we cannot tell
    /// whether another role is held.
    fn denied(host: Option<&Host>, roles: Option<&RoleInfo>) -> Self {
        let reason = roles.map_or_else(
            || "The active role has no access to this bucket".to_string(),
            |roles| {
                format!(
                    "Current role {} has no access to this bucket",
                    roles.current
                )
            },
        );
        let holds_another_role = roles.is_some_and(|roles| roles.available.len() > 1);
        Self {
            no_access: true,
            reason: Some(reason),
            role_switch_host: host
                .filter(|_| holds_another_role)
                .map(std::string::ToString::to_string),
        }
    }
}

/// Ask who the active role on `host` is, so a denial can name it.
///
/// Goes through the [`RoleCache`], so the answer is fetched once per host
/// per roster load no matter how many rows deny — the heavy phase runs one
/// command invocation per row, concurrently, and they all land here.
///
/// This never invalidates, and a single-row refresh outside a full load
/// therefore rides the name the load warmed. That is the intended cadence:
/// the roster load is the refresh point, and invalidating here instead would
/// put a `/me` behind `refresh_lock_for(host)` — the mutex credential vending
/// takes — once for every denied row, which is the pile-up the cache exists
/// to prevent.
///
/// A failed query is not fatal: [`AccessMark::denied`] degrades to the
/// unnamed wording rather than dropping the mark.
pub(super) async fn denied_mark(
    m: &impl model::QuiltModel,
    roles: &RoleCache,
    host: Option<&Host>,
) -> AccessMark {
    let info = match host {
        Some(host) => roles.get(m, host).await.ok(),
        None => None,
    };
    AccessMark::denied(host, info.as_ref())
}

/// How long the roster waits on one host's readable-bucket query before
/// giving up on it.
///
/// The roster is otherwise local data and used to paint without touching the
/// network. The query added two round trips per host — `config.json`, then a
/// GraphQL POST — serialized per host under that host's credential lock and
/// behind a retry middleware with a 10s connect timeout, so an unreachable
/// host (offline, captive portal, DNS blackhole) held the main screen blank
/// for tens of seconds.
///
/// The pre-filter is an optimistic hint, and there is already a correct
/// degrade path for not having it: reactive-only marking, where the
/// authoritative per-row status call marks the denied rows a moment later.
/// So the budget is deliberately short. Undershooting on a slow-but-working
/// link costs only the hint; overshooting costs the first paint.
const BUCKET_LIST_BUDGET: Duration = Duration::from_secs(2);

/// A message-bearing autosync pause for a namespace: the stable reason
/// discriminant plus the human-readable message (raw refusal reason, or the
/// comma-joined conflicting files for a pull conflict). Keyed by namespace in
/// the paused map the list builder consumes.
#[derive(Clone, Debug)]
pub struct PausedRow {
    pub reason: String,
    pub message: String,
}

pub(super) async fn get_installed_packages_list_data_from_model(
    m: &impl model::QuiltModel,
    roles: &RoleCache,
    tracing: &crate::telemetry::Telemetry,
    paused_reasons: &HashMap<String, PausedRow>,
) -> Result<InstalledPackagesListData, Error> {
    // A roster load is the cadence the role refresh is pinned to. A switch is
    // server-side and global, so it can happen in the web catalog with the app
    // none the wiser; held for a whole session, the cached name would make a
    // row say "Current role X has no access" about a role that in fact has it,
    // and — worse — `observe_role` would never re-run, leaving the S3 clients
    // signing as the old role until Settings is opened or the ~1h credential
    // TTL expires. Dropping the names here means the first row that needs one
    // re-fetches through `observe_role`, which finishes the coherence flush,
    // and the cache then serves every remaining row: one `/me` per host per
    // load, not one per row.
    //
    // Every host, not just the roster's: the roster is not known until the
    // list below is fetched, and an entry is only a *name*, so clearing a host
    // no row mentions costs nothing now and at most one round trip whenever
    // something next asks about it.
    roles.invalidate(None).await;

    let list = m.get_installed_packages_list().await?;
    let mut packages = Vec::new();
    for installed_package in list {
        match load_package_item(m, tracing, &installed_package, paused_reasons).await {
            Ok(item) => packages.push(item),
            Err(err) => {
                tracing::warn!(
                    "Failed to load package {}: {err}",
                    installed_package.namespace,
                );
            }
        }
    }
    mark_unreadable_buckets(m, roles, &mut packages).await;
    Ok(InstalledPackagesListData { packages })
}

/// The host a row's remote lives on, if it has one.
fn row_host(item: &InstalledPackageListItem) -> Option<&Host> {
    item.uri.as_ref().and_then(|uri| uri.catalog.as_ref())
}

/// Pre-mark the rows whose bucket is outside what the active role can read.
///
/// One query per host, not per package: the roster asks each host once and
/// intersects the answer with the buckets its rows live in. The answer is
/// an optimistic hint — it over-reports for unmanaged roles and
/// anonymous-access stacks, and says nothing about writes — so a miss only
/// greys the row; the authoritative answer still comes from the per-row
/// status call.
///
/// A failed query degrades to reactive-only marking. It must never be read
/// as "nothing is readable": an empty set would grey the entire roster.
/// A query that does not answer inside [`BUCKET_LIST_BUDGET`] counts as
/// failed, which is what keeps the roster local-only in effect.
async fn mark_unreadable_buckets(
    m: &impl model::QuiltModel,
    roles: &RoleCache,
    packages: &mut [InstalledPackageListItem],
) {
    let mut hosts: Vec<Host> = Vec::new();
    for host in packages.iter().filter_map(row_host) {
        if !hosts.contains(host) {
            hosts.push(host.clone());
        }
    }

    for host in hosts {
        let query = timeout(BUCKET_LIST_BUDGET, m.readable_buckets(&host));
        let readable: HashSet<String> = match query.await {
            Ok(Ok(buckets)) => buckets.into_iter().collect(),
            Ok(Err(err)) => {
                tracing::debug!("No readable-bucket list for {host}: {err}");
                continue;
            }
            Err(_) => {
                tracing::debug!("Readable-bucket list for {host} timed out");
                continue;
            }
        };

        let unreadable: Vec<usize> = packages
            .iter()
            .enumerate()
            .filter(|(_, item)| row_host(item) == Some(&host))
            .filter(|(_, item)| {
                item.uri
                    .as_ref()
                    .is_some_and(|uri| !readable.contains(&uri.bucket))
            })
            .map(|(index, _)| index)
            .collect();
        if unreadable.is_empty() {
            // A host the role can fully read costs one query, not two.
            continue;
        }

        let mark = denied_mark(m, roles, Some(&host)).await;
        for index in unreadable {
            let item = &mut packages[index];
            item.no_access = mark.no_access;
            item.no_access_reason.clone_from(&mark.reason);
            item.role_switch_host.clone_from(&mark.role_switch_host);
        }
    }
}

async fn load_package_item(
    m: &impl model::QuiltModel,
    tracing: &crate::telemetry::Telemetry,
    installed_package: &quilt::InstalledPackage,
    paused_reasons: &HashMap<String, PausedRow>,
) -> Result<InstalledPackageListItem, Error> {
    let namespace = installed_package.namespace.to_string();
    let paused = paused_reasons.get(&namespace);
    let paused_reason = paused.map(|p| p.message.clone());
    let paused_kind = paused.map(|p| p.reason.clone());
    let lineage = m.get_installed_package_lineage(installed_package).await?;
    // Computed before `lineage` is moved by the `into()` below.
    let has_local_commit = lineage.commit.is_some();

    let Some(remote_uri) = lineage.remote_uri.as_ref() else {
        return Ok(InstalledPackageListItem {
            namespace,
            status: "local".to_string(),
            has_changes: false,
            has_local_commit,
            uri: None,
            remote_display: None,
            paused_reason,
            paused_kind,
            no_access: false,
            no_access_reason: None,
            role_switch_host: None,
        });
    };

    let typed_uri = quilt_uri::S3PackageUri::from(remote_uri);

    if remote_uri.origin.is_none() {
        return Ok(InstalledPackageListItem {
            namespace,
            status: "error".to_string(),
            has_changes: false,
            has_local_commit,
            uri: Some(typed_uri),
            remote_display: Some(remote_uri.to_string()),
            paused_reason,
            paused_kind,
            no_access: false,
            no_access_reason: None,
            role_switch_host: None,
        });
    }

    if let Some(host) = typed_uri.catalog.as_ref() {
        tracing.add_host(host);
    }
    let remote_display = remote_uri.to_string();
    let upstream_state: quilt::lineage::UpstreamState = lineage.into();
    let has_changes = false; // Refined by refresh_package_status

    Ok(InstalledPackageListItem {
        namespace,
        status: upstream_state.to_string(),
        has_changes,
        has_local_commit,
        uri: Some(typed_uri),
        remote_display: Some(remote_display),
        paused_reason,
        paused_kind,
        // Filled in by the roster-wide access pass, which needs the whole
        // list to query each host once instead of once per row.
        no_access: false,
        no_access_reason: None,
        role_switch_host: None,
    })
}

#[tauri::command]
pub async fn get_installed_packages_list_data(
    m: tauri::State<'_, model::Model>,
    roles: tauri::State<'_, RoleCache>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
    watcher: tauri::State<'_, Watcher>,
) -> Result<InstalledPackagesListData, String> {
    // Read the watcher's paused map — the single source of truth — the
    // same way `get_autosync_snapshot` does. Only message-bearing pauses
    // (`other`, `pullConflict`, and `roleDenied` when the role could be
    // named) surface on a row; the status string alone carries the rest. The
    // reason discriminant rides along so the UI can pick conflict-,
    // role- or generic guidance.
    let paused_reasons: HashMap<String, PausedRow> = watcher
        .snapshot()
        .await
        .paused
        .into_iter()
        .filter_map(|entry| {
            entry.message.map(|message| {
                (
                    entry.namespace,
                    PausedRow {
                        reason: entry.reason,
                        message,
                    },
                )
            })
        })
        .collect();

    get_installed_packages_list_data_from_model(&*m, &roles, &tracing, &paused_reasons)
        .await
        .map_err(|e| e.to_frontend_string())
}

// ── Refresh package status (heavy phase) ──

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RefreshedPackageStatus {
    pub status: String,
    pub has_changes: bool,
    /// See [`InstalledPackageListItem::no_access`]. The heavy phase carries
    /// it too, so a denial discovered by the real call reaches a row the
    /// light phase's bucket list cleared.
    pub no_access: bool,
    pub no_access_reason: Option<String>,
    pub role_switch_host: Option<String>,
}

impl RefreshedPackageStatus {
    /// A refreshed row with no access question attached.
    fn new(status: String, has_changes: bool) -> Self {
        Self {
            status,
            has_changes,
            no_access: false,
            no_access_reason: None,
            role_switch_host: None,
        }
    }

    /// A refreshed row the active role may not reach.
    fn marked(status: String, has_changes: bool, mark: AccessMark) -> Self {
        Self {
            status,
            has_changes,
            no_access: mark.no_access,
            no_access_reason: mark.reason,
            role_switch_host: mark.role_switch_host,
        }
    }
}

/// Whether the working tree differs from the installed manifest, answered
/// without touching the remote.
///
/// The fallback for a row whose remote refresh was refused. Local changes are
/// computed locally at both ends, so a denial says nothing about them, and the
/// UI gates Publish on the answer.
async fn local_changes(m: &impl model::QuiltModel, package: &quilt::InstalledPackage) -> bool {
    match m.recompute_local_status(package, None).await {
        Ok(status) => !status.changes.is_empty(),
        Err(err) => {
            tracing::warn!(
                "Failed to recompute local status for {}: {err}",
                package.namespace,
            );
            false
        }
    }
}

pub(super) async fn refresh_package_status_from_model(
    m: &impl model::QuiltModel,
    roles: &RoleCache,
    tracing: &crate::telemetry::Telemetry,
    namespace: &quilt_uri::Namespace,
) -> Result<RefreshedPackageStatus, Error> {
    let installed_package = m.get_installed_package(namespace).await?.ok_or_else(|| {
        Error::from(quilt::InstallPackageError::NotInstalled(
            namespace.to_owned(),
        ))
    })?;

    let lineage = m.get_installed_package_lineage(&installed_package).await?;

    let Some(remote_uri) = lineage.remote_uri.as_ref() else {
        let has_changes = match m
            .get_installed_package_status(&installed_package, None)
            .await
        {
            Ok(s) => !s.changes.is_empty(),
            Err(err) => {
                tracing::warn!(
                    "Failed to get status for {}: {err}",
                    installed_package.namespace,
                );
                false
            }
        };
        return Ok(RefreshedPackageStatus::new(
            "local".to_string(),
            has_changes,
        ));
    };
    if remote_uri.origin.is_none() {
        return Ok(RefreshedPackageStatus::new("error".to_string(), false));
    }

    let host = remote_uri.origin.clone();
    if let Some(host) = host.as_ref() {
        tracing.add_host(host);
    }
    // What lineage alone already knows. A row we are not allowed to
    // re-check keeps this instead of being downgraded to `Error`, which is
    // the state the UI renders as "sign in again".
    let cached_state: quilt::lineage::UpstreamState = lineage.into();

    let (upstream_state, has_changes, denied) = match m
        .get_installed_package_status(&installed_package, None)
        .await
    {
        Ok(s) => (s.upstream_state, !s.changes.is_empty(), false),
        Err(err) if err.is_access_denied() => {
            // Not an auth failure: credential vending succeeded, so the
            // session is healthy and the active role simply cannot reach
            // this bucket. Rendering "sign in again" here is what sent the
            // original bug reporter into an unrecoverable re-login loop —
            // the re-vend hands back the same denied role.
            //
            // The changes, though, are the working tree against the cached
            // manifest — local both ends, and none of the remote's business.
            // Reporting `false` here would take the row's Publish button
            // away while real uncommitted work sits underneath it.
            (
                cached_state,
                local_changes(m, &installed_package).await,
                true,
            )
        }
        Err(err) => {
            tracing::warn!(
                "Failed to get status for {}: {err}",
                installed_package.namespace,
            );
            (quilt::lineage::UpstreamState::Error, false, false)
        }
    };

    if denied {
        let mark = denied_mark(m, roles, host.as_ref()).await;
        return Ok(RefreshedPackageStatus::marked(
            upstream_state.to_string(),
            has_changes,
            mark,
        ));
    }

    Ok(RefreshedPackageStatus::new(
        upstream_state.to_string(),
        has_changes,
    ))
}

#[tauri::command]
pub async fn refresh_package_status(
    m: tauri::State<'_, model::Model>,
    roles: tauri::State<'_, RoleCache>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
    namespace: String,
) -> Result<RefreshedPackageStatus, String> {
    let namespace: quilt_uri::Namespace = namespace
        .try_into()
        .map_err(|e: quilt_uri::UriError| e.to_string())?;

    refresh_package_status_from_model(&*m, &roles, &tracing, &namespace)
        .await
        .map_err(|e| e.to_frontend_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::{Arc, Mutex};

    use quilt_rs::RoleInfo;

    use crate::commands::test_support::*;
    use crate::model::MockQuiltModel;
    use crate::model::mocks;

    // ── Access-marking fixtures ──

    /// A `ManifestUri` in an explicit bucket, so a test can put two rows on
    /// the same host in different buckets — the shape the readable-bucket
    /// intersection is about.
    fn make_manifest_uri_in_bucket(bucket: &str, namespace: &str) -> quilt_uri::ManifestUri {
        quilt_uri::ManifestUri {
            origin: Some("test.quilt.dev".parse().unwrap()),
            bucket: bucket.to_string(),
            namespace: namespace.try_into().unwrap(),
            hash: "abcdef".to_string(),
        }
    }

    /// Every roster fetch now asks each host what the active role can read.
    /// Tests that are not about access declare the query unavailable, which
    /// is the degrade-to-reactive path: no row is marked either way.
    fn without_bucket_list(model: &mut MockQuiltModel) {
        model
            .expect_readable_buckets()
            .returning(|_| Err(Error::General("bucket list unavailable".to_string())));
    }

    /// A user holding two roles, the second of which is active.
    fn two_roles() -> RoleInfo {
        RoleInfo {
            current: "ReadOnly".to_string(),
            available: vec!["ReadWrite".to_string(), "ReadOnly".to_string()],
        }
    }

    /// A roster of two packages on one host — `team/open` in `reachable`,
    /// `team/locked` in `locked` — with the role's readable set and role
    /// list under the test's control.
    fn mock_two_bucket_roster(
        readable: Result<Vec<String>, ()>,
        roles: Option<RoleInfo>,
    ) -> MockQuiltModel {
        let mut model = mocks::create();

        let pkgs = vec![
            make_installed_package(("team", "open")),
            make_installed_package(("team", "locked")),
        ];
        model
            .expect_get_installed_packages_list()
            .return_once(move || Ok(pkgs));
        model
            .expect_get_installed_package_lineage()
            .returning(|pkg| {
                let ns = pkg.namespace.to_string();
                let bucket = if ns == "team/locked" {
                    "locked"
                } else {
                    "reachable"
                };
                Ok(quilt::lineage::PackageLineage::from_remote(
                    make_manifest_uri_in_bucket(bucket, &ns),
                    "abcdef".to_string(),
                ))
            });
        model.expect_readable_buckets().returning(move |_| {
            readable
                .clone()
                .map_err(|()| Error::General("bucket list unavailable".to_string()))
        });
        if let Some(roles) = roles {
            model
                .expect_refresh_roles()
                .returning(move |_| Ok(roles.clone()));
        } else {
            model
                .expect_refresh_roles()
                .returning(|_| Err(Error::General("role query unavailable".to_string())));
        }
        // Reading the role can expire the stored credentials, so the cache
        // finishes the flush by dropping the clients. See `RoleCache::get`.
        model.expect_clear_remote_client_cache().returning(|_| ());
        model
    }

    /// A single package on `test.quilt.dev` whose status call fails with
    /// `err`, with the host's roles under the test's control and a clean
    /// working tree.
    fn mock_model_with_status_error(err: Error, roles: Option<RoleInfo>) -> MockQuiltModel {
        mock_model_with_status_error_and_local_changes(err, roles, false)
    }

    /// As above, but with the local working tree under the test's control
    /// too: the denial path answers `has_changes` from a local recompute,
    /// since the remote has no say in what the working tree contains.
    fn mock_model_with_status_error_and_local_changes(
        err: Error,
        roles: Option<RoleInfo>,
        dirty: bool,
    ) -> MockQuiltModel {
        let mut model = mocks::create();
        model
            .expect_recompute_local_status()
            .returning(move |_, _| {
                let changes = if dirty {
                    one_local_change()
                } else {
                    quilt::lineage::ChangeSet::new()
                };
                Ok(quilt::lineage::InstalledPackageStatus::new(
                    quilt::lineage::UpstreamState::UpToDate,
                    changes,
                ))
            });
        let pkg = make_installed_package(("test", "denied"));
        model
            .expect_get_installed_package()
            .return_once(move |_| Ok(Some(pkg)));
        model
            .expect_get_installed_package_lineage()
            .returning(|pkg| {
                Ok(quilt::lineage::PackageLineage::from_remote(
                    make_manifest_uri(&pkg.namespace.to_string()),
                    "abcdef".to_string(),
                ))
            });
        model
            .expect_get_installed_package_status()
            .return_once(move |_, _| Err(err));
        if let Some(roles) = roles {
            model
                .expect_refresh_roles()
                .returning(move |_| Ok(roles.clone()));
            model.expect_clear_remote_client_cache().returning(|_| ());
        } else {
            model.expect_refresh_roles().never();
            model.expect_clear_remote_client_cache().never();
        }
        model
    }

    /// An access denial is NOT an auth failure. Rendering it as one sends
    /// the user into a re-login loop that cannot succeed, because the
    /// re-vend returns the same denied role.
    #[tokio::test]
    async fn access_denied_status_is_not_reported_as_a_login_error() {
        let m = mock_model_with_status_error(access_denied_error(), Some(two_roles()));

        let item = refresh_package_status_from_model(
            &m,
            &RoleCache::default(),
            &crate::telemetry::Telemetry::default(),
            &("test", "denied").into(),
        )
        .await
        .expect("refresh");

        assert!(
            item.no_access,
            "a denial must set the roster's no-access mark"
        );
        assert_ne!(
            item.status, "error",
            "must not collapse into the sign-in-again state"
        );
    }

    /// A denied row keeps reporting its local changes. They are computed
    /// against the cached manifest — local at both ends — and the UI gates
    /// Publish on them, so dropping them hides the button on a row with real
    /// uncommitted work.
    #[tokio::test]
    async fn a_denied_row_still_reports_its_local_changes() {
        let m = mock_model_with_status_error_and_local_changes(
            access_denied_error(),
            Some(two_roles()),
            true,
        );

        let item = refresh_package_status_from_model(
            &m,
            &RoleCache::default(),
            &crate::telemetry::Telemetry::default(),
            &("test", "denied").into(),
        )
        .await
        .expect("refresh");

        assert!(item.no_access, "the row is still marked");
        assert!(
            item.has_changes,
            "a denial says nothing about the working tree"
        );
    }

    /// A genuine failure keeps its `error` status and its Login route —
    /// only a denial is rerouted.
    #[tokio::test]
    async fn a_generic_status_failure_is_still_reported_as_an_error() {
        let m = mock_model_with_status_error(Error::General("network error".to_string()), None);

        let item = refresh_package_status_from_model(
            &m,
            &RoleCache::default(),
            &crate::telemetry::Telemetry::default(),
            &("test", "denied").into(),
        )
        .await
        .expect("refresh");

        assert_eq!(item.status, "error");
        assert!(!item.no_access, "a network blip is not a denial");
    }

    /// The mark states a fact and names the role, so the user can tell
    /// whether the narrow role is the one they chose. Not a judgement: it
    /// reads the same whether the switch was deliberate or not.
    #[tokio::test]
    async fn the_mark_names_the_active_role() {
        let m = mock_model_with_status_error(access_denied_error(), Some(two_roles()));

        let item = refresh_package_status_from_model(
            &m,
            &RoleCache::default(),
            &crate::telemetry::Telemetry::default(),
            &("test", "denied").into(),
        )
        .await
        .expect("refresh");

        assert_eq!(
            item.no_access_reason.as_deref(),
            Some("Current role ReadOnly has no access to this bucket")
        );
        assert_eq!(
            item.role_switch_host.as_deref(),
            Some("test.quilt.dev"),
            "a user holding two roles gets the switch affordance, pointed at the row's host"
        );
    }

    /// A single-role user has nowhere to switch to, so the affordance would
    /// be a dead end. They still get the reason.
    #[tokio::test]
    async fn a_single_role_user_gets_the_reason_without_a_switch_affordance() {
        let only_role = RoleInfo {
            current: "ReadOnly".to_string(),
            available: vec!["ReadOnly".to_string()],
        };
        let m = mock_model_with_status_error(access_denied_error(), Some(only_role));

        let item = refresh_package_status_from_model(
            &m,
            &RoleCache::default(),
            &crate::telemetry::Telemetry::default(),
            &("test", "denied").into(),
        )
        .await
        .expect("refresh");

        assert!(item.no_access);
        assert!(
            item.role_switch_host.is_none(),
            "one role means the switch button leads nowhere"
        );
    }

    /// A package whose bucket is outside the role's readable set is greyed
    /// before any per-row network call.
    #[tokio::test]
    async fn packages_outside_the_readable_bucket_set_are_pre_greyed() {
        let m = mock_two_bucket_roster(Ok(vec!["reachable".to_string()]), Some(two_roles()));

        let data = get_installed_packages_list_data_from_model(
            &m,
            &RoleCache::default(),
            &crate::telemetry::Telemetry::default(),
            &HashMap::new(),
        )
        .await
        .expect("list");

        let find = |ns: &str| {
            data.packages
                .iter()
                .find(|p| p.namespace == ns)
                .unwrap_or_else(|| panic!("{ns} in roster"))
        };
        let unreachable = find("team/locked");
        assert!(
            unreachable.no_access,
            "a bucket outside the readable set must be marked"
        );
        assert_eq!(
            unreachable.no_access_reason.as_deref(),
            Some("Current role ReadOnly has no access to this bucket")
        );
        assert!(
            !find("team/open").no_access,
            "a readable bucket must not be marked"
        );
    }

    /// The bucket list over-reports for unmanaged roles and anonymous-access
    /// stacks, so a listed bucket can still deny. The reactive path is the
    /// authority: the row is marked even though the pre-filter cleared it.
    #[tokio::test]
    async fn a_listed_bucket_that_denies_is_still_greyed() {
        // The pre-filter sees the bucket as readable...
        let m = mock_two_bucket_roster(Ok(vec!["reachable".to_string()]), Some(two_roles()));
        let data = get_installed_packages_list_data_from_model(
            &m,
            &RoleCache::default(),
            &crate::telemetry::Telemetry::default(),
            &HashMap::new(),
        )
        .await
        .expect("list");
        let listed = data
            .packages
            .iter()
            .find(|p| p.namespace == "team/open")
            .expect("team/open in roster");
        assert!(!listed.no_access, "the pre-filter clears a listed bucket");

        // ...and the real call denies anyway.
        let m = mock_model_with_status_error(access_denied_error(), Some(two_roles()));
        let refreshed = refresh_package_status_from_model(
            &m,
            &RoleCache::default(),
            &crate::telemetry::Telemetry::default(),
            &("test", "denied").into(),
        )
        .await
        .expect("refresh");

        assert!(
            refreshed.no_access,
            "the reactive path is authoritative over the list"
        );
    }

    /// A failed bucket query is not evidence that nothing is readable.
    /// Treating it as an empty set would grey every row in the roster; the
    /// only safe degrade is to reactive-only marking.
    #[tokio::test]
    async fn a_failed_bucket_query_leaves_every_row_unmarked() {
        let m = mock_two_bucket_roster(Err(()), Some(two_roles()));

        let data = get_installed_packages_list_data_from_model(
            &m,
            &RoleCache::default(),
            &crate::telemetry::Telemetry::default(),
            &HashMap::new(),
        )
        .await
        .expect("list");

        assert_eq!(data.packages.len(), 2);
        assert!(
            data.packages.iter().all(|p| !p.no_access),
            "an unanswerable bucket query must not grey the whole roster"
        );
    }

    /// A host that accepts the connection and then says nothing — the
    /// captive-portal shape, which a connect timeout does not cover.
    ///
    /// Hand-written rather than mocked: a mockall expectation resolves
    /// synchronously, so it cannot model a call that hangs.
    struct SilentHost {
        domain: tokio::sync::Mutex<quilt::LocalDomain>,
    }

    impl Default for SilentHost {
        fn default() -> Self {
            Self {
                domain: tokio::sync::Mutex::new(quilt::LocalDomain::new(std::path::PathBuf::new())),
            }
        }
    }

    impl model::QuiltModel for SilentHost {
        fn get_quilt(&self) -> &tokio::sync::Mutex<quilt::LocalDomain> {
            &self.domain
        }

        async fn get_installed_packages_list(&self) -> Result<Vec<quilt::InstalledPackage>, Error> {
            Ok(vec![
                make_installed_package(("team", "open")),
                make_installed_package(("team", "locked")),
            ])
        }

        async fn get_installed_package_lineage(
            &self,
            package: &quilt::InstalledPackage,
        ) -> Result<quilt::lineage::PackageLineage, Error> {
            let ns = package.namespace.to_string();
            Ok(quilt::lineage::PackageLineage::from_remote(
                make_manifest_uri_in_bucket("locked", &ns),
                "abcdef".to_string(),
            ))
        }

        async fn readable_buckets(&self, _host: &Host) -> Result<Vec<String>, Error> {
            // Far beyond any budget the roster could reasonably wait.
            tokio::time::sleep(BUCKET_LIST_BUDGET * 100).await;
            Ok(Vec::new())
        }
    }

    /// The roster is local data; the bucket query is an optimistic hint on
    /// top of it. A host that never answers — offline, captive portal — must
    /// not hold the main screen: the hint is dropped, the roster paints, and
    /// the per-row status call marks whatever is really denied.
    #[tokio::test(start_paused = true)]
    async fn an_unanswerable_host_does_not_hold_the_roster() {
        let m = SilentHost::default();

        let started = tokio::time::Instant::now();
        let data = get_installed_packages_list_data_from_model(
            &m,
            &RoleCache::default(),
            &crate::telemetry::Telemetry::default(),
            &HashMap::new(),
        )
        .await
        .expect("list");

        assert_eq!(data.packages.len(), 2, "the roster paints from local data");
        assert!(
            data.packages.iter().all(|p| !p.no_access),
            "a query that never answered is not evidence of denial"
        );
        assert!(
            started.elapsed() <= BUCKET_LIST_BUDGET,
            "the roster waited {:?} on a host that never answers",
            started.elapsed(),
        );
    }

    /// A denial is known even when the role query behind the wording fails,
    /// so the row is still marked — just unnamed, and with no switch
    /// affordance, since we cannot tell whether another role is held.
    #[tokio::test]
    async fn a_denial_is_marked_even_when_the_role_cannot_be_named() {
        let m = mock_two_bucket_roster(Ok(vec!["reachable".to_string()]), None);

        let data = get_installed_packages_list_data_from_model(
            &m,
            &RoleCache::default(),
            &crate::telemetry::Telemetry::default(),
            &HashMap::new(),
        )
        .await
        .expect("list");

        let locked = data
            .packages
            .iter()
            .find(|p| p.namespace == "team/locked")
            .expect("team/locked in roster");
        assert!(locked.no_access);
        assert!(locked.no_access_reason.is_some(), "still says why");
        assert!(locked.role_switch_host.is_none());
    }

    /// The heavy phase runs once per row, concurrently. Asking the host who
    /// the active role is on every denied row would queue one `/me` round
    /// trip per row behind `refresh_lock_for(host)` — the same mutex
    /// credential vending takes — so a roster of 50 denied rows would make
    /// 50 calls to re-answer one question. `times(1)` across two refreshes
    /// is what pins that; mockall fails the test on the second call.
    #[tokio::test]
    async fn the_role_behind_a_denial_is_fetched_once_per_host() {
        let mut m = mocks::create();
        m.expect_get_installed_package()
            .returning(|_| Ok(Some(make_installed_package(("test", "denied")))));
        m.expect_get_installed_package_lineage().returning(|pkg| {
            Ok(quilt::lineage::PackageLineage::from_remote(
                make_manifest_uri(&pkg.namespace.to_string()),
                "abcdef".to_string(),
            ))
        });
        m.expect_get_installed_package_status()
            .returning(|_, _| Err(access_denied_error()));
        m.expect_recompute_local_status()
            .returning(|_, _| Ok(quilt::lineage::InstalledPackageStatus::default()));
        m.expect_refresh_roles()
            .times(1)
            .returning(|_| Ok(two_roles()));
        // Once per fetch, not once per row: the flush rides with the fetch.
        m.expect_clear_remote_client_cache()
            .times(1)
            .returning(|_| ());

        // Two rows on the same host, each its own command invocation,
        // sharing the app-lifetime cache.
        let roles = RoleCache::default();
        for _ in 0..2 {
            let item = refresh_package_status_from_model(
                &m,
                &roles,
                &crate::telemetry::Telemetry::default(),
                &("test", "denied").into(),
            )
            .await
            .expect("refresh");

            assert_eq!(
                item.no_access_reason.as_deref(),
                Some("Current role ReadOnly has no access to this bucket"),
                "a cached role must name the row exactly as a fresh fetch would"
            );
        }
    }

    /// A roster whose `team/locked` row is always denied, and whose host
    /// answers `/me` with a different role each time it is asked — the shape
    /// of a switch made in the web catalog between two roster loads.
    ///
    /// Returns the model alongside the log of hosts whose S3 clients were
    /// dropped, so a test can assert on the flush and not only on the wording.
    fn mock_roster_switching_role_between_loads(
        first: RoleInfo,
        second: RoleInfo,
    ) -> (MockQuiltModel, Arc<Mutex<Vec<String>>>) {
        let mut model = mocks::create();

        model.expect_get_installed_packages_list().returning(|| {
            Ok(vec![
                make_installed_package(("team", "open")),
                make_installed_package(("team", "locked")),
            ])
        });
        model
            .expect_get_installed_package_lineage()
            .returning(|pkg| {
                let ns = pkg.namespace.to_string();
                let bucket = if ns == "team/locked" {
                    "locked"
                } else {
                    "reachable"
                };
                Ok(quilt::lineage::PackageLineage::from_remote(
                    make_manifest_uri_in_bucket(bucket, &ns),
                    "abcdef".to_string(),
                ))
            });
        // Live over GraphQL, so the pre-filter already reflects the switch;
        // only the *name* comes from the cache.
        model
            .expect_readable_buckets()
            .returning(|_| Ok(vec!["reachable".to_string()]));

        let answers = Mutex::new(vec![second, first]);
        model.expect_refresh_roles().times(2).returning(move |_| {
            Ok(answers
                .lock()
                .expect("role answers")
                .pop()
                .expect("one role answer per load"))
        });

        let cache_clears = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&cache_clears);
        model
            .expect_clear_remote_client_cache()
            .returning(move |host: Option<Host>| {
                log.lock()
                    .expect("cache clear log")
                    .push(host.map(|h| h.to_string()).unwrap_or_default());
            });

        (model, cache_clears)
    }

    /// Run one roster load and return the denied row's reason line.
    async fn locked_row_reason(m: &impl model::QuiltModel, roles: &RoleCache) -> String {
        let data = get_installed_packages_list_data_from_model(
            m,
            roles,
            &crate::telemetry::Telemetry::default(),
            &HashMap::new(),
        )
        .await
        .expect("list");
        data.packages
            .into_iter()
            .find(|p| p.namespace == "team/locked")
            .expect("team/locked in roster")
            .no_access_reason
            .expect("a denied row says why")
    }

    /// A role switched outside the app — in the web catalog, where a switch
    /// is server-side and global — must be picked up on the **next roster
    /// load**, both halves of it.
    ///
    /// The visible half is the name: the mark itself is live (the pre-filter
    /// queries the registry), but the wording quotes the cache, so a role
    /// cached for the whole session makes a row read "Current role
    /// `ReadWrite` has no access" while naming the role that granted it.
    ///
    /// The half that actually costs the user is the coherence flush. Only
    /// `refresh_roles` can notice the new role and expire the stored
    /// credentials, and only dropping the S3 clients releases the copy the
    /// SDK's identity cache is still signing with. Cached per session, that
    /// pair never re-ran on the roster path, so every read kept using the old
    /// role until Settings was opened or the ~1h TTL ran out.
    #[tokio::test]
    async fn an_out_of_band_switch_is_picked_up_on_the_next_list_load() {
        let read_write = RoleInfo {
            current: "ReadWrite".to_string(),
            available: vec!["ReadWrite".to_string(), "ReadOnly".to_string()],
        };
        let (m, cache_clears) = mock_roster_switching_role_between_loads(read_write, two_roles());
        let roles = RoleCache::default();

        assert_eq!(
            locked_row_reason(&m, &roles).await,
            "Current role ReadWrite has no access to this bucket"
        );
        // The user switches to ReadOnly in the web catalog here.
        assert_eq!(
            locked_row_reason(&m, &roles).await,
            "Current role ReadOnly has no access to this bucket",
            "the next roster load must re-ask, not quote the role the session opened with"
        );

        assert_eq!(
            *cache_clears.lock().expect("cache clear log"),
            vec!["test.quilt.dev".to_string(), "test.quilt.dev".to_string()],
            "each load's fetch must finish the flush, or the S3 clients keep \
             signing as the role the user left"
        );
    }

    /// A switch makes the cached role name wrong. Quoting the previous role
    /// back at the user is worse than not caching at all.
    #[tokio::test]
    async fn a_switch_drops_the_cached_role() {
        let roles = RoleCache::default();
        let host: quilt_uri::Host = "test.quilt.dev".parse().expect("host");

        let mut first = MockQuiltModel::new();
        first
            .expect_refresh_roles()
            .times(1)
            .returning(|_| Ok(two_roles()));
        first.expect_clear_remote_client_cache().returning(|_| ());
        assert_eq!(
            roles.get(&first, &host).await.expect("roles").current,
            "ReadOnly"
        );

        roles.invalidate(Some(&host)).await;

        let mut after = MockQuiltModel::new();
        after.expect_clear_remote_client_cache().returning(|_| ());
        after.expect_refresh_roles().times(1).returning(|_| {
            Ok(RoleInfo {
                current: "ReadWrite".to_string(),
                available: vec!["ReadWrite".to_string(), "ReadOnly".to_string()],
            })
        });
        assert_eq!(
            roles.get(&after, &host).await.expect("roles").current,
            "ReadWrite",
            "an invalidated host must be re-fetched, not served the old role"
        );
    }

    #[tokio::test]
    async fn test_get_installed_packages_list_data_empty() -> Result<(), String> {
        let mut model = mocks::create();
        mocks::mock_installed_packages_list(&mut model);
        let tracing = crate::telemetry::Telemetry::default();

        let data = get_installed_packages_list_data_from_model(
            &model,
            &RoleCache::default(),
            &tracing,
            &HashMap::new(),
        )
        .await
        .map_err(|e| e.to_string())?;

        assert!(data.packages.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn test_installed_packages_list_data_statuses() -> Result<(), String> {
        let mut model = mocks::create();

        let pkgs = vec![
            make_installed_package(("test", "ahead")),
            make_installed_package(("test", "behind")),
            make_installed_package(("test", "diverged")),
            make_installed_package(("test", "uptodate")),
        ];
        model
            .expect_get_installed_packages_list()
            .return_once(move || Ok(pkgs));

        // Set up lineage so From<PackageLineage> produces the expected status.
        // Status is derived from base_hash vs current_hash (ahead) and
        // base_hash vs latest_hash (behind).
        model
            .expect_get_installed_package_lineage()
            .returning(|pkg| {
                let ns = pkg.namespace.to_string();
                let uri = make_manifest_uri(&ns);
                // base_hash comes from uri.hash ("abcdef")
                let lineage = match ns.as_str() {
                    // Ahead: current_hash != base_hash, base_hash == latest_hash
                    "test/ahead" => {
                        let mut l =
                            quilt::lineage::PackageLineage::from_remote(uri, "abcdef".into());
                        l.commit = Some(quilt::lineage::CommitState {
                            hash: "local1".into(),
                            ..Default::default()
                        });
                        l
                    }
                    // Behind: base_hash != latest_hash, current_hash == base_hash
                    "test/behind" => {
                        quilt::lineage::PackageLineage::from_remote(uri, "remote1".into())
                    }
                    // Diverged: both ahead and behind
                    "test/diverged" => {
                        let mut l =
                            quilt::lineage::PackageLineage::from_remote(uri, "remote2".into());
                        l.commit = Some(quilt::lineage::CommitState {
                            hash: "local2".into(),
                            ..Default::default()
                        });
                        l
                    }
                    // UpToDate: all hashes match
                    _ => quilt::lineage::PackageLineage::from_remote(uri, "abcdef".into()),
                };
                Ok(lineage)
            });

        // Not an access test: the roster's bucket query is unavailable,
        // which degrades to reactive-only marking and leaves rows untouched.
        without_bucket_list(&mut model);
        let tracing = crate::telemetry::Telemetry::default();
        let data = get_installed_packages_list_data_from_model(
            &model,
            &RoleCache::default(),
            &tracing,
            &HashMap::new(),
        )
        .await
        .map_err(|e| e.to_string())?;

        assert_eq!(data.packages.len(), 4);

        let find = |ns: &str| data.packages.iter().find(|p| p.namespace == ns).unwrap();

        // Spot-check URI propagation on one package; the other packages
        // share the same fixture shape, and `S3PackageUri::from(&ManifestUri)`
        // is exercised by quilt-uri's own tests.
        let ahead = find("test/ahead");
        assert_eq!(ahead.status, "ahead");
        assert!(!ahead.has_changes); // Light phase always returns false
        let ahead_uri = ahead.uri.as_ref().expect("URI present");
        assert_eq!(ahead_uri.bucket, "test");
        assert_eq!(ahead_uri.namespace.to_string(), "test/ahead");
        assert_eq!(
            catalog_host(ahead.uri.as_ref()).as_deref(),
            Some("test.quilt.dev")
        );
        assert!(ahead.remote_display.is_some());

        // For the rest, only the status mapping is the point of this test.
        assert_eq!(find("test/behind").status, "behind");
        assert_eq!(find("test/diverged").status, "diverged");
        assert_eq!(find("test/uptodate").status, "up_to_date");

        Ok(())
    }

    #[tokio::test]
    async fn test_installed_packages_list_data_with_origin_shows_cached_status()
    -> Result<(), String> {
        let mut model = mocks::create();

        let pkgs = vec![make_installed_package(("test", "pkg"))];
        model
            .expect_get_installed_packages_list()
            .return_once(move || Ok(pkgs));

        // Lineage indicates up_to_date (base == latest == remote hash)
        model
            .expect_get_installed_package_lineage()
            .returning(|pkg| {
                let uri = make_manifest_uri(&pkg.namespace.to_string());
                Ok(quilt::lineage::PackageLineage::from_remote(
                    uri,
                    "abcdef".to_string(),
                ))
            });

        // Not an access test: the roster's bucket query is unavailable,
        // which degrades to reactive-only marking and leaves rows untouched.
        without_bucket_list(&mut model);
        let tracing = crate::telemetry::Telemetry::default();
        let data = get_installed_packages_list_data_from_model(
            &model,
            &RoleCache::default(),
            &tracing,
            &HashMap::new(),
        )
        .await
        .map_err(|e| e.to_string())?;

        assert_eq!(data.packages.len(), 1);
        let pkg = &data.packages[0];
        assert_eq!(pkg.namespace, "test/pkg");
        // Light phase derives status from lineage (up_to_date, not error)
        assert_eq!(pkg.status, "up_to_date");
        assert!(!pkg.has_changes); // Always false in light phase
        assert_eq!(
            catalog_host(pkg.uri.as_ref()).as_deref(),
            Some("test.quilt.dev")
        );

        Ok(())
    }

    #[tokio::test]
    async fn test_installed_packages_list_data_no_origin() -> Result<(), String> {
        let mut model = mocks::create();

        let pkgs = vec![make_installed_package(("test", "noorigin"))];
        model
            .expect_get_installed_packages_list()
            .return_once(move || Ok(pkgs));

        // Remote URI exists but has no origin → triggers early return with error status
        model
            .expect_get_installed_package_lineage()
            .returning(|pkg| {
                let uri = make_manifest_uri_no_origin(&pkg.namespace.to_string());
                Ok(quilt::lineage::PackageLineage::from_remote(
                    uri,
                    "abcdef".to_string(),
                ))
            });

        let tracing = crate::telemetry::Telemetry::default();
        let data = get_installed_packages_list_data_from_model(
            &model,
            &RoleCache::default(),
            &tracing,
            &HashMap::new(),
        )
        .await
        .map_err(|e| e.to_string())?;

        assert_eq!(data.packages.len(), 1);
        let pkg = &data.packages[0];
        assert_eq!(pkg.namespace, "test/noorigin");
        assert_eq!(pkg.status, "error");
        // URI is exposed (so the "Set remote" popup can pre-fill bucket)
        // but its catalog is unset.
        let pkg_uri = pkg.uri.as_ref().expect("URI present");
        assert_eq!(pkg_uri.bucket, "test");
        assert!(pkg_uri.catalog.is_none());
        // remote_display should still be present
        assert!(pkg.remote_display.is_some());

        Ok(())
    }

    #[tokio::test]
    async fn test_installed_packages_list_data_local_without_remote() -> Result<(), String> {
        let mut model = mocks::create();

        let pkgs = vec![make_installed_package(("test", "local"))];
        model
            .expect_get_installed_packages_list()
            .return_once(move || Ok(pkgs));

        // No remote_uri at all → local-only package
        model
            .expect_get_installed_package_lineage()
            .returning(|_| Ok(quilt::lineage::PackageLineage::default()));

        let tracing = crate::telemetry::Telemetry::default();
        let data = get_installed_packages_list_data_from_model(
            &model,
            &RoleCache::default(),
            &tracing,
            &HashMap::new(),
        )
        .await
        .map_err(|e| e.to_string())?;

        assert_eq!(data.packages.len(), 1);
        let pkg = &data.packages[0];
        assert_eq!(pkg.namespace, "test/local");
        assert_eq!(pkg.status, "local");
        assert!(pkg.uri.is_none());
        assert!(pkg.remote_display.is_none());

        Ok(())
    }

    #[tokio::test]
    async fn test_installed_packages_list_data_local_with_origin() -> Result<(), String> {
        let mut model = mocks::create();

        let pkgs = vec![make_installed_package(("test", "localpush"))];
        model
            .expect_get_installed_packages_list()
            .return_once(move || Ok(pkgs));

        // Has remote URI with origin but never pushed (empty hash → Local)
        model
            .expect_get_installed_package_lineage()
            .returning(|pkg| {
                let uri = quilt_uri::ManifestUri {
                    origin: Some("test.quilt.dev".parse().unwrap()),
                    bucket: "test".to_string(),
                    namespace: pkg.namespace.clone(),
                    hash: String::new(),
                };
                Ok(quilt::lineage::PackageLineage::from_remote(
                    uri,
                    String::new(),
                ))
            });

        // Not an access test: the roster's bucket query is unavailable,
        // which degrades to reactive-only marking and leaves rows untouched.
        without_bucket_list(&mut model);
        let tracing = crate::telemetry::Telemetry::default();
        let data = get_installed_packages_list_data_from_model(
            &model,
            &RoleCache::default(),
            &tracing,
            &HashMap::new(),
        )
        .await
        .map_err(|e| e.to_string())?;

        assert_eq!(data.packages.len(), 1);
        let pkg = &data.packages[0];
        assert_eq!(pkg.namespace, "test/localpush");
        assert_eq!(pkg.status, "local");
        assert!(!pkg.has_changes);
        // Has origin (for Push button and disabled Catalog button in UI).
        assert_eq!(
            catalog_host(pkg.uri.as_ref()).as_deref(),
            Some("test.quilt.dev")
        );

        Ok(())
    }

    // ── refresh_package_status tests (heavy phase) ──

    #[tokio::test]
    async fn test_refresh_package_status_local_only_no_changes() -> Result<(), String> {
        let mut model = mocks::create();
        let pkg = make_installed_package(("test", "local"));
        model
            .expect_get_installed_package()
            .return_once(move |_| Ok(Some(pkg)));
        model
            .expect_get_installed_package_lineage()
            .returning(|_| Ok(quilt::lineage::PackageLineage::default()));
        model
            .expect_get_installed_package_status()
            .returning(|_, _| {
                Ok(quilt::lineage::InstalledPackageStatus::new(
                    quilt::lineage::UpstreamState::Local,
                    quilt::lineage::ChangeSet::new(),
                ))
            });

        let tracing = crate::telemetry::Telemetry::default();
        let ns = ("test", "local").into();
        let result =
            refresh_package_status_from_model(&model, &RoleCache::default(), &tracing, &ns)
                .await
                .map_err(|e| e.to_string())?;

        assert_eq!(result.status, "local");
        assert!(!result.has_changes);
        Ok(())
    }

    #[tokio::test]
    async fn test_refresh_package_status_local_only_with_changes() -> Result<(), String> {
        let mut model = mocks::create();
        let pkg = make_installed_package(("test", "local"));
        model
            .expect_get_installed_package()
            .return_once(move |_| Ok(Some(pkg)));
        model
            .expect_get_installed_package_lineage()
            .returning(|_| Ok(quilt::lineage::PackageLineage::default()));
        model
            .expect_get_installed_package_status()
            .returning(|_, _| {
                let mut changes = quilt::lineage::ChangeSet::new();
                changes.insert(
                    std::path::PathBuf::from("file.txt"),
                    quilt::lineage::Change::Added(quilt::manifest::ManifestRow::default()),
                );
                Ok(quilt::lineage::InstalledPackageStatus::new(
                    quilt::lineage::UpstreamState::Local,
                    changes,
                ))
            });

        let tracing = crate::telemetry::Telemetry::default();
        let ns = ("test", "local").into();
        let result =
            refresh_package_status_from_model(&model, &RoleCache::default(), &tracing, &ns)
                .await
                .map_err(|e| e.to_string())?;

        assert_eq!(result.status, "local");
        assert!(result.has_changes);
        Ok(())
    }

    #[tokio::test]
    async fn test_refresh_package_status_no_origin() -> Result<(), String> {
        let mut model = mocks::create();
        let pkg = make_installed_package(("test", "noorigin"));
        model
            .expect_get_installed_package()
            .return_once(move |_| Ok(Some(pkg)));
        model
            .expect_get_installed_package_lineage()
            .returning(|pkg| {
                let uri = make_manifest_uri_no_origin(&pkg.namespace.to_string());
                Ok(quilt::lineage::PackageLineage::from_remote(
                    uri,
                    "abcdef".to_string(),
                ))
            });

        let tracing = crate::telemetry::Telemetry::default();
        let ns = ("test", "noorigin").into();
        let result =
            refresh_package_status_from_model(&model, &RoleCache::default(), &tracing, &ns)
                .await
                .map_err(|e| e.to_string())?;

        assert_eq!(result.status, "error");
        assert!(!result.has_changes);
        Ok(())
    }

    #[tokio::test]
    async fn test_refresh_package_status_with_changes() -> Result<(), String> {
        let mut model = mocks::create();
        let pkg = make_installed_package(("test", "changed"));
        model
            .expect_get_installed_package()
            .return_once(move |_| Ok(Some(pkg)));
        model
            .expect_get_installed_package_lineage()
            .returning(|pkg| {
                let uri = make_manifest_uri(&pkg.namespace.to_string());
                Ok(quilt::lineage::PackageLineage::from_remote(
                    uri,
                    "abcdef".to_string(),
                ))
            });
        model
            .expect_get_installed_package_status()
            .returning(|_, _| {
                let mut changes = quilt::lineage::ChangeSet::new();
                changes.insert(
                    std::path::PathBuf::from("file.txt"),
                    quilt::lineage::Change::Added(quilt::manifest::ManifestRow::default()),
                );
                Ok(quilt::lineage::InstalledPackageStatus::new(
                    quilt::lineage::UpstreamState::UpToDate,
                    changes,
                ))
            });

        let tracing = crate::telemetry::Telemetry::default();
        let ns = ("test", "changed").into();
        let result =
            refresh_package_status_from_model(&model, &RoleCache::default(), &tracing, &ns)
                .await
                .map_err(|e| e.to_string())?;

        assert_eq!(result.status, "up_to_date");
        assert!(result.has_changes);
        Ok(())
    }

    #[tokio::test]
    async fn test_refresh_package_status_without_changes() -> Result<(), String> {
        let mut model = mocks::create();
        let pkg = make_installed_package(("test", "clean"));
        model
            .expect_get_installed_package()
            .return_once(move |_| Ok(Some(pkg)));
        model
            .expect_get_installed_package_lineage()
            .returning(|pkg| {
                let uri = make_manifest_uri(&pkg.namespace.to_string());
                Ok(quilt::lineage::PackageLineage::from_remote(
                    uri,
                    "remote1".to_string(),
                ))
            });
        model
            .expect_get_installed_package_status()
            .returning(|_, _| {
                Ok(quilt::lineage::InstalledPackageStatus::new(
                    quilt::lineage::UpstreamState::Behind,
                    quilt::lineage::ChangeSet::default(),
                ))
            });

        let tracing = crate::telemetry::Telemetry::default();
        let ns = ("test", "clean").into();
        let result =
            refresh_package_status_from_model(&model, &RoleCache::default(), &tracing, &ns)
                .await
                .map_err(|e| e.to_string())?;

        assert_eq!(result.status, "behind");
        assert!(!result.has_changes);
        Ok(())
    }

    #[tokio::test]
    async fn test_refresh_package_status_error_on_status_fetch() -> Result<(), String> {
        let mut model = mocks::create();
        let pkg = make_installed_package(("test", "broken"));
        model
            .expect_get_installed_package()
            .return_once(move |_| Ok(Some(pkg)));
        model
            .expect_get_installed_package_lineage()
            .returning(|pkg| {
                let uri = make_manifest_uri(&pkg.namespace.to_string());
                Ok(quilt::lineage::PackageLineage::from_remote(
                    uri,
                    "abcdef".to_string(),
                ))
            });
        model
            .expect_get_installed_package_status()
            .returning(|_, _| Err(crate::error::Error::General("network error".to_string())));

        let tracing = crate::telemetry::Telemetry::default();
        let ns = ("test", "broken").into();
        let result =
            refresh_package_status_from_model(&model, &RoleCache::default(), &tracing, &ns)
                .await
                .map_err(|e| e.to_string())?;

        assert_eq!(result.status, "error");
        assert!(!result.has_changes);
        Ok(())
    }

    // ── paused_reason population (data-driven red state) ──

    #[tokio::test]
    async fn test_installed_packages_list_data_populates_paused_reason() -> Result<(), String> {
        let mut model = mocks::create();

        let pkgs = vec![
            make_installed_package(("test", "paused")),
            make_installed_package(("test", "clean")),
        ];
        model
            .expect_get_installed_packages_list()
            .return_once(move || Ok(pkgs));
        model
            .expect_get_installed_package_lineage()
            .returning(|pkg| {
                let uri = make_manifest_uri(&pkg.namespace.to_string());
                Ok(quilt::lineage::PackageLineage::from_remote(
                    uri,
                    "abcdef".to_string(),
                ))
            });

        // Stand in for the watcher's paused map: only the paused namespace
        // has an `Other` message.
        let mut paused_reasons = HashMap::new();
        paused_reasons.insert(
            "test/paused".to_string(),
            PausedRow {
                reason: "other".to_string(),
                message: "workflow rejected metadata".to_string(),
            },
        );

        // Not an access test: the roster's bucket query is unavailable,
        // which degrades to reactive-only marking and leaves rows untouched.
        without_bucket_list(&mut model);
        let tracing = crate::telemetry::Telemetry::default();
        let data = get_installed_packages_list_data_from_model(
            &model,
            &RoleCache::default(),
            &tracing,
            &paused_reasons,
        )
        .await
        .map_err(|e| e.to_string())?;

        let find = |ns: &str| data.packages.iter().find(|p| p.namespace == ns).unwrap();
        assert_eq!(
            find("test/paused").paused_reason.as_deref(),
            Some("workflow rejected metadata"),
        );
        assert!(find("test/clean").paused_reason.is_none());
        Ok(())
    }

    /// The serialized row must be byte-identical to what the UI mirror
    /// (`quilt_sync_ui::commands::PackageItemData`) deserializes in its
    /// `package_item_data_wire_form_is_verbatim`. If the two drift, the
    /// list silently drops the pause reason (or a whole field) at the
    /// Tauri boundary.
    #[test]
    fn package_item_data_wire_form_is_verbatim() {
        let item = InstalledPackageListItem {
            namespace: "acme/data".to_string(),
            status: "paused".to_string(),
            has_changes: false,
            has_local_commit: false,
            uri: None,
            remote_display: None,
            paused_reason: Some("workflow rejected metadata".to_string()),
            paused_kind: Some("other".to_string()),
            no_access: true,
            no_access_reason: Some(
                "Current role ReadOnly has no access to this bucket".to_string(),
            ),
            role_switch_host: Some("acme.quilt.dev".to_string()),
        };
        assert_eq!(
            serde_json::to_string(&item).unwrap(),
            r#"{"namespace":"acme/data","status":"paused","hasChanges":false,"hasLocalCommit":false,"uri":null,"remoteDisplay":null,"pausedReason":"workflow rejected metadata","pausedKind":"other","noAccess":true,"noAccessReason":"Current role ReadOnly has no access to this bucket","roleSwitchHost":"acme.quilt.dev"}"#
        );
    }
}
