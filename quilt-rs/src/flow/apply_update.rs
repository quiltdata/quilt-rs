use std::path::PathBuf;

use tracing::debug;

use crate::Res;
use crate::flow;
use crate::io::remote::Remote;
use crate::io::storage::Storage;
use crate::lineage::PackageLineage;
use crate::manifest::Manifest;
use crate::paths::DomainPaths;
use crate::paths::copy_cached_to_installed;
use quilt_uri::ManifestUri;
use quilt_uri::Namespace;

/// Apply a set of `latest`-manifest path updates to the working tree, object
/// store, lineage, and installed-manifest base — the mechanic shared by
/// gentle [`pull`](super::pull) and [`reset_to_latest`](super::reset_to_latest).
///
/// `touched` is the caller's touch-set over **tracked** paths; each keeps its
/// own touch-set (reset: every differing path; gentle pull: remote-changed
/// minus locally-changed). Paths in `touched` absent from `latest` were
/// remote-removed and stay uninstalled.
///
/// # Errors
/// Propagates uninstall / caching / install failures. The reconcile is not a
/// transaction across I/O; callers treat `pull`/`reset` as retryable.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn apply_latest_update(
    lineage: PackageLineage,
    manifest: &mut Manifest,
    paths: &DomainPaths,
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    working_dir: PathBuf,
    namespace: Namespace,
    latest: ManifestUri,
    touched: &[PathBuf],
) -> Res<PackageLineage> {
    // Uninstall only the touched paths we currently track (a remote-added
    // path is not tracked; a remote-removed path is).
    let to_uninstall: Vec<PathBuf> = touched
        .iter()
        .filter(|p| lineage.paths.contains_key(*p))
        .cloned()
        .collect();
    debug!("⏳ Uninstalling {} touched paths", to_uninstall.len());
    let mut lineage =
        flow::uninstall_paths(lineage, working_dir.clone(), storage, &to_uninstall).await?;

    debug!("⏳ Advancing lineage to latest {}", latest.hash);
    lineage.remote_mut()?.hash.clone_from(&latest.hash);
    lineage.base_hash.clone_from(&latest.hash);
    lineage.latest_hash.clone_from(&latest.hash);

    debug!("⏳ Caching + installing latest manifest as the new base");
    // Cache the remote manifest for its side effect only; the parse result is
    // discarded because `*manifest` is (re)loaded from the installed copy just
    // below from a byte-identical file.
    flow::cache_remote_manifest(paths, storage, remote, &latest).await?;
    copy_cached_to_installed(
        paths,
        storage,
        &ManifestUri {
            namespace: namespace.clone(),
            ..latest.clone()
        },
    )
    .await?;
    *manifest =
        Manifest::from_path(storage, &paths.installed_manifest(&namespace, &latest.hash)).await?;
    lineage.remote_uri = Some(latest);

    // Prune lineage paths that have no row in the new base manifest. This only
    // catches trivially-resolved both-removed paths (locally deleted + absent
    // from `latest`): they are filtered out of the touch-set, so they never go
    // through `uninstall_paths`, yet the persisted lineage must not track a
    // path with no manifest row — `create_status` hard-errors on that for
    // remote-backed packages. Every other case is already handled: a
    // locally-removed path the remote still has keeps its row; a
    // remote-removed + locally-modified path is classified `Blocked` and never
    // reaches apply; a remote-removed untouched path is in the touch-set and
    // uninstalled normally. For reset (touch-set = every path) this is a no-op.
    lineage
        .paths
        .retain(|path, _| manifest.contains_record(path));

    let to_install: Vec<&PathBuf> = touched
        .iter()
        .filter(|p| manifest.contains_record(p))
        .collect();
    debug!(
        "⏳ Reinstalling {} touched paths present in latest",
        to_install.len()
    );
    let lineage = flow::install_paths(
        lineage,
        manifest,
        paths,
        working_dir,
        namespace,
        storage,
        remote,
        &to_install,
    )
    .await?;

    Ok(lineage)
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_log::test;

    use std::collections::BTreeMap;

    use crate::io::remote::mocks::MockRemote;
    use crate::io::storage::mocks::MockStorage;
    use crate::lineage::PathState;
    use quilt_uri::S3Uri;

    // An empty touch-set advances the hashes (`base_hash`, `latest_hash`, and
    // `remote.hash`) to `latest` and reloads the manifest cache without touching
    // any installed paths. Uninstall/reinstall mechanics are covered by the
    // callers' suites (reset_to_latest, pull, quilt-cli).
    #[test(tokio::test)]
    async fn empty_touch_set_advances_hashes() -> Res {
        let manifest_uri = ManifestUri {
            bucket: "b".to_string(),
            namespace: ("f", "a").into(),
            hash: "OLD".to_string(),
            origin: None,
        };
        let paths = DomainPaths::default();
        let storage = MockStorage::default();
        paths
            .scaffold_for_caching(&storage, &manifest_uri.bucket)
            .await?;

        let lineage = PackageLineage {
            remote_uri: Some(manifest_uri.clone()),
            base_hash: "OLD".to_string(),
            latest_hash: "NEW".to_string(),
            ..PackageLineage::default()
        };

        let new_hash = "deadbeef";
        let remote = MockRemote::default();
        remote
            .put_object(
                None,
                &S3Uri::try_from(format!("s3://b/.quilt/packages/{new_hash}").as_str())?,
                r#"{"version": "v0"}"#.as_bytes().to_vec(),
            )
            .await?;

        let latest = ManifestUri {
            hash: new_hash.to_string(),
            ..manifest_uri
        };
        let mut manifest = Manifest::default();
        let result = apply_latest_update(
            lineage,
            &mut manifest,
            &paths,
            &storage,
            &remote,
            PathBuf::default(),
            Namespace::default(),
            latest,
            &[],
        )
        .await?;

        assert_eq!(result.base_hash, new_hash);
        assert_eq!(result.latest_hash, new_hash);
        assert_eq!(result.remote()?.hash, new_hash);
        Ok(())
    }

    // A path tracked in lineage but absent from the new `latest` manifest — a
    // trivially-resolved both-removed path (locally deleted + gone from remote,
    // so filtered out of the touch-set and never uninstalled) — must be pruned
    // from `lineage.paths`. Otherwise the persisted lineage tracks a path with
    // no manifest row and `create_status` hard-errors for remote-backed
    // packages.
    #[test(tokio::test)]
    async fn prunes_lineage_path_absent_from_latest_manifest() -> Res {
        let manifest_uri = ManifestUri {
            bucket: "b".to_string(),
            namespace: ("f", "a").into(),
            hash: "OLD".to_string(),
            origin: None,
        };
        let paths = DomainPaths::default();
        let storage = MockStorage::default();
        paths
            .scaffold_for_caching(&storage, &manifest_uri.bucket)
            .await?;

        let stale = PathBuf::from("both-removed.txt");
        let lineage = PackageLineage {
            remote_uri: Some(manifest_uri.clone()),
            base_hash: "OLD".to_string(),
            latest_hash: "NEW".to_string(),
            paths: BTreeMap::from([(stale.clone(), PathState::default())]),
            ..PackageLineage::default()
        };

        let new_hash = "deadbeef";
        let remote = MockRemote::default();
        remote
            .put_object(
                None,
                &S3Uri::try_from(format!("s3://b/.quilt/packages/{new_hash}").as_str())?,
                // `latest` manifest has no rows → no record for `stale`.
                r#"{"version": "v0"}"#.as_bytes().to_vec(),
            )
            .await?;

        let latest = ManifestUri {
            hash: new_hash.to_string(),
            ..manifest_uri
        };
        let mut manifest = Manifest::default();
        let result = apply_latest_update(
            lineage,
            &mut manifest,
            &paths,
            &storage,
            &remote,
            PathBuf::default(),
            Namespace::default(),
            latest,
            // Empty touch-set: `stale` is NOT uninstalled the normal way.
            &[],
        )
        .await?;

        assert!(
            !result.paths.contains_key(&stale),
            "stale both-removed path must be pruned from lineage"
        );
        Ok(())
    }
}
