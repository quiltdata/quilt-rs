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
/// `PendingChanges`, `PullConflict` and `RoleDenied` are never constructed by
/// `resolve_state` in this build (see its doc comment) — they exist only for
/// wire-shape stability, hence the blanket `dead_code` allowance.
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
    /// Always `None` in this build: deriving a "last changed" timestamp is Plan
    /// 2's job (`qhq-8mgw.3`), plumbed through the heavy phase. The light phase
    /// has nothing to compute it from.
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
fn resolve_state(
    upstream: UpstreamState,
    has_local_commit: bool,
    has_remote: bool,
) -> PackageStateDto {
    match upstream {
        // `Local` means either no bucket chosen, or a bucket with nothing in it yet.
        // v1 called both "no remote"; v2 has a word for each.
        UpstreamState::Local if has_remote => PackageStateDto::Unpublished,
        UpstreamState::Local => PackageStateDto::NoRemote,
        UpstreamState::UpToDate if has_local_commit => PackageStateDto::PendingCommit,
        UpstreamState::UpToDate => PackageStateDto::Latest,
        UpstreamState::Ahead => PackageStateDto::PendingCommit,
        UpstreamState::Behind => PackageStateDto::Behind,
        UpstreamState::Diverged => PackageStateDto::Diverged,
        UpstreamState::Error => PackageStateDto::Unknown,
    }
}

// `PendingChanges`, `PullConflict` and `RoleDenied` are never produced here. The
// first needs the heavy phase's file walk, the other two come from the watcher's
// paused map and the access pass — all of them Plan 2. The DTO can express them so
// the wire shape does not change when they arrive.

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
    let upstream_state: UpstreamState = lineage.into();

    Ok(MainPagePackage {
        namespace,
        state: resolve_state(upstream_state, has_local_commit, has_remote),
        changed_at: None,
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
            resolve_state(UpstreamState::Local, false, false),
            PackageStateDto::NoRemote
        );
    }

    #[test]
    fn local_with_a_bucket_is_unpublished() {
        // `UpstreamState::Local` covers BOTH "no remote configured" and "remote set
        // but never pushed" — its own doc comment says so. v2 splits them.
        assert_eq!(
            resolve_state(UpstreamState::Local, false, true),
            PackageStateDto::Unpublished
        );
    }

    #[test]
    fn up_to_date_with_nothing_local_is_latest() {
        assert_eq!(
            resolve_state(UpstreamState::UpToDate, false, true),
            PackageStateDto::Latest
        );
    }

    #[test]
    fn up_to_date_with_a_local_revision_is_pending_commit() {
        assert_eq!(
            resolve_state(UpstreamState::UpToDate, true, true),
            PackageStateDto::PendingCommit
        );
    }

    #[test]
    fn ahead_is_pending_commit() {
        assert_eq!(
            resolve_state(UpstreamState::Ahead, false, true),
            PackageStateDto::PendingCommit
        );
    }

    #[test]
    fn behind_carries_no_count() {
        assert_eq!(
            resolve_state(UpstreamState::Behind, false, true),
            PackageStateDto::Behind,
            "a hash inequality is not a distance; no revision count is derivable"
        );
    }

    #[test]
    fn diverged_maps_straight_through() {
        assert_eq!(
            resolve_state(UpstreamState::Diverged, false, true),
            PackageStateDto::Diverged
        );
    }

    #[test]
    fn error_becomes_the_fallback() {
        assert_eq!(
            resolve_state(UpstreamState::Error, false, true),
            PackageStateDto::Unknown
        );
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
