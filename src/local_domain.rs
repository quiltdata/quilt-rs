use std::collections::BTreeMap;
use std::marker::Unpin;
use std::path::Path;
use std::path::PathBuf;
use std::str::FromStr;

use crate::flow;
use crate::installed_package::InstalledPackage;
use crate::io::manifest::build_manifest_from_rows_stream;
use crate::io::manifest::RowsStream;
use crate::io::remote::Remote;
use crate::io::remote::RemoteS3;
use crate::io::storage::LocalStorage;
use crate::io::storage::Storage;
use crate::lineage;
use crate::lineage::DomainLineage;
use crate::lineage::Home;
use crate::lineage::PackageLineage;
use crate::manifest::Header;
use crate::manifest::Table;
use crate::paths;
use crate::uri::Host;
use crate::uri::ManifestUri;
use crate::uri::Namespace;
use crate::Res;

/// This is the entrypoint for the lib.
/// All the work you can do with packages is done through calling `LocalDomain` methods.
#[derive(Debug)]
pub struct LocalDomain<S: Storage = LocalStorage, R: Remote = RemoteS3> {
    paths: paths::DomainPaths,
    lineage: lineage::DomainLineageIo,
    storage: S,
    remote: R,
}

impl LocalDomain {
    pub fn get_remote(&self) -> &RemoteS3 {
        &self.remote
    }

    pub fn new(root_dir: impl AsRef<Path>) -> Self {
        let paths = paths::DomainPaths::new(root_dir.as_ref().to_path_buf());
        let lineage = lineage::DomainLineageIo::new(paths.lineage());
        let storage = LocalStorage::new();
        let remote = RemoteS3::new(paths.clone(), storage.clone());
        Self {
            lineage,
            paths,
            remote,
            storage,
        }
    }

    pub async fn get_home(&self) -> Res<Home> {
        let lineage: DomainLineage = self.lineage.read(&self.storage).await?;
        Ok(lineage.home)
    }

    pub async fn set_home(&self, dir: impl AsRef<Path>) -> Res<Home> {
        Ok(self.lineage.set_home(&self.storage, dir).await?.home)
    }

    pub async fn scaffold_paths_for_installing(&self, namespace: &Namespace) -> Res {
        let home = self.get_home().await?;
        self.paths
            .scaffold_for_installing(&self.storage, &home, namespace)
            .await
    }

    pub async fn scaffold_paths_for_caching(&self, bucket: &str) -> Res {
        self.paths.scaffold_for_caching(&self.storage, bucket).await
    }

    pub async fn browse_remote_manifest(&self, uri: &ManifestUri) -> Res<Table> {
        self.scaffold_paths_for_caching(&uri.bucket).await?;
        flow::browse(&self.paths, &self.storage, &self.remote, uri).await
    }

    pub fn create_installed_package(&self, namespace: Namespace) -> Res<InstalledPackage> {
        // TODO: seems like you can use PackageLineage as an argument instead of namespace
        Ok(InstalledPackage {
            lineage: self.lineage.create_package_lineage(namespace.clone()),
            namespace,
            paths: self.paths.clone(),
            remote: self.remote.try_clone()?,
            storage: self.storage.clone(),
        })
    }

    pub async fn install_package(&self, manifest_uri: &ManifestUri) -> Res<InstalledPackage> {
        self.scaffold_paths_for_caching(&manifest_uri.bucket)
            .await?;
        self.scaffold_paths_for_installing(&manifest_uri.namespace)
            .await?;
        let lineage: DomainLineage = self.lineage.read(&self.storage).await?;
        let lineage = flow::install_package(
            lineage,
            &self.paths,
            &self.storage,
            &self.remote,
            manifest_uri,
        )
        .await?;
        self.lineage.write(&self.storage, lineage).await?;

        self.create_installed_package(manifest_uri.namespace.clone())
    }

    pub async fn uninstall_package(&self, namespace: Namespace) -> Res<()> {
        self.scaffold_paths_for_installing(&namespace).await?;

        let lineage = self.lineage.read(&self.storage).await?;
        let lineage =
            flow::uninstall_package(lineage, &self.paths, &self.storage, namespace).await?;
        self.lineage.write(&self.storage, lineage).await?;
        Ok(())
    }

    pub async fn list_installed_packages(&self) -> Res<Vec<InstalledPackage>> {
        let lineage = self.lineage.read(&self.storage).await?;
        let namespaces = lineage.namespaces();
        let mut packages = Vec::with_capacity(namespaces.len());
        for namespace in namespaces {
            packages.push(self.create_installed_package(namespace)?);
        }
        Ok(packages)
    }

    pub async fn get_installed_package(
        &self,
        namespace: &Namespace,
    ) -> Res<Option<InstalledPackage>> {
        let lineage = self.lineage.read(&self.storage).await?;
        if lineage.packages.contains_key(namespace) {
            Ok(Some(self.create_installed_package(namespace.to_owned())?))
        } else {
            Ok(None)
        }
    }

    pub async fn build_manifest(
        &self,
        dest_path: PathBuf,
        stream: impl RowsStream + Unpin,
    ) -> Res<(PathBuf, String)> {
        let dest_dir = dest_path.parent().unwrap_or(&dest_path).to_path_buf();
        build_manifest_from_rows_stream(&self.storage, dest_dir, Header::default(), stream).await
    }

    /// Create a brand new package from scratch that doesn't exist in S3 yet.
    ///
    /// This method:
    /// 1. Creates the package home directory structure
    /// 2. Creates an empty manifest with a placeholder hash
    /// 3. Registers the package in the local lineage
    /// 4. Returns an InstalledPackage that can be committed and pushed
    ///
    /// # Arguments
    /// * `namespace` - The package namespace (e.g., "owner/package-name")
    /// * `bucket` - The S3 bucket where this package will be pushed to
    /// * `catalog` - Optional catalog URL for authentication
    ///
    /// # Example
    /// ```ignore
    /// let domain = LocalDomain::new("/path/to/quilt");
    /// domain.set_home("/path/to/working/dir").await?;
    /// let package = domain.create_new_package(
    ///     ("myteam", "mypackage").into(),
    ///     "my-s3-bucket",
    ///     None
    /// ).await?;
    /// // Now you can add files to the package home and commit
    /// package.commit("Initial commit", None, None, None).await?;
    /// package.push(None).await?;
    /// ```
    pub async fn create_new_package(
        &self,
        namespace: Namespace,
        bucket: impl Into<String>,
        catalog: Option<String>,
    ) -> Res<InstalledPackage> {
        let bucket = bucket.into();

        // Ensure home directory is set
        let home = self.get_home().await?;

        // Create the necessary directory structure for this package
        self.scaffold_paths_for_installing(&namespace).await?;
        self.scaffold_paths_for_caching(&bucket).await?;

        // Build an empty manifest to get a proper hash
        // The manifest file will be created in the installed manifests directory
        let manifest_dir = self.paths.installed_manifests(&namespace);
        let (_manifest_path, initial_hash) = build_manifest_from_rows_stream(
            &self.storage,
            manifest_dir,
            Header::default(),
            tokio_stream::empty(),
        )
        .await?;

        // Convert catalog string to Host if provided
        let catalog_host: Option<Host> = match catalog {
            Some(ref s) => Some(Host::from_str(s).map_err(|_| {
                crate::Error::Host(format!("Invalid catalog host: {}", s))
            })?),
            None => None,
        };

        // Create the package lineage entry
        let package_lineage = PackageLineage {
            commit: None,
            remote: ManifestUri {
                bucket: bucket.clone(),
                namespace: namespace.clone(),
                hash: initial_hash.clone(),
                catalog: catalog_host,
            },
            base_hash: initial_hash.clone(),
            latest_hash: initial_hash.clone(),
            paths: BTreeMap::new(),
        };

        // Read, update, and write the domain lineage
        let mut domain_lineage = match self.lineage.read(&self.storage).await {
            Ok(lineage) => lineage,
            Err(crate::Error::LineageMissing) => DomainLineage::new(home.as_ref() as &PathBuf),
            Err(e) => return Err(e),
        };

        domain_lineage
            .packages
            .insert(namespace.clone(), package_lineage);

        self.lineage
            .write(&self.storage, domain_lineage)
            .await?;

        // Create the package home directory if it doesn't exist
        let package_home = paths::package_home(&home, &namespace);
        self.storage.create_dir_all(&package_home).await?;

        self.create_installed_package(namespace)
    }

    /// Check if a package exists locally (is installed)
    pub async fn package_exists(&self, namespace: &Namespace) -> Res<bool> {
        match self.lineage.read(&self.storage).await {
            Ok(lineage) => Ok(lineage.packages.contains_key(namespace)),
            Err(crate::Error::LineageMissing) => Ok(false),
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use tempfile::TempDir;
    use test_log::test;

    #[test(tokio::test)]
    async fn test_list_installed_packages() -> Res<()> {
        // Create a temporary directory for testing
        let temp_dir = TempDir::new()?;
        let local_domain = super::LocalDomain::new(temp_dir.path());

        // Set home directory
        local_domain.set_home(&temp_dir.path()).await?;

        // Initially there should be no packages
        let packages = local_domain.list_installed_packages().await?;
        assert!(packages.is_empty());

        // Add some packages to the lineage
        let mut lineage = local_domain.lineage.read(&local_domain.storage).await?;

        let namespaces = vec![
            Namespace::from(("foo", "bar")),
            Namespace::from(("test", "package")),
            Namespace::from(("abc", "xyz")),
        ];

        for namespace in &namespaces {
            lineage.packages.insert(
                namespace.clone(),
                crate::lineage::PackageLineage {
                    commit: None,
                    remote: crate::uri::ManifestUri {
                        bucket: "test-bucket".to_string(),
                        namespace: namespace.clone(),
                        hash: "abcdef".to_string(),
                        catalog: None,
                    },
                    base_hash: "abcdef".to_string(),
                    latest_hash: "abcdef".to_string(),
                    paths: std::collections::BTreeMap::new(),
                },
            );
        }

        local_domain
            .lineage
            .write(&local_domain.storage, lineage)
            .await?;

        // Now list_installed_packages should return packages in sorted order
        let packages = local_domain.list_installed_packages().await?;
        assert_eq!(packages.len(), 3);

        // Check that packages are returned in sorted order by namespace
        assert_eq!(packages[0].namespace, Namespace::from(("abc", "xyz")));
        assert_eq!(packages[1].namespace, Namespace::from(("foo", "bar")));
        assert_eq!(packages[2].namespace, Namespace::from(("test", "package")));

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_create_new_package() -> Res<()> {
        // Create a temporary directory for testing
        let temp_dir = TempDir::new()?;
        let local_domain = LocalDomain::new(temp_dir.path());

        // Set home directory
        local_domain.set_home(&temp_dir.path()).await?;

        let namespace = Namespace::from(("myteam", "mypackage"));
        let bucket = "test-bucket";

        // Initially the package should not exist
        assert!(!local_domain.package_exists(&namespace).await?);

        // Create the new package
        let installed_package = local_domain
            .create_new_package(namespace.clone(), bucket, None)
            .await?;

        // Verify the package was created
        assert_eq!(installed_package.namespace, namespace);

        // Verify the package exists now
        assert!(local_domain.package_exists(&namespace).await?);

        // Verify it appears in the list
        let packages = local_domain.list_installed_packages().await?;
        assert_eq!(packages.len(), 1);
        assert_eq!(packages[0].namespace, namespace);

        // Verify the lineage has correct bucket
        let lineage = installed_package.lineage().await?;
        assert_eq!(lineage.remote.bucket, bucket);
        assert_eq!(lineage.remote.namespace, namespace);

        // Verify the package home directory was created
        let package_home = paths::package_home(
            &local_domain.get_home().await?,
            &namespace,
        );
        assert!(package_home.exists());

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_create_new_package_with_catalog() -> Res<()> {
        let temp_dir = TempDir::new()?;
        let local_domain = LocalDomain::new(temp_dir.path());
        local_domain.set_home(&temp_dir.path()).await?;

        let namespace = Namespace::from(("team", "data"));
        let bucket = "my-bucket";
        let catalog = Some("catalog.quiltdata.com".to_string());

        let installed_package = local_domain
            .create_new_package(namespace.clone(), bucket, catalog)
            .await?;

        let lineage = installed_package.lineage().await?;
        assert!(lineage.remote.catalog.is_some());
        assert_eq!(
            lineage.remote.catalog.unwrap().to_string(),
            "catalog.quiltdata.com"
        );

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_package_exists() -> Res<()> {
        let temp_dir = TempDir::new()?;
        let local_domain = LocalDomain::new(temp_dir.path());
        local_domain.set_home(&temp_dir.path()).await?;

        let namespace = Namespace::from(("test", "pkg"));

        // Should not exist initially
        assert!(!local_domain.package_exists(&namespace).await?);

        // Create the package
        local_domain
            .create_new_package(namespace.clone(), "bucket", None)
            .await?;

        // Should exist now
        assert!(local_domain.package_exists(&namespace).await?);

        // A different namespace should not exist
        let other_namespace = Namespace::from(("other", "pkg"));
        assert!(!local_domain.package_exists(&other_namespace).await?);

        Ok(())
    }
}
