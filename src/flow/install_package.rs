use crate::flow;
use crate::io::manifest::resolve_latest;
use crate::io::remote::Remote;
use crate::io::storage::Storage;
use crate::lineage::DomainLineage;
use crate::lineage::PackageLineage;
use crate::paths;
use crate::uri::ManifestUri;
use crate::Error;
use crate::Res;

/// Installs the package.
/// It fetches manifest and puts it into `installed_packages`.
/// Also, start tracking this package in lineage.
/// DOES NOT install any paths!
pub async fn install_package(
    lineage: DomainLineage,
    paths: &paths::DomainPaths,
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    manifest_uri: &ManifestUri,
) -> Res<DomainLineage> {
    // bail if already installed
    // TODO: if compatible (same remote), just return the installed package
    if lineage.packages.contains_key(&manifest_uri.namespace) {
        return Err(Error::PackageAlreadyInstalled(
            manifest_uri.namespace.clone(),
        ));
    }

    flow::cache_remote_manifest(paths, storage, remote, &manifest_uri.clone()).await?;

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
    let latest = resolve_latest(remote, manifest_uri.catalog.clone(), manifest_uri.into()).await?;
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
    use std::str::FromStr;

    use crate::io::storage::LocalStorage;
    use crate::mocks;
    use crate::uri::S3Uri;

    /// Verify that attempting to install a package that is already installed results in an error.
    /// A package is considered installed if it is present in the lineage.
    #[tokio::test]
    async fn test_if_already_installed() -> Res {
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

    /// Verify that a manifest is fetched from the remote storage and installed locally.
    /// This test focuses on the manifest itself, not on the package files.
    /// Package files are installed separately using `install_paths`.
    #[tokio::test]
    async fn test_installing() -> Res {
        let manifest_uri = ManifestUri {
            bucket: "a".to_string(),
            hash: "abcdef1234".to_string(),
            namespace: ("f", "b").into(),
            catalog: None,
        };

        // Load the reference manifest from `./fixtures`
        let parquet = std::fs::read(mocks::manifest::parquet())?;
        let remote = mocks::remote::MockRemote::default();

        // Simulate the remote storage containing the Parquet manifest
        let remote_uri = S3Uri::from_str(&format!(
            "s3://{}/.quilt/packages/1220{}.parquet",
            manifest_uri.bucket, manifest_uri.hash
        ))?;
        remote
            .put_object(manifest_uri.catalog.clone(), &remote_uri, parquet)
            .await?;

        // Simulate the remote storage containing the reference to the latest manifest
        let latest_uri = S3Uri::from_str(&format!(
            "s3://{}/.quilt/named_packages/{}/latest",
            manifest_uri.bucket, manifest_uri.namespace
        ))?;
        remote
            .put_object(
                manifest_uri.catalog.clone(),
                &latest_uri,
                manifest_uri.hash.as_bytes().to_vec(),
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

        let installed_package = result.packages.get(&("f", "b").into()).unwrap();
        let tracked = installed_package.remote.clone();

        assert_eq!(installed_package.latest_hash, "abcdef1234".to_string());

        // Verify that the lineage records the installed package
        assert_eq!(tracked, manifest_uri);

        // Verify that the manifest is stored locally in the immutable manifest directory
        let installed_manifest_path = PathBuf::from(format!(
            ".quilt/installed/{}/{}",
            tracked.namespace, tracked.hash
        ));
        assert!(storage.exists(&installed_manifest_path).await);

        // Verify that the manifest is cached locally
        let cached_manifest_path = PathBuf::from(format!(
            ".quilt/packages/{}/{}",
            tracked.bucket, tracked.hash
        ));
        assert!(storage.exists(&cached_manifest_path).await);
        Ok(())
    }

    // Verify it throws correct error when no permissions
    // Permissions denied, because we try to create a file in the OS root directory
    #[tokio::test]
    async fn test_installing_when_no_permissions() -> Res {
        let manifest_uri = ManifestUri {
            bucket: "a".to_string(),
            hash: "h".to_string(),
            namespace: ("f", "b").into(),
            catalog: None,
        };

        // Load the reference manifest from `./fixtures`
        let parquet = std::fs::read(mocks::manifest::parquet())?;
        let remote = mocks::remote::MockRemote::default();

        // Simulate the remote storage containing the Parquet manifest
        let remote_uri = S3Uri::from_str(&format!(
            "s3://{}/.quilt/packages/1220{}.parquet",
            manifest_uri.bucket, manifest_uri.hash
        ))?;
        remote
            .put_object(manifest_uri.catalog.clone(), &remote_uri, parquet)
            .await?;

        let storage = LocalStorage::new();
        let result = install_package(
            DomainLineage::default(),
            &paths::DomainPaths::new(PathBuf::from("/")),
            &storage,
            &remote,
            &manifest_uri,
        )
        .await;

        let err = result.unwrap_err();
        if let Error::Io(orig_err) = err {
            assert_eq!(orig_err.kind(), std::io::ErrorKind::PermissionDenied);
        } else {
            panic!("Expected IO error, got: {:?}", err);
        }

        Ok(())
    }
}
