use std::path::PathBuf;

use crate::flow::browse::cache_remote_manifest;
use crate::flow::install_paths::install_paths;
use crate::flow::uninstall_paths::uninstall_paths;
use crate::io::remote::Remote;
use crate::io::storage::Storage;
use crate::lineage::PackageLineage;
use crate::paths::copy_cached_to_installed;
use crate::paths::DomainPaths;
use crate::quilt::manifest_handle::ReadableManifest;
use crate::quilt::uri::Namespace;
use crate::Error;

pub async fn reset_to_latest(
    lineage: PackageLineage,
    manifest: &(impl ReadableManifest + Sync),
    paths: &DomainPaths,
    storage: &(impl Storage + std::marker::Sync),
    remote: &impl Remote,
    working_dir: PathBuf,
    namespace: Namespace,
) -> Result<PackageLineage, Error> {
    let new_latest = lineage.remote.resolve_latest(remote).await?;
    if new_latest == lineage.remote.hash {
        // already at latest
        return Ok(lineage);
    }

    let entries_paths: Vec<PathBuf> = lineage.paths.clone().into_keys().collect();
    let mut lineage =
        uninstall_paths(lineage, working_dir.clone(), storage, &entries_paths).await?;

    lineage.latest_hash = new_latest.clone();
    lineage.remote.hash = new_latest.clone();
    lineage.base_hash = new_latest;

    cache_remote_manifest(paths, storage, remote, &lineage.remote).await?;
    copy_cached_to_installed(
        paths,
        storage,
        &lineage.remote.bucket,
        &namespace,
        &lineage.remote.hash,
    )
    .await?;

    let materialized_manifest = manifest.read(storage).await?;
    let paths_to_install = entries_paths
        .into_iter()
        .filter(|x| materialized_manifest.records.contains_key(x))
        .collect();
    install_paths(
        lineage,
        manifest,
        paths,
        working_dir,
        namespace,
        storage,
        remote,
        &paths_to_install,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::io::remote::mocks::MockRemote;
    use crate::io::s3::S3Uri;
    use crate::io::storage::mocks::MockStorage;
    use crate::lineage::PackageLineage;
    use crate::quilt::mocks;
    use crate::quilt::RemoteManifest;
    use crate::utils::local_uri_json;

    #[tokio::test]
    async fn test_if_already_latest() -> Result<(), Error> {
        let source_lineage = mocks::lineage::with_remote("quilt+s3://b#package=f/a@foo")?;

        let remote = MockRemote::default();
        remote
            .put_object(
                &S3Uri::try_from("s3://b/.quilt/named_packages/f/a/latest")?,
                b"foo".to_vec(),
            )
            .await?;

        let resolved_lineage = reset_to_latest(
            source_lineage.clone(),
            &mocks::manifest::default(),
            &DomainPaths::default(),
            &MockStorage::default(),
            &remote,
            PathBuf::default(),
            Namespace::default(),
        )
        .await?;
        assert_eq!(resolved_lineage, source_lineage);
        Ok(())
    }

    #[tokio::test]
    async fn test_reseting_to_latest() -> Result<(), Error> {
        let source_lineage = mocks::lineage::with_remote("quilt+s3://b#package=f/a@OUTDATED_HASH")?;

        let jsonl = std::fs::read(local_uri_json())?;
        let remote = MockRemote::default();
        remote
            .put_object(
                &S3Uri::try_from("s3://b/.quilt/named_packages/f/a/latest")?,
                b"LATEST_HASH".to_vec(),
            )
            .await?;
        remote
            .put_object(
                &S3Uri::try_from("s3://b/.quilt/packages/LATEST_HASH")?,
                jsonl,
            )
            .await?;

        let resolved_lineage = reset_to_latest(
            source_lineage.clone(),
            &mocks::manifest::default(),
            &DomainPaths::default(),
            &MockStorage::default(),
            &remote,
            PathBuf::default(),
            Namespace::default(),
        )
        .await?;
        assert_eq!(
            resolved_lineage,
            PackageLineage {
                base_hash: "LATEST_HASH".to_string(),
                latest_hash: "LATEST_HASH".to_string(),
                remote: RemoteManifest {
                    hash: "LATEST_HASH".to_string(),
                    ..source_lineage.remote
                },
                ..source_lineage
            }
        );
        Ok(())
    }
}
