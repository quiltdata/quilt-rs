//!
//! Module that contains various structs and helpers to work with `.quilt/lineage.json`.

use std::collections::BTreeMap;
use std::path::Path;
use std::path::PathBuf;

use serde::Deserialize;
use serde::Serialize;
use tracing::log;

#[cfg(test)]
use tempfile::TempDir;

use crate::io::storage::Storage;
use crate::paths;
use crate::uri::Namespace;
use crate::Error;
use crate::Res;

mod status;
pub use status::Change;
pub use status::ChangeSet;
pub use status::InstalledPackageStatus;
pub use status::UpstreamState;

mod package;
pub use package::CommitState;
pub use package::LineagePaths;
pub use package::PackageLineage;
pub use package::PathState;

mod home;
pub use home::Home;

/// It's essentially just a map of `PackageLineage`.
/// Represents the contents of `.quilt/data.json`
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct DomainLineage {
    #[serde(default = "BTreeMap::new")]
    pub packages: BTreeMap<Namespace, PackageLineage>,
    #[serde(default)]
    pub home: Home,
}

impl DomainLineage {
    pub fn new(home_dir: impl AsRef<Path>) -> Self {
        DomainLineage {
            packages: BTreeMap::new(),
            home: Home::from(home_dir),
        }
    }

    /// Returns a sorted vector of all namespaces in the lineage
    pub fn namespaces(&self) -> Vec<Namespace> {
        let mut namespaces: Vec<Namespace> = self.packages.keys().cloned().collect();
        namespaces.sort();
        namespaces
    }

    #[cfg(test)]
    pub fn from_temp_dir() -> Res<(Self, tempfile::TempDir)> {
        let temp_dir = TempDir::new()?;
        Ok((DomainLineage::new(temp_dir.path()), temp_dir))
    }
}

impl AsRef<PathBuf> for DomainLineage {
    fn as_ref(&self) -> &PathBuf {
        self.home.as_ref()
    }
}

impl TryFrom<Vec<u8>> for DomainLineage {
    type Error = Error;

    fn try_from(input: Vec<u8>) -> Result<Self, Self::Error> {
        let result: Result<Self, serde_json::Error> = serde_json::from_slice(&input);

        match result {
            Ok(lineage) => {
                if lineage.as_ref().as_os_str().is_empty() {
                    return Err(Error::LineageMissingHome);
                }
                Ok(lineage)
            }
            Err(err) => {
                log::error!("Failed to parse `Vec<u8>` for `DomainLineage` in `{input:?}`");
                Err(Error::LineageParse(err))
            }
        }
    }
}

/// Wrapper for reading and writing `DomainLineage`
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DomainLineageIo {
    path: PathBuf,
}

// TODO impl std::io::Write and std::io::Read for DomainLineageIo
impl DomainLineageIo {
    pub fn new(path: PathBuf) -> Self {
        DomainLineageIo { path }
    }

    pub async fn read(&self, storage: &impl Storage) -> Res<DomainLineage> {
        let contents = storage
            .read_file(&self.path)
            .await
            .map_err(|err| match err {
                Error::Io(inner_err) => match inner_err.kind() {
                    std::io::ErrorKind::NotFound => Error::LineageMissing,
                    _ => Error::Io(inner_err),
                },
                other => other,
            })?;

        DomainLineage::try_from(contents)
    }

    /// Read a specific package lineage from the domain lineage
    pub async fn read_package_lineage(
        &self,
        storage: &impl Storage,
        namespace: &Namespace,
    ) -> Res<(PathBuf, PackageLineage)> {
        let domain_lineage = self.read(storage).await?;

        match domain_lineage.packages.get(namespace) {
            Some(package_lineage) => {
                let package_home = paths::package_home(&domain_lineage.home, namespace);
                Ok((package_home, package_lineage.clone()))
            }
            None => Err(Error::PackageNotInstalled(namespace.clone())),
        }
    }

    /// Write a specific package lineage to the domain lineage
    pub async fn write_package_lineage(
        &self,
        storage: &impl Storage,
        namespace: &Namespace,
        package_lineage: PackageLineage,
    ) -> Res<PackageLineage> {
        let mut domain_lineage = self.read(storage).await?;
        domain_lineage
            .packages
            .insert(namespace.clone(), package_lineage.clone());
        self.write(storage, domain_lineage).await?;
        Ok(package_lineage)
    }

    pub async fn set_home(
        &self,
        storage: &impl Storage,
        home: impl AsRef<Path>,
    ) -> Res<DomainLineage> {
        match storage.read_file(&self.path).await {
            Ok(contents) => {
                let mut lineage: DomainLineage = serde_json::from_slice(&contents)?;
                lineage.home = home.into();
                self.write(storage, lineage).await
            }
            Err(Error::Io(e)) => match e.kind() {
                std::io::ErrorKind::NotFound => self.write(storage, DomainLineage::new(home)).await,
                _ => Err(Error::Io(e)),
            },
            Err(e) => Err(e),
        }
    }

    pub async fn write(
        &self,
        storage: &impl Storage,
        lineage: DomainLineage,
    ) -> Res<DomainLineage> {
        let contents = serde_json::to_string_pretty(&lineage)?;
        storage
            .write_file(self.path.clone(), contents.as_bytes())
            .await?;
        Ok(lineage)
    }

    pub fn create_package_lineage(&self, namespace: Namespace) -> PackageLineageIo {
        PackageLineageIo::new(self.clone(), namespace)
    }
}

/// Wrapper for reading and writing `PackageLineage`
/// It re-uses `DomainLineageIo`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageLineageIo {
    domain_lineage: DomainLineageIo,
    namespace: Namespace,
}

impl PackageLineageIo {
    pub fn new(domain_lineage: DomainLineageIo, namespace: Namespace) -> Self {
        PackageLineageIo {
            domain_lineage,
            namespace,
        }
    }

    pub async fn read(&self, storage: &impl Storage) -> Res<(PathBuf, PackageLineage)> {
        self.domain_lineage
            .read_package_lineage(storage, &self.namespace)
            .await
    }

    pub async fn package_home(&self, storage: &impl Storage) -> Res<PathBuf> {
        Ok(self
            .domain_home(storage)
            .await?
            .join(self.namespace.to_string()))
    }

    pub async fn domain_home(&self, storage: &impl Storage) -> Res<Home> {
        let domain_lineage = self.domain_lineage.read(storage).await?;
        Ok(domain_lineage.home)
    }

    pub async fn write(
        &self,
        storage: &impl Storage,
        lineage: PackageLineage,
    ) -> Res<PackageLineage> {
        self.domain_lineage
            .write_package_lineage(storage, &self.namespace, lineage)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use base64::prelude::BASE64_STANDARD;
    use base64::Engine;

    use crate::checksum::sha256_chunked;
    use crate::io::storage::mocks::MockStorage;
    use crate::uri::ManifestUri;

    #[test]
    fn test_syntax_error() {
        assert_eq!(
            DomainLineage::try_from(b"err".to_vec())
                .unwrap_err()
                .to_string(),
            "Failed to parse lineage file: expected value at line 1 column 1".to_string()
        );
    }

    #[test]
    fn test_wrong_key() {
        // NOTE: @fiskus I don't think this is developer friendly
        //       I'd like to remove serde(default), so this test fails
        assert!(DomainLineage::try_from(br#"{"notkey": 123}"#.to_vec()).is_err());
    }

    #[test]
    fn test_wrong_value() {
        assert!(DomainLineage::try_from(br#"{"packages": 123}"#.to_vec())
            .unwrap_err()
            .to_string()
            .starts_with("Failed to parse lineage file: invalid type:"));
    }

    #[test]
    fn test_missing_working_directory() {
        assert_eq!(
            DomainLineage::try_from(br###"{"packages":{}}"###.to_vec())
                .unwrap_err()
                .to_string(),
            "Domain lineage missing Home directory".to_string()
        );
    }

    #[test]
    fn test_with_working_directory() -> Res {
        let lineage =
            DomainLineage::try_from(br###"{"packages":{},"home":"/tmp/working_dir"}"###.to_vec())
                .unwrap();
        assert_eq!(lineage.as_ref(), &PathBuf::from("/tmp/working_dir"));
        Ok(())
    }

    #[test]
    fn test_domain_lineage_from_temp_dir() -> Res {
        let (lineage, temp_dir) = DomainLineage::from_temp_dir()?;
        assert_eq!(lineage.as_ref(), &temp_dir.path().to_path_buf());
        assert!(lineage.packages.is_empty());
        Ok(())
    }

    #[test]
    fn test_namespaces() -> Res {
        let mut lineage = DomainLineage::new("/tmp/home");

        // Empty lineage should return empty vector
        assert!(lineage.namespaces().is_empty());

        // Add some packages
        lineage
            .packages
            .insert(Namespace::from(("foo", "bar")), PackageLineage::default());
        lineage
            .packages
            .insert(Namespace::from(("abc", "xyz")), PackageLineage::default());
        lineage.packages.insert(
            Namespace::from(("test", "package")),
            PackageLineage::default(),
        );

        // Check that namespaces are returned in sorted order
        let namespaces = lineage.namespaces();
        assert_eq!(namespaces.len(), 3);
        assert_eq!(namespaces[0], Namespace::from(("abc", "xyz")));
        assert_eq!(namespaces[1], Namespace::from(("foo", "bar")));
        assert_eq!(namespaces[2], Namespace::from(("test", "package")));

        Ok(())
    }

    #[tokio::test]
    async fn test_domain_lineage_from_file() -> Res {
        let storage = MockStorage::default();
        let file_path = PathBuf::from("foo");
        storage
            .write_file(
                &file_path,
                br###"{"packages":{},"home":"/home/directory"}"###.as_ref(),
            )
            .await?;
        let lineage = DomainLineageIo::new(file_path).read(&storage).await?;
        assert_eq!(lineage, DomainLineage::new("/home/directory"));
        Ok(())
    }

    #[tokio::test]
    async fn test_domain_lineage_from_nothing() -> Res {
        let storage = MockStorage::default();
        let lineage = DomainLineageIo::new(PathBuf::from("does-not-exist"))
            .read(&storage)
            .await
            .unwrap_err();
        assert!(matches!(lineage, Error::LineageMissing));
        Ok(())
    }

    #[tokio::test]
    async fn test_domain_lineage_write() -> Res {
        let storage = MockStorage::default();
        let file_path = PathBuf::from("foo");
        assert!(!storage.exists(&file_path).await);
        let bytes = "0123456789abcdef".as_bytes();
        let working_dir = PathBuf::from("/tmp/working_dir");
        DomainLineageIo::new(file_path.clone())
            .write(
                &storage,
                DomainLineage {
                    home: Home::new(working_dir),
                    packages: BTreeMap::from([(
                        ("foo", "bar").into(),
                        PackageLineage {
                            commit: None,
                            remote: ManifestUri {
                                bucket: "bucket".to_string(),
                                namespace: ("foo", "bar").into(),
                                hash: "abcdef".to_string(),
                                catalog: None,
                            },
                            base_hash: "abcdef".to_string(),
                            latest_hash: "abcdef".to_string(),
                            paths: BTreeMap::from([(
                                PathBuf::from("foo"),
                                PathState {
                                    timestamp: chrono::DateTime::from_timestamp_millis(
                                        1737031820534,
                                    )
                                    .unwrap(),
                                    hash: sha256_chunked(bytes, bytes.len() as u64).await?.into(),
                                },
                            )]),
                        },
                    )]),
                },
            )
            .await?;
        assert!(storage.exists(&file_path).await);
        let file_contents = storage.read_file(&file_path).await?;
        let lineage = DomainLineage::try_from(file_contents)?;

        assert_eq!(lineage.as_ref(), &PathBuf::from("/tmp/working_dir"));

        let multihash_from_lineage = lineage
            .packages
            .get(&(("foo".to_string(), "bar".to_string()).into()))
            .unwrap()
            .paths
            .get(&PathBuf::from("foo"))
            .unwrap()
            .hash;
        let hash_from_lineage = BASE64_STANDARD.encode(multihash_from_lineage.digest());
        assert_eq!(
            hash_from_lineage,
            "Xb1PbjJeWof4zD7zuHc9PI7sLiz/Ykj4gphlaZEt3xA="
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_read_package_lineage() -> Res {
        let storage = MockStorage::default();
        let file_path = PathBuf::from("lineage.json");
        let namespace = Namespace::from(("foo", "bar"));
        let package_lineage = PackageLineage {
            commit: None,
            remote: ManifestUri {
                bucket: "bucket".to_string(),
                namespace: namespace.clone(),
                hash: "abcdef".to_string(),
                catalog: None,
            },
            base_hash: "abcdef".to_string(),
            latest_hash: "abcdef".to_string(),
            paths: BTreeMap::new(),
        };

        // Create a domain lineage with a package
        let lineage = DomainLineage {
            home: Home::from("/home/user/quilt"),
            packages: BTreeMap::from([(namespace.clone(), package_lineage.clone())]),
        };

        // Write it to storage
        let lineage_io = DomainLineageIo::new(file_path.clone());
        lineage_io.write(&storage, lineage).await?;

        // Read the package lineage
        let (package_home, read_package_lineage) = lineage_io
            .read_package_lineage(&storage, &namespace)
            .await?;

        // Verify the results
        assert_eq!(package_home, PathBuf::from("/home/user/quilt/foo/bar"));
        assert_eq!(read_package_lineage, package_lineage);

        // Try reading a non-existent package
        let non_existent = Namespace::from(("does", "notexist"));
        let result = lineage_io
            .read_package_lineage(&storage, &non_existent)
            .await;
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            "The given package is not installed: does/notexist"
        );

        Ok(())
    }

    #[tokio::test]
    async fn test_write_package_lineage() -> Res {
        let storage = MockStorage::default();
        let file_path = PathBuf::from("lineage.json");
        let lineage_io = DomainLineageIo::new(file_path.clone());

        // Create an initial domain lineage with home directory
        let initial_lineage = DomainLineage {
            home: Home::from("/home/user/quilt"),
            packages: BTreeMap::new(),
        };

        // Write the initial lineage
        lineage_io.write(&storage, initial_lineage).await?;

        // Create a package lineage to write
        let namespace = Namespace::from(("foo", "bar"));
        let package_lineage = PackageLineage {
            commit: None,
            remote: ManifestUri {
                bucket: "bucket".to_string(),
                namespace: namespace.clone(),
                hash: "abcdef".to_string(),
                catalog: None,
            },
            base_hash: "abcdef".to_string(),
            latest_hash: "abcdef".to_string(),
            paths: BTreeMap::new(),
        };

        // Write the package lineage
        let written_lineage = lineage_io
            .write_package_lineage(&storage, &namespace, package_lineage.clone())
            .await?;

        // Verify the written lineage matches what we provided
        assert_eq!(written_lineage, package_lineage);

        // Read the domain lineage to verify the package was added
        let domain_lineage = lineage_io.read(&storage).await?;
        assert_eq!(domain_lineage.packages.len(), 1);
        assert!(domain_lineage.packages.contains_key(&namespace));
        assert_eq!(
            domain_lineage.packages.get(&namespace).unwrap(),
            &package_lineage
        );

        // Update the package lineage
        let updated_package_lineage = PackageLineage {
            commit: Some(CommitState {
                timestamp: chrono::Utc::now(),
                hash: "".to_string(),
                prev_hashes: Vec::new(),
            }),
            ..package_lineage.clone()
        };

        // Write the updated package lineage
        lineage_io
            .write_package_lineage(&storage, &namespace, updated_package_lineage.clone())
            .await?;

        // Read the domain lineage again to verify the update
        let updated_domain_lineage = lineage_io.read(&storage).await?;
        assert_eq!(updated_domain_lineage.packages.len(), 1);
        assert!(updated_domain_lineage.packages.contains_key(&namespace));
        assert_eq!(
            updated_domain_lineage.packages.get(&namespace).unwrap(),
            &updated_package_lineage
        );

        Ok(())
    }

    #[tokio::test]
    async fn test_domain_lineage_create_package_lineage() -> Res {
        let namespace = ("foo", "bar");
        let domain_lineage = DomainLineageIo::default();
        let lineage = domain_lineage.create_package_lineage(namespace.into());
        assert_eq!(
            lineage,
            PackageLineageIo {
                namespace: namespace.into(),
                domain_lineage,
            }
        );
        Ok(())
    }
}
