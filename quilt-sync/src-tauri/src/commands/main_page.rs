//! v2's main-page payloads. Nothing here is shared with `package_list.rs`, which
//! is v1's, frozen, and deleted once v2 ships.

use std::collections::HashMap;
use std::collections::HashSet;
use std::time::Duration;

use serde::Serialize;
use tokio::time::timeout;

use quilt_rs::RoleInfo;
use quilt_uri::Host;

use crate::autopull::Watcher;
use crate::commands::RoleCache;
use crate::error::Error;
use crate::model;
use crate::quilt;
use crate::quilt::lineage::UpstreamState;

/// A package's resolved state, as the UI's `kit::PackageState` expects it.
///
/// §2: a discriminator, never prose. The UI owns the words; a rewording must not
/// need a backend release.
///
/// `PullConflict` and `RoleDenied` are never constructed by `resolve_state` in
/// this build (see its doc comment) — they exist only for wire-shape stability,
/// hence the blanket `dead_code` allowance.
#[allow(dead_code)]
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PackageStateDto {
    Latest,
    Behind,
    PendingChanges {
        files: usize,
    },
    PendingCommit,
    Diverged,
    PullConflict {
        files: Vec<String>,
    },
    /// `None` when the denial is certain but the role query behind the wording
    /// failed. The denial still stands — the bucket refused — so suppressing the
    /// state would lose a real fact; it simply cannot be named.
    RoleDenied {
        role: Option<String>,
    },
    NoRemote,
    Unpublished,
    /// `UpstreamState::Error`. The UI's `PackageState` catches this with
    /// `#[serde(other)]`, the same arm that catches a kind added after this build.
    Unknown,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MainPagePackage {
    pub namespace: String,
    pub state: PackageStateDto,
    /// Epoch milliseconds. The backend being generous with a format the UI can use
    /// directly, rather than the UI carrying date arithmetic.
    ///
    /// Epoch milliseconds: the most recent thing that happened to this copy.
    ///
    /// `None` only when nothing has, which is genuine rather than a gap — see
    /// [`last_changed`].
    pub changed_at: Option<f64>,
    pub bucket: Option<String>,
    /// True while the state came from cached lineage alone. The heavy phase clears
    /// it. Every row in this build is provisional, because the heavy phase does not
    /// exist yet.
    pub provisional: bool,
    /// The host whose role selector this row's switch affordance opens. `Some`
    /// only when the user holds more than one role there, so the affordance is
    /// never a dead end: a single-role user gets the state and no button.
    ///
    /// The denial itself is not a second field — it is `state ==
    /// RoleDenied`. Two fields carrying one fact is §1's rule at payload scale.
    pub role_switch_host: Option<String>,
    pub paused_reason: Option<String>,
    pub paused_kind: Option<String>,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MainPagePackages {
    pub packages: Vec<MainPagePackage>,
}

/// The light phase's resolution: **map** the state `quilt-rs` already derived.
///
/// It does NOT re-derive from hashes. `quilt-rs` computes `UpstreamState` from a
/// lineage via `let upstream: UpstreamState = lineage.into()`, and a second
/// resolver in the tree is exactly what §1 forbids — "resolution happens exactly
/// once, upstream of every payload". Two places deciding what is true is the
/// 2026-07-11 bug.
///
/// The two extra booleans are not a second resolution: they split one
/// `UpstreamState` variant that v2's vocabulary distinguishes and v1's did not.
///
/// `changed_files` is what the heavy phase measured — `None` when nobody has
/// looked yet, which is the light phase. It can only reach rank 7 of §5's
/// precedence lattice: `Diverged` and `Behind` outrank it, and a package with
/// nowhere to publish to has no use for a file count.
fn resolve_state(
    upstream: UpstreamState,
    has_local_commit: bool,
    has_remote: bool,
    changed_files: Option<usize>,
) -> PackageStateDto {
    // A working tree measured as non-empty. Rank 7 only; see the doc comment.
    let pending_changes = match changed_files {
        Some(files) if files > 0 => Some(PackageStateDto::PendingChanges { files }),
        _ => None,
    };

    match upstream {
        // `Local` means either no bucket chosen, or a bucket with nothing in it yet.
        // v1 called both "no remote"; v2 has a word for each.
        UpstreamState::Local if has_remote => PackageStateDto::Unpublished,
        UpstreamState::Local => PackageStateDto::NoRemote,
        UpstreamState::Behind => PackageStateDto::Behind,
        UpstreamState::Diverged => PackageStateDto::Diverged,
        UpstreamState::Error => PackageStateDto::Unknown,
        UpstreamState::Ahead => pending_changes.unwrap_or(PackageStateDto::PendingCommit),
        UpstreamState::UpToDate => pending_changes.unwrap_or({
            if has_local_commit {
                PackageStateDto::PendingCommit
            } else {
                PackageStateDto::Latest
            }
        }),
    }
}

/// A remote with a bucket but no catalog host.
///
/// `impl From<PackageLineage> for UpstreamState` deliberately ignores `origin`
/// (`quilt-rs/src/lineage/package.rs:179-186`) and answers from the hashes, which
/// for this shape is a state the app cannot act on: without a catalog there is
/// nowhere to vend credentials from. Both phases check this BEFORE resolving, so
/// there is one answer to the question rather than one per phase.
///
/// v2's word for it is `Unknown` — "Sync stopped" — which is where v1's `error`
/// status lands too (`package_list.rs:311-324`).
fn misconfigured_remote(lineage: &quilt::lineage::PackageLineage) -> bool {
    lineage
        .remote_uri
        .as_ref()
        .is_some_and(|uri| uri.origin.is_none())
}

// `PullConflict` and `RoleDenied` are not produced by `resolve_state`: the first
// comes from the watcher's paused map (Plan 3), the second from the access pass
// below, which resolves a denied row after the state has been mapped.

/// A message-bearing autosync pause for a namespace, keyed by namespace in the
/// map the loop below consumes. Deliberately not `package_list.rs`'s `PausedRow`:
/// that type is v1's and frozen, even though the shape is the same.
struct PausedRow {
    kind: String,
    message: String,
}

/// How long the roster waits on one host's readable-bucket query before giving
/// up on it.
///
/// COPIED from `package_list.rs:160` per `qhq-8mgw.1`, not shared: v1 is frozen
/// and deleted wholesale. Mirror fixes in both until then.
///
/// The roster is otherwise local data and paints without touching the network.
/// The query adds two round trips per host — `config.json`, then a GraphQL POST
/// — serialised under that host's credential lock behind a retry middleware with
/// a 10s connect timeout, so an unreachable host held the main screen blank for
/// tens of seconds. The pre-filter is an optimistic hint with a correct degrade
/// path (reactive-only marking), so the budget is deliberately short:
/// undershooting on a slow-but-working link costs only the hint; overshooting
/// costs the first paint.
const BUCKET_LIST_BUDGET: Duration = Duration::from_secs(2);

/// Whether the active role can reach a row's bucket, and what to say about it.
///
/// COPIED from `package_list.rs:81-87`. v2 carries the ROLE NAME rather than
/// v1's rendered `reason` string: §2 keeps prose off the wire, and
/// `kit::render` already words `RoleDenied` for both sites.
#[derive(Clone, Debug, Default)]
struct AccessMark {
    /// Read by Task 4's reactive marking to decide whether the heavy phase's
    /// own resolved state should be overridden. This pass only ever
    /// constructs a mark when denying, so the field is unread here.
    #[allow(dead_code)]
    denied: bool,
    role: Option<String>,
    role_switch_host: Option<String>,
}

impl AccessMark {
    /// A denial. `roles` is `None` when the role query itself failed: the mark
    /// still stands (the bucket refused either way), it just cannot name the
    /// role, and it offers no switch because we cannot tell whether another
    /// role is held.
    fn denied(host: Option<&Host>, roles: Option<&RoleInfo>) -> Self {
        let holds_another_role = roles.is_some_and(|roles| roles.available.len() > 1);
        Self {
            denied: true,
            role: roles.map(|roles| roles.current.clone()),
            role_switch_host: host
                .filter(|_| holds_another_role)
                .map(std::string::ToString::to_string),
        }
    }

    /// The state a denied row resolves to. Ruling R3: denial is precedence rank
    /// 1, so it replaces the mapped state rather than riding beside it.
    fn state(&self) -> PackageStateDto {
        PackageStateDto::RoleDenied {
            role: self.role.clone(),
        }
    }
}

/// Ask who the active role on `host` is, so a denial can name it.
///
/// COPIED from `package_list.rs:133-146`. Goes through the [`RoleCache`], so the
/// answer is fetched once per host per load however many rows deny — the heavy
/// phase runs one command invocation per row, concurrently, and they all land
/// here. A failed query is not fatal: the mark degrades to unnamed.
async fn denied_mark(
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

/// A row plus the URI the access pass needs and the wire does not carry.
///
/// §4.4 lists `bucket` and no URI, so the host lives here for the length of the
/// walk rather than on the DTO, where it would exist only for the backend's own
/// convenience.
struct Row {
    package: MainPagePackage,
    uri: Option<quilt_uri::S3PackageUri>,
}

fn row_host(row: &Row) -> Option<&Host> {
    row.uri.as_ref().and_then(|uri| uri.catalog.as_ref())
}

/// Resolve the rows whose bucket is outside what the active role can read.
///
/// COPIED from `package_list.rs:232-283`. One query per host, not per package.
/// The answer is an optimistic hint — it over-reports for unmanaged roles and
/// anonymous-access stacks, and says nothing about writes — so a miss only greys
/// the row; the authoritative answer still comes from the per-row status call,
/// which clears the mark in both directions.
///
/// A failed query degrades to reactive-only marking. It must NEVER be read as
/// "nothing is readable": an empty set would grey the entire roster. A query
/// that does not answer inside [`BUCKET_LIST_BUDGET`] counts as failed, which is
/// what keeps the roster local-only in effect.
async fn mark_unreadable_buckets(m: &impl model::QuiltModel, roles: &RoleCache, rows: &mut [Row]) {
    let mut hosts: Vec<Host> = Vec::new();
    for host in rows.iter().filter_map(row_host) {
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

        let unreadable: Vec<usize> = rows
            .iter()
            .enumerate()
            .filter(|(_, row)| row_host(row) == Some(&host))
            .filter(|(_, row)| {
                row.uri
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
            let row = &mut rows[index];
            row.package.state = mark.state();
            row.package
                .role_switch_host
                .clone_from(&mark.role_switch_host);
        }
    }
}

async fn get_main_page_packages_from_model(
    m: &impl model::QuiltModel,
    roles: &RoleCache,
    tracing: &crate::telemetry::Telemetry,
    paused_reasons: &HashMap<String, PausedRow>,
) -> Result<MainPagePackages, Error> {
    // COPIED from `package_list.rs:193` per `qhq-8mgw.1`. A load is the cadence
    // the role refresh is pinned to. A switch is server-side and global, so it can
    // happen in the web catalog with the app none the wiser; held for a whole
    // session, the cached name would make a row name a role that in fact has
    // access, and — worse — `observe_role` would never re-run, leaving the S3
    // clients signing as the old role until Settings is opened or the ~1h
    // credential TTL expires. Every host, not just the roster's: the roster is not
    // known until the list below is fetched, and an entry is only a name.
    roles.invalidate(None).await;

    let list = m.get_installed_packages_list().await?;
    let mut rows = Vec::new();
    for installed_package in list {
        match load_main_page_package(m, tracing, &installed_package, paused_reasons).await {
            Ok(row) => rows.push(row),
            Err(err) => {
                tracing::warn!(
                    "Failed to load package {}: {err}",
                    installed_package.namespace,
                );
            }
        }
    }
    mark_unreadable_buckets(m, roles, &mut rows).await;
    Ok(MainPagePackages {
        packages: rows.into_iter().map(|row| row.package).collect(),
    })
}

/// When this copy last changed: the last local commit, or the last file we installed
/// or committed, whichever is later. Epoch milliseconds.
///
/// **Not a filesystem mtime, and no directory walk happens here.** Both values come
/// out of `data.json`, which the light phase has already read and deserialized —
/// `qhq-8mgw.3` called this plumbing rather than I/O, and it was right.
///
/// `PathState`'s own doc says why the distinction matters: *"We don't track files
/// modifications in real time. We calculate hash when we commit or install file."* So
/// this is the last time `QuiltSync` touched the copy, not the last time anything on
/// disk did. A file edited in the working directory since does not move it — that is
/// what the heavy phase's hashing is for.
///
/// `None` means we have never written to this package: no commit, and no installed
/// paths. That is a real answer, not a missing one, which is why the row says
/// `not recorded` rather than leaving the cell blank.
fn last_changed(lineage: &quilt::lineage::PackageLineage) -> Option<f64> {
    let newest = lineage
        .commit
        .as_ref()
        .map(|commit| commit.timestamp)
        .into_iter()
        .chain(lineage.paths.values().map(|path| path.timestamp).max())
        .max()?;
    // i64 milliseconds into f64: exact until year 287396, and `f64` is what crosses
    // the wire because JavaScript has no other number.
    #[allow(
        clippy::cast_precision_loss,
        reason = "epoch millis fit f64 exactly for any date this program can see"
    )]
    Some(newest.timestamp_millis() as f64)
}

async fn load_main_page_package(
    m: &impl model::QuiltModel,
    tracing: &crate::telemetry::Telemetry,
    installed_package: &quilt::InstalledPackage,
    paused_reasons: &HashMap<String, PausedRow>,
) -> Result<Row, Error> {
    let namespace = installed_package.namespace.to_string();
    let paused = paused_reasons.get(&namespace);
    let paused_reason = paused.map(|p| p.message.clone());
    let paused_kind = paused.map(|p| p.kind.clone());

    let lineage = m.get_installed_package_lineage(installed_package).await?;
    // Computed before `lineage` is moved by the `into()` below.
    let has_local_commit = lineage.commit.is_some();
    let has_remote = lineage.remote_uri.is_some();
    let bucket = lineage.remote_uri.as_ref().map(|uri| uri.bucket.clone());
    let changed_at = last_changed(&lineage);
    let typed_uri = lineage
        .remote_uri
        .as_ref()
        .map(quilt_uri::S3PackageUri::from);
    // Crash reports are attributed to the deployment of the most recent action.
    // v1 does this in its own light phase (`package_list.rs:334`); without it here,
    // the attribution dies with v1.
    if let Some(host) = typed_uri.as_ref().and_then(|uri| uri.catalog.as_ref()) {
        tracing.add_host(host);
    }
    let state = if misconfigured_remote(&lineage) {
        PackageStateDto::Unknown
    } else {
        // `None`, not `Some(0)`: this phase has not looked at the working tree.
        // The heavy phase (`refresh_main_page_package`) measures it.
        resolve_state(lineage.into(), has_local_commit, has_remote, None)
    };

    Ok(Row {
        package: MainPagePackage {
            namespace,
            state,
            changed_at,
            bucket,
            provisional: true,
            role_switch_host: None,
            paused_reason,
            paused_kind,
        },
        uri: typed_uri,
    })
}

/// Walk the installed packages, read each one's cached lineage, resolve, and
/// attach the paused reason from the watcher's authoritative map. A failed
/// package is warned-and-skipped, per `package_list.rs:198-206` — one bad
/// lineage must not blank the whole list.
#[tauri::command]
pub async fn get_main_page_packages(
    m: tauri::State<'_, model::Model>,
    roles: tauri::State<'_, RoleCache>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
    watcher: tauri::State<'_, Watcher>,
) -> Result<MainPagePackages, String> {
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
                        kind: entry.reason,
                        message,
                    },
                )
            })
        })
        .collect();

    let started = std::time::Instant::now();
    let result = get_main_page_packages_from_model(&*m, &roles, &tracing, &paused_reasons).await;
    tracing::debug!(
        elapsed_ms = started.elapsed().as_millis(),
        packages = result.as_ref().map_or(0, |r| r.packages.len()),
        "main page light phase"
    );
    result.map_err(|e| e.to_frontend_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::commands::test_support::*;
    use crate::quilt::lineage::UpstreamState;

    #[test]
    fn local_with_no_bucket_is_no_remote() {
        assert_eq!(
            resolve_state(UpstreamState::Local, false, false, None),
            PackageStateDto::NoRemote
        );
    }

    #[test]
    fn local_with_a_bucket_is_unpublished() {
        // `UpstreamState::Local` covers BOTH "no remote configured" and "remote set
        // but never pushed" — its own doc comment says so. v2 splits them.
        assert_eq!(
            resolve_state(UpstreamState::Local, false, true, None),
            PackageStateDto::Unpublished
        );
    }

    #[test]
    fn up_to_date_with_nothing_local_is_latest() {
        assert_eq!(
            resolve_state(UpstreamState::UpToDate, false, true, None),
            PackageStateDto::Latest
        );
    }

    #[test]
    fn up_to_date_with_a_local_revision_is_pending_commit() {
        assert_eq!(
            resolve_state(UpstreamState::UpToDate, true, true, None),
            PackageStateDto::PendingCommit
        );
    }

    #[test]
    fn ahead_is_pending_commit() {
        assert_eq!(
            resolve_state(UpstreamState::Ahead, false, true, None),
            PackageStateDto::PendingCommit
        );
    }

    #[test]
    fn behind_carries_no_count() {
        assert_eq!(
            resolve_state(UpstreamState::Behind, false, true, None),
            PackageStateDto::Behind,
            "a hash inequality is not a distance; no revision count is derivable"
        );
    }

    #[test]
    fn diverged_maps_straight_through() {
        assert_eq!(
            resolve_state(UpstreamState::Diverged, false, true, None),
            PackageStateDto::Diverged
        );
    }

    #[test]
    fn error_becomes_the_fallback() {
        assert_eq!(
            resolve_state(UpstreamState::Error, false, true, None),
            PackageStateDto::Unknown
        );
    }

    /// A lineage carrying only the timestamps `last_changed` reads.
    fn lineage_with(commit_ms: Option<i64>, path_ms: &[i64]) -> quilt::lineage::PackageLineage {
        let mut lineage = quilt::lineage::PackageLineage::default();
        lineage.commit = commit_ms.map(|ms| quilt::lineage::CommitState {
            timestamp: chrono::DateTime::from_timestamp_millis(ms).unwrap(),
            hash: String::new(),
            prev_hashes: Vec::new(),
        });
        for (i, ms) in path_ms.iter().enumerate() {
            lineage.paths.insert(
                std::path::PathBuf::from(format!("f{i}.csv")),
                quilt::lineage::PathState {
                    timestamp: chrono::DateTime::from_timestamp_millis(*ms).unwrap(),
                    // `Multihash` by name would mean taking the `multihash` crate as a
                    // direct dependency of this one — it is not, and `quilt-rs` does not
                    // re-export it — purely to spell a test fixture's zero value.
                    #[allow(
                        clippy::default_trait_access,
                        reason = "the type is not nameable here without a new dependency"
                    )]
                    hash: Default::default(),
                },
            );
        }
        lineage
    }

    #[test]
    fn last_changed_takes_the_newest_path_when_it_beats_the_commit() {
        let l = lineage_with(Some(1_000), &[5_000, 3_000]);
        assert_eq!(last_changed(&l), Some(5_000.0));
    }

    #[test]
    fn last_changed_takes_the_commit_when_it_beats_every_path() {
        let l = lineage_with(Some(9_000), &[5_000, 3_000]);
        assert_eq!(last_changed(&l), Some(9_000.0));
    }

    #[test]
    fn last_changed_is_none_only_when_nothing_has_ever_been_written() {
        // No commit and no installed paths — a real answer, not a gap.
        assert_eq!(last_changed(&lineage_with(None, &[])), None);
        // Paths but no commit still has an answer.
        assert_eq!(last_changed(&lineage_with(None, &[7_000])), Some(7_000.0));
    }

    #[test]
    fn a_measured_working_tree_beats_an_unpushed_revision() {
        // Rank 7 of the precedence lattice, both arms of it. Both offer Publish;
        // only one of them can say how much.
        assert_eq!(
            resolve_state(UpstreamState::UpToDate, true, true, Some(3)),
            PackageStateDto::PendingChanges { files: 3 }
        );
        assert_eq!(
            resolve_state(UpstreamState::Ahead, false, true, Some(2)),
            PackageStateDto::PendingChanges { files: 2 }
        );
    }

    #[test]
    fn a_measured_clean_tree_falls_through_to_the_revision_state() {
        assert_eq!(
            resolve_state(UpstreamState::UpToDate, false, true, Some(0)),
            PackageStateDto::Latest
        );
        assert_eq!(
            resolve_state(UpstreamState::UpToDate, true, true, Some(0)),
            PackageStateDto::PendingCommit
        );
        assert_eq!(
            resolve_state(UpstreamState::Ahead, false, true, Some(0)),
            PackageStateDto::PendingCommit
        );
    }

    #[test]
    fn a_count_never_outranks_a_state_above_it_in_the_lattice() {
        // §5: Diverged (5) and Behind (6) both outrank rank 7. The local edits are
        // real and they are not what this row is about; they show on the package page.
        assert_eq!(
            resolve_state(UpstreamState::Behind, false, true, Some(4)),
            PackageStateDto::Behind
        );
        assert_eq!(
            resolve_state(UpstreamState::Diverged, false, true, Some(4)),
            PackageStateDto::Diverged
        );
        // No bucket to publish to: the number is not the thing to say.
        assert_eq!(
            resolve_state(UpstreamState::Local, false, false, Some(4)),
            PackageStateDto::NoRemote
        );
        assert_eq!(
            resolve_state(UpstreamState::Local, false, true, Some(4)),
            PackageStateDto::Unpublished
        );
    }

    #[test]
    fn an_unmeasured_tree_agrees_with_a_measured_empty_one() {
        // Deliberate: `None` exists so the LIGHT PHASE'S CALL SITE cannot assert a
        // clean tree, not to produce a third state. If a future arm makes these
        // differ, that is a decision to take on purpose — this test is the tripwire.
        for upstream in [
            UpstreamState::UpToDate,
            UpstreamState::Ahead,
            UpstreamState::Behind,
            UpstreamState::Diverged,
            UpstreamState::Local,
            UpstreamState::Error,
        ] {
            for has_local_commit in [true, false] {
                assert_eq!(
                    resolve_state(upstream, has_local_commit, true, None),
                    resolve_state(upstream, has_local_commit, true, Some(0)),
                    "{upstream:?} / commit={has_local_commit} disagreed"
                );
            }
        }
    }

    #[test]
    fn a_remote_with_no_catalog_host_is_not_read_through_the_hashes() {
        // The classifier ignores `origin` on purpose and would answer from the hash
        // comparison — a state the app cannot act on, because without a catalog there
        // is nowhere to vend credentials from. v1 short-circuits the same case
        // (`package_list.rs:311`).
        let mut lineage = quilt::lineage::PackageLineage::from_remote(
            make_manifest_uri_no_origin("team/one"),
            "abcdef".to_string(),
        );
        lineage.latest_hash = "abcdef".to_string();
        assert!(misconfigured_remote(&lineage));

        // A remote WITH a catalog host, and a package with no remote at all, are both fine.
        assert!(!misconfigured_remote(
            &quilt::lineage::PackageLineage::from_remote(
                make_manifest_uri("team/one"),
                "abcdef".to_string(),
            )
        ));
        assert!(!misconfigured_remote(
            &quilt::lineage::PackageLineage::default()
        ));
    }

    #[test]
    fn the_wire_shape_is_a_discriminator_and_carries_no_words() {
        let json = serde_json::to_string(&PackageStateDto::Diverged).unwrap();
        assert_eq!(json, r#"{"kind":"diverged"}"#);
        assert!(
            !json.contains("Changed in both places"),
            "§2: the wire carries a discriminator, never prose"
        );
    }

    #[test]
    fn pending_changes_carries_the_count_because_the_ui_cannot_measure_it() {
        let json = serde_json::to_string(&PackageStateDto::PendingChanges { files: 2 }).unwrap();
        assert_eq!(json, r#"{"kind":"pending_changes","files":2}"#);
    }

    #[test]
    fn an_unnameable_role_crosses_the_wire_as_null_not_as_an_empty_name() {
        assert_eq!(
            serde_json::to_string(&PackageStateDto::RoleDenied { role: None }).unwrap(),
            r#"{"kind":"role_denied","role":null}"#
        );
        assert_eq!(
            serde_json::to_string(&PackageStateDto::RoleDenied {
                role: Some("ReadOnly".to_string())
            })
            .unwrap(),
            r#"{"kind":"role_denied","role":"ReadOnly"}"#
        );
    }

    /// Drives `get_main_page_packages_from_model`'s real loop over a mock model —
    /// the paused map, the per-package lineage read, and the warn-and-skip on a
    /// failed lineage — and pins the result's serialized JSON. That JSON is the
    /// contract the UI's `MainPagePackageData` must deserialize (`kit/package_state.rs`),
    /// so this one test buys both the loop's integration coverage and the wire-shape
    /// pin the final review asked for, rather than two separate tests.
    ///
    /// The access pass runs here too — `readable_buckets` fails, which degrades
    /// to reactive-only marking and marks nothing, so the unchanged `"latest"`
    /// state and null `roleSwitchHost` below are what that degrade asserts.
    #[tokio::test]
    async fn get_main_page_packages_from_model_serializes_the_wire_shape() {
        let mut model = crate::model::mocks::create();

        let pkgs = vec![
            make_installed_package(("team", "latest")),
            make_installed_package(("team", "broken")),
        ];
        model
            .expect_get_installed_packages_list()
            .return_once(move || Ok(pkgs));
        model
            .expect_get_installed_package_lineage()
            .returning(|pkg| {
                let ns = pkg.namespace.to_string();
                if ns == "team/broken" {
                    // A package whose lineage load fails must be skipped, not
                    // fail the whole list — asserted below via `packages.len()`.
                    return Err(access_denied_error());
                }
                Ok(quilt::lineage::PackageLineage::from_remote(
                    make_manifest_uri(&ns),
                    "abcdef".to_string(),
                ))
            });
        model
            .expect_readable_buckets()
            .returning(|_| Err(Error::General("no bucket list in this test".to_string())));

        let mut paused_reasons = HashMap::new();
        paused_reasons.insert(
            "team/latest".to_string(),
            PausedRow {
                kind: "workflow".to_string(),
                message: "blocked by workflow rule".to_string(),
            },
        );

        let result = get_main_page_packages_from_model(
            &model,
            &RoleCache::default(),
            &crate::telemetry::Telemetry::default(),
            &paused_reasons,
        )
        .await
        .expect("one bad lineage must not fail the whole list");

        assert_eq!(
            result.packages.len(),
            1,
            "team/broken's lineage failure must be skipped, not surfaced"
        );

        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "packages": [{
                    "namespace": "team/latest",
                    "state": {"kind": "latest"},
                    "changedAt": null,
                    "bucket": "test",
                    "provisional": true,
                    "roleSwitchHost": null,
                    "pausedReason": "blocked by workflow rule",
                    "pausedKind": "workflow",
                }]
            }),
            "this shape is what the UI's MainPagePackageData must deserialize"
        );
    }

    use crate::commands::RoleCache;
    use quilt_rs::RoleInfo;
    use quilt_uri::Host;

    /// A `ManifestUri` in an explicit bucket, so two rows can sit on one host in
    /// different buckets — the shape the readable-bucket intersection is about.
    fn make_manifest_uri_in_bucket(bucket: &str, namespace: &str) -> quilt_uri::ManifestUri {
        quilt_uri::ManifestUri {
            origin: Some("test.quilt.dev".parse().unwrap()),
            bucket: bucket.to_string(),
            namespace: namespace.try_into().unwrap(),
            hash: "abcdef".to_string(),
        }
    }

    /// A user holding two roles, the second of which is active.
    fn two_roles() -> RoleInfo {
        RoleInfo {
            current: "ReadOnly".to_string(),
            available: vec!["ReadWrite".to_string(), "ReadOnly".to_string()],
        }
    }

    /// A roster of two packages on one host — `team/open` in `reachable`,
    /// `team/locked` in `locked` — with the role's readable set and role list
    /// under the test's control.
    fn mock_two_bucket_roster(
        readable: Result<Vec<String>, ()>,
        roles: Option<RoleInfo>,
    ) -> crate::model::MockQuiltModel {
        let mut model = crate::model::mocks::create();
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
        // Reading the role can expire the stored credentials, so the cache finishes
        // the flush by dropping the clients. See `RoleCache::get`.
        model.expect_clear_remote_client_cache().returning(|_| ());
        model
    }

    async fn roster(m: &impl model::QuiltModel, roles: &RoleCache) -> Vec<MainPagePackage> {
        get_main_page_packages_from_model(
            m,
            roles,
            &crate::telemetry::Telemetry::default(),
            &HashMap::new(),
        )
        .await
        .expect("list")
        .packages
    }

    fn row<'a>(rows: &'a [MainPagePackage], namespace: &str) -> &'a MainPagePackage {
        rows.iter()
            .find(|p| p.namespace == namespace)
            .expect("row present")
    }

    #[tokio::test]
    async fn a_bucket_outside_the_readable_set_resolves_to_a_denial() {
        let m = mock_two_bucket_roster(Ok(vec!["reachable".to_string()]), Some(two_roles()));
        let rows = roster(&m, &RoleCache::default()).await;

        assert_eq!(
            row(&rows, "team/locked").state,
            PackageStateDto::RoleDenied {
                role: Some("ReadOnly".to_string())
            }
        );
        assert_eq!(
            row(&rows, "team/locked").role_switch_host.as_deref(),
            Some("test.quilt.dev"),
            "the user holds a second role, so the switch is not a dead end"
        );
        assert_ne!(
            row(&rows, "team/open").state,
            PackageStateDto::RoleDenied {
                role: Some("ReadOnly".to_string())
            },
            "a readable bucket must not be greyed"
        );
    }

    #[tokio::test]
    async fn a_failed_bucket_query_denies_nothing() {
        // A failed query is not evidence that nothing is readable. Treating it as
        // an empty set would grey every row; the only safe degrade is reactive-only
        // marking, where the per-row status call marks what is really denied.
        let m = mock_two_bucket_roster(Err(()), Some(two_roles()));
        let rows = roster(&m, &RoleCache::default()).await;

        assert_eq!(rows.len(), 2);
        assert!(
            rows.iter()
                .all(|p| !matches!(p.state, PackageStateDto::RoleDenied { .. })),
            "an unanswerable bucket query must not grey the whole roster"
        );
    }

    #[tokio::test]
    async fn a_denial_whose_role_query_failed_is_still_a_denial() {
        let m = mock_two_bucket_roster(Ok(vec!["reachable".to_string()]), None);
        let rows = roster(&m, &RoleCache::default()).await;

        assert_eq!(
            row(&rows, "team/locked").state,
            PackageStateDto::RoleDenied { role: None },
            "the bucket refused; that is known whether or not the role can be named"
        );
        assert_eq!(
            row(&rows, "team/locked").role_switch_host,
            None,
            "no switch affordance: we cannot tell whether another role is held"
        );
    }

    #[tokio::test]
    async fn a_single_role_user_is_offered_no_switch() {
        let solo = RoleInfo {
            current: "ReadOnly".to_string(),
            available: vec!["ReadOnly".to_string()],
        };
        let m = mock_two_bucket_roster(Ok(vec!["reachable".to_string()]), Some(solo));
        let rows = roster(&m, &RoleCache::default()).await;

        assert_eq!(
            row(&rows, "team/locked").state,
            PackageStateDto::RoleDenied {
                role: Some("ReadOnly".to_string())
            }
        );
        assert_eq!(
            row(&rows, "team/locked").role_switch_host,
            None,
            "a switch affordance that leads nowhere is worse than none"
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

    #[allow(
        clippy::unused_async_trait_impl,
        reason = "`readable_buckets` awaits; the other two do not. Rewriting just those would leave one impl split between `async fn` and `fn -> impl Future`, which reads worse than either consistent choice."
    )]
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

    #[tokio::test(start_paused = true)]
    async fn an_unanswerable_host_does_not_hold_the_roster() {
        let m = SilentHost::default();
        let started = tokio::time::Instant::now();
        let rows = roster(&m, &RoleCache::default()).await;

        assert_eq!(rows.len(), 2, "the roster paints from local data");
        assert!(
            rows.iter()
                .all(|p| !matches!(p.state, PackageStateDto::RoleDenied { .. })),
            "a query that never answered is not evidence of denial"
        );
        assert!(
            started.elapsed() <= BUCKET_LIST_BUDGET,
            "the roster waited {:?} on a host that never answers",
            started.elapsed(),
        );
    }

    #[tokio::test]
    async fn the_role_behind_a_denial_is_fetched_once_per_host() {
        // Two denied rows on one host. `RoleCache` is why this is one `/me` and not
        // one per row — the pile-up it exists to prevent.
        let mut model = crate::model::mocks::create();
        let pkgs = vec![
            make_installed_package(("team", "one")),
            make_installed_package(("team", "two")),
        ];
        model
            .expect_get_installed_packages_list()
            .return_once(move || Ok(pkgs));
        model
            .expect_get_installed_package_lineage()
            .returning(|pkg| {
                Ok(quilt::lineage::PackageLineage::from_remote(
                    make_manifest_uri_in_bucket("locked", &pkg.namespace.to_string()),
                    "abcdef".to_string(),
                ))
            });
        model
            .expect_readable_buckets()
            .returning(|_| Ok(vec!["elsewhere".to_string()]));
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counted = calls.clone();
        model.expect_refresh_roles().returning(move |_| {
            counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(two_roles())
        });
        model.expect_clear_remote_client_cache().returning(|_| ());

        let rows = roster(&model, &RoleCache::default()).await;

        assert_eq!(rows.len(), 2);
        assert!(
            rows.iter()
                .all(|p| matches!(p.state, PackageStateDto::RoleDenied { .. }))
        );
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "one role query per host per load, however many rows deny"
        );
    }
}
