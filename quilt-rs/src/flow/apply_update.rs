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
    *manifest = flow::cache_remote_manifest(paths, storage, remote, &latest).await?;
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

    use crate::io::remote::mocks::MockRemote;
    use crate::io::storage::mocks::MockStorage;
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
}
