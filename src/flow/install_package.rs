use crate::flow::browse::cache_remote_manifest;
use crate::io::manifest::resolve_latest;
use crate::io::remote::Remote;
use crate::io::storage::Storage;
use crate::lineage::DomainLineage;
use crate::lineage::PackageLineage;
use crate::paths;
use crate::uri::ManifestUri;
use crate::Error;

pub async fn install_package(
    lineage: DomainLineage,
    paths: &paths::DomainPaths,
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    manifest_uri: &ManifestUri,
) -> Result<DomainLineage, Error> {
    // bail if already installed
    // TODO: if compatible (same remote), just return the installed package
    if lineage.packages.contains_key(&manifest_uri.namespace) {
        return Err(Error::PackageAlreadyInstalled(
            manifest_uri.namespace.clone(),
        ));
    }

    cache_remote_manifest(paths, storage, remote, &manifest_uri.clone()).await?;

    // Make an "installed" copy of the remote manifest.
    let installed_manifest_path =
        paths.installed_manifest(&manifest_uri.namespace, &manifest_uri.hash);
    storage
        .create_dir_all(&installed_manifest_path.parent().unwrap())
        .await?;
    paths::copy_cached_to_installed(paths, storage, manifest_uri).await?;

    // Create the identity cache dir.
    let objects_dir = paths.objects_dir();
    storage.create_dir_all(&objects_dir).await?;

    // Create the working dir.
    let working_dir = paths.working_dir(&manifest_uri.namespace);
    storage.create_dir_all(&working_dir).await?;

    // Resolve and record latest manifest hash
    let latest = resolve_latest(remote, manifest_uri.into()).await?;
    // Update the lineage (with empty paths).
    let mut lineage = lineage;
    lineage.packages.insert(
        manifest_uri.namespace.clone(),
        PackageLineage::from_remote(manifest_uri.clone(), latest.hash),
    );
    Ok(lineage)
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use crate::mocks;
    use crate::uri::S3Uri;

    #[tokio::test]
    async fn test_if_already_installed() -> Result<(), Error> {
        let namespace = ("foo", "bar");
        let lineage = DomainLineage {
            packages: BTreeMap::from([(namespace.into(), PackageLineage::default())]),
        };
        let result = install_package(
            lineage,
            &paths::DomainPaths::default(),
            &mocks::storage::MockStorage::default(),
            &mocks::remote::MockRemote::default(),
            &ManifestUri {
                namespace: namespace.into(),
                ..ManifestUri::default()
            },
        )
        .await;
        assert_eq!(
            result.unwrap_err().to_string(),
            "The package foo/bar is already installed"
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_installing() -> Result<(), Error> {
        let manifest_uri = ManifestUri {
            bucket: "a".to_string(),
            hash: "c".to_string(),
            namespace: ("f", "b").into(),
        };
        let parquet = std::fs::read(mocks::manifest::parquet())?;
        let remote = mocks::remote::MockRemote::default();
        remote
            .put_object(
                &S3Uri::try_from("s3://a/.quilt/packages/1220c.parquet")?,
                parquet,
            )
            .await?;
        remote
            .put_object(
                &S3Uri::try_from("s3://a/.quilt/named_packages/f/b/latest")?,
                Vec::new(),
            )
            .await?;
        let storage = mocks::storage::MockStorage::default();
        let result = install_package(
            DomainLineage::default(),
            &paths::DomainPaths::default(),
            &storage,
            &remote,
            &manifest_uri,
        )
        .await?;
        assert_eq!(
            result.packages.get(&("f", "b").into()).unwrap().remote,
            manifest_uri
        );
        assert!(
            storage
                .exists(&PathBuf::from(".quilt/installed/f/b/c"))
                .await
        );
        assert!(storage.exists(&PathBuf::from(".quilt/packages/a/c")).await);
        Ok(())
    }
}
