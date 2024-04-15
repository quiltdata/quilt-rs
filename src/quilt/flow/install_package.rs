use crate::paths;
use crate::quilt::flow::browse::cache_remote_manifest;
use crate::quilt::lineage::DomainLineage;
use crate::quilt::lineage::PackageLineage;
use crate::quilt::manifest_handle::RemoteManifest;
use crate::quilt::remote::Remote;
use crate::quilt::Storage;
use crate::Error;

pub async fn install_package(
    lineage: DomainLineage,
    paths: &paths::DomainPaths,
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    remote_manifest: &RemoteManifest,
) -> Result<DomainLineage, Error> {
    // bail if already installed
    // TODO: if compatible (same remote), just return the installed package
    if lineage.packages.contains_key(&remote_manifest.namespace) {
        return Err(Error::PackageAlreadyInstalled(
            remote_manifest.namespace.clone(),
        ));
    }

    cache_remote_manifest(paths, storage, remote, remote_manifest).await?;

    // Make an "installed" copy of the remote manifest.
    let installed_manifest_path =
        paths.installed_manifest(&remote_manifest.namespace, &remote_manifest.hash);
    storage
        .create_dir_all(&installed_manifest_path.parent().unwrap())
        .await?;
    paths::copy_cached_to_installed(
        paths,
        storage,
        &remote_manifest.bucket,
        &remote_manifest.namespace,
        &remote_manifest.hash,
    )
    .await?;

    // Create the identity cache dir.
    let objects_dir = paths.objects_dir();
    storage.create_dir_all(&objects_dir).await?;

    // Create the working dir.
    let working_dir = paths.working_dir(&remote_manifest.namespace);
    storage.create_dir_all(&working_dir).await?;

    // Resolve and record latest manifest hash
    let latest_hash = remote_manifest.resolve_latest(remote).await?;
    // Update the lineage (with empty paths).
    let mut lineage = lineage;
    lineage.packages.insert(
        remote_manifest.namespace.clone(),
        PackageLineage::from_remote(remote_manifest.to_owned(), latest_hash),
    );
    Ok(lineage)
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use crate::quilt::remote::mock_remote::MockRemote;
    use crate::quilt::storage::mock_storage::MockStorage;
    use crate::quilt::storage::s3::S3Uri;

    #[tokio::test]
    async fn test_if_already_installed() -> Result<(), Error> {
        let lineage = DomainLineage {
            packages: BTreeMap::from([("foo".to_string(), PackageLineage::default())]),
        };
        let result = install_package(
            lineage,
            &paths::DomainPaths::default(),
            &MockStorage::default(),
            &MockRemote::default(),
            &RemoteManifest {
                namespace: "foo".to_string(),
                ..RemoteManifest::default()
            },
        )
        .await;
        assert_eq!(
            result.unwrap_err().to_string(),
            "The package foo is already installed"
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_installing() -> Result<(), Error> {
        let remote_manifest = RemoteManifest {
            bucket: "a".to_string(),
            hash: "c".to_string(),
            namespace: "b".to_string(),
        };
        let remote = MockRemote::default();
        remote
            .put_object(
                &S3Uri::try_from("s3://a/.quilt/packages/1220c.parquet")?,
                Vec::new(),
            )
            .await?;
        remote
            .put_object(
                &S3Uri::try_from("s3://a/.quilt/named_packages/b/latest")?,
                Vec::new(),
            )
            .await?;
        let storage = MockStorage::default();
        let result = install_package(
            DomainLineage::default(),
            &paths::DomainPaths::default(),
            &storage,
            &remote,
            &remote_manifest,
        )
        .await?;
        assert_eq!(result.packages.get("b").unwrap().remote, remote_manifest);
        assert!(storage.exists(&PathBuf::from(".quilt/installed/b/c")).await);
        assert!(storage.exists(&PathBuf::from(".quilt/packages/a/c")).await);
        Ok(())
    }
}
