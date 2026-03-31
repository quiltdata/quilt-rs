use tracing::debug;
use tracing::info;

use crate::flow;
use crate::io::manifest::resolve_tag;
use crate::io::remote::Remote;
use crate::io::storage::Storage;
use crate::lineage::DomainLineage;
use crate::lineage::PackageLineage;
use crate::paths::copy_cached_to_installed;
use crate::paths::DomainPaths;
use crate::uri::ManifestUri;
use crate::uri::Tag;
use crate::Error;
use crate::Res;

/// Installs the package.
/// It fetches manifest and puts it into `installed_packages`.
/// Also, start tracking this package in lineage.
/// DOES NOT install any paths!
pub async fn install_package(
    lineage: DomainLineage,
    paths: &DomainPaths,
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    manifest_uri: &ManifestUri,
) -> Res<DomainLineage> {
    info!("⏳ Installing package: {}", manifest_uri.display());

    paths
        .scaffold_for_installing(storage, &lineage.home, &manifest_uri.namespace)
        .await?;
    paths
        .scaffold_for_caching(storage, &manifest_uri.bucket)
        .await?;

    // TODO: if compatible (same remote), just return the installed package
    if lineage.packages.contains_key(&manifest_uri.namespace) {
        debug!("❌ Package already installed: {}", manifest_uri.namespace);
        return Err(Error::PackageAlreadyInstalled(
            manifest_uri.namespace.clone(),
        ));
    }

    debug!("⏳ Caching remote manifest");
    flow::cache_remote_manifest(paths, storage, remote, manifest_uri).await?;

    debug!("⏳ Creating installed copy of manifest");
    let installed_manifest_path =
        paths.installed_manifest(&manifest_uri.namespace, &manifest_uri.hash);
    copy_cached_to_installed(paths, storage, manifest_uri).await?;
    debug!(
        "✔️ Manifest installed at: {}",
        installed_manifest_path.display()
    );

    debug!("⏳ Resolving latest hash for this package handle");
    let latest = resolve_tag(
        remote,
        &manifest_uri.origin,
        &manifest_uri.into(),
        Tag::Latest,
    )
    .await?;
    debug!("✔️ Latest hash is {}", latest.hash);

    let mut lineage = lineage;
    lineage.packages.insert(
        manifest_uri.namespace.clone(),
        PackageLineage::from_remote(manifest_uri.clone(), latest.hash),
    );
    info!(
        "✔️ Successfully installed package: {}",
        manifest_uri.display()
    );
    Ok(lineage)
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_log::test;

    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use std::str::FromStr;

    use crate::io::remote::mocks::MockRemote;
    use crate::io::storage::mocks::MockStorage;
    use crate::io::storage::LocalStorage;
    use crate::lineage::Home;
    use crate::uri::S3Uri;

    /// Verify that attempting to install a package that is already installed results in an error.
    /// A package is considered installed if it is present in the lineage.
    #[test(tokio::test)]
    async fn test_if_already_installed() -> Res {
        let (home, _temp_dir) = Home::from_temp_dir()?;
        let namespace = ("foo", "bar");
        let lineage = DomainLineage {
            packages: BTreeMap::from([(namespace.into(), PackageLineage::default())]),
            home,
        };
        let result = install_package(
            lineage,
            &DomainPaths::default(),
            &MockStorage::default(),
            &MockRemote::default(),
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
    #[test(tokio::test)]
    async fn test_installing() -> Res {
        let (lineage, _temp_dir) = DomainLineage::from_temp_dir()?;
        let test_hash = "deadbeef".to_string();

        let manifest_uri = ManifestUri {
            bucket: "a".to_string(),
            hash: test_hash.clone(),
            namespace: ("f", "b").into(),
            origin: None,
        };

        // Load the reference manifest from `./fixtures`
        let test_manifest = r#"{"version": "v0"}"#;
        let remote = MockRemote::default();

        // Simulate the remote storage containing the JSONL manifest
        let remote_uri = S3Uri::from_str(&format!(
            "s3://{}/.quilt/packages/{}",
            manifest_uri.bucket, manifest_uri.hash
        ))?;
        remote
            .put_object(
                &manifest_uri.origin,
                &remote_uri,
                test_manifest.as_bytes().to_vec(),
            )
            .await?;

        // Simulate the remote storage containing the reference to the latest manifest
        let latest_uri = S3Uri::from_str(&format!(
            "s3://{}/.quilt/named_packages/{}/latest",
            manifest_uri.bucket, manifest_uri.namespace
        ))?;
        remote
            .put_object(
                &manifest_uri.origin,
                &latest_uri,
                manifest_uri.hash.as_bytes().to_vec(),
            )
            .await?;

        let storage = MockStorage::default();
        let result = install_package(
            lineage,
            &DomainPaths::default(),
            &storage,
            &remote,
            &manifest_uri,
        )
        .await?;

        let installed_package = result.packages.get(&("f", "b").into()).unwrap();
        let tracked = installed_package.remote().unwrap().clone();

        assert_eq!(installed_package.latest_hash, test_hash);

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
    #[test(tokio::test)]
    async fn test_installing_when_no_permissions() -> Res {
        let manifest_uri = ManifestUri {
            bucket: "a".to_string(),
            hash: "h".to_string(),
            namespace: ("f", "b").into(),
            origin: None,
        };

        // Load the reference manifest from `./fixtures`
        let test_manifest = r#"{"version": "v0"}"#;
        let remote = MockRemote::default();

        // Simulate the remote storage containing the JSONL manifest
        let remote_uri = S3Uri::from_str(&format!(
            "s3://{}/.quilt/packages/{}",
            manifest_uri.bucket, manifest_uri.hash
        ))?;
        remote
            .put_object(
                &manifest_uri.origin,
                &remote_uri,
                test_manifest.as_bytes().to_vec(),
            )
            .await?;

        let (lineage, _temp_dir) = DomainLineage::from_temp_dir()?;

        let storage = LocalStorage::new();
        let result = install_package(
            lineage,
            &DomainPaths::new(PathBuf::from("/")),
            &storage,
            &remote,
            &manifest_uri,
        )
        .await;

        let err = result.unwrap_err();
        if let Error::DirectoryCreate { source, .. } = err {
            // macOS (SIP) returns ReadOnlyFilesystem; Linux returns PermissionDenied
            assert!(
                matches!(
                    source.kind(),
                    std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::ReadOnlyFilesystem
                ),
                "Expected PermissionDenied or ReadOnlyFilesystem, got: {:?}",
                source.kind()
            );
        } else {
            panic!("Expected DirectoryCreate error, got: {err:?}");
        }

        Ok(())
    }
}
