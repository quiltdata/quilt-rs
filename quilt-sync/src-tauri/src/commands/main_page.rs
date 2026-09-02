//! v2's main-page payloads. Nothing here is shared with `package_list.rs`, which
//! is v1's, frozen, and deleted once v2 ships.

use std::collections::HashMap;

use serde::Serialize;

use crate::autopull::Watcher;
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
    PendingChanges { files: usize },
    PendingCommit,
    Diverged,
    PullConflict { files: Vec<String> },
    RoleDenied { role: String },
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

async fn get_main_page_packages_from_model(
    m: &impl model::QuiltModel,
    paused_reasons: &HashMap<String, PausedRow>,
) -> Result<MainPagePackages, Error> {
    let list = m.get_installed_packages_list().await?;
    let mut packages = Vec::new();
    for installed_package in list {
        match load_main_page_package(m, &installed_package, paused_reasons).await {
            Ok(item) => packages.push(item),
            Err(err) => {
                tracing::warn!(
                    "Failed to load package {}: {err}",
                    installed_package.namespace,
                );
            }
        }
    }
    Ok(MainPagePackages { packages })
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
    installed_package: &quilt::InstalledPackage,
    paused_reasons: &HashMap<String, PausedRow>,
) -> Result<MainPagePackage, Error> {
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
    let state = if misconfigured_remote(&lineage) {
        PackageStateDto::Unknown
    } else {
        // `None`, not `Some(0)`: this phase has not looked at the working tree.
        // The heavy phase (`refresh_main_page_package`) measures it.
        resolve_state(lineage.into(), has_local_commit, has_remote, None)
    };

    Ok(MainPagePackage {
        namespace,
        state,
        changed_at,
        bucket,
        provisional: true,
        paused_reason,
        paused_kind,
    })
}

/// Walk the installed packages, read each one's cached lineage, resolve, and
/// attach the paused reason from the watcher's authoritative map. A failed
/// package is warned-and-skipped, per `package_list.rs:198-206` — one bad
/// lineage must not blank the whole list.
#[tauri::command]
pub async fn get_main_page_packages(
    m: tauri::State<'_, model::Model>,
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

    get_main_page_packages_from_model(&*m, &paused_reasons)
        .await
        .map_err(|e| e.to_frontend_string())
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
        assert!(!misconfigured_remote(&quilt::lineage::PackageLineage::default()));
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

    /// Drives `get_main_page_packages_from_model`'s real loop over a mock model —
    /// the paused map, the per-package lineage read, and the warn-and-skip on a
    /// failed lineage — and pins the result's serialized JSON. That JSON is the
    /// contract the UI's `MainPagePackageData` must deserialize (`kit/package_state.rs`),
    /// so this one test buys both the loop's integration coverage and the wire-shape
    /// pin the final review asked for, rather than two separate tests.
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

        let mut paused_reasons = HashMap::new();
        paused_reasons.insert(
            "team/latest".to_string(),
            PausedRow {
                kind: "workflow".to_string(),
                message: "blocked by workflow rule".to_string(),
            },
        );

        let result = get_main_page_packages_from_model(&model, &paused_reasons)
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
                    "pausedReason": "blocked by workflow rule",
                    "pausedKind": "workflow",
                }]
            }),
            "this shape is what the UI's MainPagePackageData must deserialize"
        );
    }
}
