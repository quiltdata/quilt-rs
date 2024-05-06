use std::path::PathBuf;

use crate::flow::browse::cache_remote_manifest;
use crate::flow::install_paths::install_paths;
use crate::flow::uninstall_paths::uninstall_paths;
use crate::io::manifest::resolve_latest;
use crate::io::remote::Remote;
use crate::io::storage::Storage;
use crate::lineage::PackageLineage;
use crate::manifest::Table;
use crate::paths::copy_cached_to_installed;
use crate::paths::DomainPaths;
use crate::uri::Namespace;
use crate::Error;

pub async fn reset_to_latest(
    lineage: PackageLineage,
    manifest: &mut Table,
    paths: &DomainPaths,
    storage: &(impl Storage + std::marker::Sync),
    remote: &impl Remote,
    working_dir: PathBuf,
    namespace: Namespace,
) -> Result<PackageLineage, Error> {
    let latest_top_hash = resolve_latest(remote, lineage.remote.clone().into()).await?;
    if latest_top_hash == lineage.remote.hash {
        // already at latest
        return Ok(lineage);
    }

    let installed_paths: Vec<PathBuf> = lineage.paths.clone().into_keys().collect();
    let mut lineage =
        uninstall_paths(lineage, working_dir.clone(), storage, &installed_paths).await?;

    // TODO: Should be a method of lineage
    lineage.latest_hash = latest_top_hash.clone();
    lineage.remote.hash = latest_top_hash.clone();
    lineage.base_hash = latest_top_hash.clone();

    cache_remote_manifest(paths, storage, remote, &lineage.remote.clone().into()).await?;
    copy_cached_to_installed(
        paths,
        storage,
        &lineage.remote.bucket,
        &namespace,
        &latest_top_hash,
    )
    .await?;

    let mut paths_to_install = Vec::new();
    for x in installed_paths {
        if manifest.contains_record(&x).await {
            paths_to_install.push(x)
        }
    }
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

    use crate::lineage::PackageLineage;
    use crate::mocks;
    use crate::uri::ManifestUri;
    use crate::uri::S3Uri;

    #[tokio::test]
    async fn test_if_already_latest() -> Result<(), Error> {
        let source_lineage = mocks::lineage::with_remote(ManifestUri {
            bucket: "b".to_string(),
            namespace: ("f", "a").into(),
            hash: "foo".to_string(),
        });

        let remote = mocks::remote::MockRemote::default();
        remote
            .put_object(
                &S3Uri::try_from("s3://b/.quilt/named_packages/f/a/latest")?,
                b"foo".to_vec(),
            )
            .await?;

        let resolved_lineage = reset_to_latest(
            source_lineage.clone(),
            &mut Table::default(),
            &DomainPaths::default(),
            &mocks::storage::MockStorage::default(),
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
        let source_lineage = mocks::lineage::with_remote(ManifestUri {
            bucket: "b".to_string(),
            namespace: ("f", "a").into(),
            hash: "OUTDATED_HASH".to_string(),
        });

        let jsonl = std::fs::read(mocks::manifest::jsonl())?;
        let remote = mocks::remote::MockRemote::default();
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
            &mut Table::default(),
            &DomainPaths::default(),
            &mocks::storage::MockStorage::default(),
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
                remote: ManifestUri {
                    hash: "LATEST_HASH".to_string(),
                    ..source_lineage.remote
                },
                ..source_lineage
            }
        );
        Ok(())
    }
}
