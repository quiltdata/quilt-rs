//!
//! Module that contains various structs and helpers to work with `.quilt/lineage.json`.

use std::collections::BTreeMap;
use std::path::PathBuf;

#[cfg(test)]
use tempfile::TempDir;

use serde::Deserialize;
use serde::Serialize;
use tracing::log;

use crate::io::storage::Storage;
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

/// Wrapper for working directory path with proper serialization/deserialization
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct DomainWorkingDir {
    inner: Option<PathBuf>,
}

impl DomainWorkingDir {
    pub fn new(path: PathBuf) -> Self {
        DomainWorkingDir { inner: Some(path) }
    }

    pub fn get(&self) -> Res<&PathBuf> {
        self.inner
            .as_ref()
            .ok_or(Error::DomainLineageMissingWorkingDirectory)
    }

    pub fn is_some(&self) -> bool {
        self.inner.is_some()
    }

    pub fn join(&self, path: impl AsRef<std::path::Path>) -> Result<PathBuf, Error> {
        match &self.inner {
            Some(dir) => Ok(dir.join(path)),
            None => Err(Error::DomainLineageMissingWorkingDirectory),
        }
    }

    #[cfg(test)]
    pub fn from_temp_dir() -> Res<(Self, TempDir)> {
        let temp_dir = TempDir::new()?;
        Ok((
            DomainWorkingDir {
                inner: Some(temp_dir.path().to_path_buf()),
            },
            temp_dir,
        ))
    }
}

impl Serialize for DomainWorkingDir {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match &self.inner {
            Some(path) => serializer.serialize_str(&path.to_string_lossy()),
            None => serializer.serialize_none(),
        }
    }
}

impl<'de> Deserialize<'de> for DomainWorkingDir {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let path_str = String::deserialize(deserializer)?;
        Ok(DomainWorkingDir {
            inner: Some(PathBuf::from(path_str)),
        })
    }
}

/// It's essentially just a map of `PackageLineage`.
/// Represents the contents of `.quilt/data.json`
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct DomainLineage {
    #[serde(default = "BTreeMap::new")]
    pub packages: BTreeMap<Namespace, PackageLineage>,
    #[serde(default)]
    pub working_directory: DomainWorkingDir,
}

impl TryFrom<Vec<u8>> for DomainLineage {
    type Error = Error;

    fn try_from(input: Vec<u8>) -> Result<Self, Self::Error> {
        let result: Result<Self, serde_json::Error> = serde_json::from_slice(&input);

        match result {
            Ok(lineage) => {
                let working_dir = lineage.working_directory.get()?;
                if working_dir.as_os_str().is_empty() {
                    return Err(Error::DomainLineageMissingWorkingDirectory);
                }
                Ok(lineage)
            }
            Err(err) => {
                log::error!(
                    "Failed to parse `Vec<u8>` for `DomainLineage` in `{:?}`",
                    input
                );
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
            .or_else(|err| match err {
                Error::Io(inner_err) => {
                    if inner_err.kind() == std::io::ErrorKind::NotFound {
                        Ok("{}".into())
                    } else {
                        Err(Error::Io(inner_err))
                    }
                }
                other => Err(other),
            })?;

        DomainLineage::try_from(contents)
    }

    pub async fn set_working_directory(
        &self,
        storage: &impl Storage,
        working_directory: DomainWorkingDir,
    ) -> Res<DomainLineage> {
        match storage.read_file(&self.path).await {
            Ok(contents) => {
                let mut lineage: DomainLineage = serde_json::from_slice(&contents)?;
                lineage.working_directory = working_directory.clone();
                self.write(storage, lineage.clone()).await
            }
            Err(Error::Io(e)) => match e.kind() {
                std::io::ErrorKind::NotFound => {
                    let lineage = DomainLineage {
                        packages: BTreeMap::new(),
                        working_directory,
                    };
                    self.write(storage, lineage.clone()).await
                }
                _ => return Err(Error::Io(e)),
            },
            Err(e) => return Err(e),
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
        let domain_lineage = self.domain_lineage.read(storage).await?;
        let namespace = domain_lineage.packages.get(&self.namespace);

        match namespace {
            Some(ns) => {
                let package_working_dir = domain_lineage
                    .working_directory
                    .join(self.namespace.to_string())?;
                Ok((package_working_dir, ns.clone()))
            }
            None => Err(Error::PackageNotInstalled(self.namespace.clone())),
        }
    }

    pub async fn working_directory(&self, storage: &impl Storage) -> Res<PathBuf> {
        self.domain_working_directory(storage)
            .await?
            .join(self.namespace.to_string())
    }

    pub async fn domain_working_directory(&self, storage: &impl Storage) -> Res<DomainWorkingDir> {
        let domain_lineage = self.domain_lineage.read(storage).await?;
        Ok(domain_lineage.working_directory)
    }

    pub async fn write(
        &self,
        storage: &impl Storage,
        lineage: PackageLineage,
    ) -> Res<PackageLineage> {
        let mut domain_lineage = self.domain_lineage.read(storage).await?;
        domain_lineage
            .packages
            .insert(self.namespace.clone(), lineage.clone());
        self.domain_lineage.write(storage, domain_lineage).await?;
        Ok(lineage)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use base64::prelude::BASE64_STANDARD;
    use base64::Engine;

    use crate::checksum::calculate_sha256_chunked_checksum;
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
            "Domain lineage missing working directory".to_string()
        );
    }

    #[test]
    fn test_with_working_directory() -> Res {
        let lineage = DomainLineage::try_from(
            br###"{"packages":{},"working_directory":"/tmp/working_dir"}"###.to_vec(),
        )
        .unwrap();
        assert_eq!(
            lineage.working_directory.get()?,
            &PathBuf::from("/tmp/working_dir")
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_domain_lineage_from_file() -> Res {
        let storage = MockStorage::default();
        let file_path = PathBuf::from("foo");
        storage
            .write_file(
                &file_path,
                br###"{"packages":{},"working_directory":"/tmp/working_dir"}"###.as_ref(),
            )
            .await?;
        let lineage = DomainLineageIo::new(file_path).read(&storage).await?;
        assert_eq!(
            lineage,
            DomainLineage {
                packages: BTreeMap::new(),
                working_directory: DomainWorkingDir::new(PathBuf::from("/tmp/working_dir")),
            }
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_domain_lineage_from_nothing() -> Res {
        let storage = MockStorage::default();
        let lineage = DomainLineageIo::new(PathBuf::from("does-not-exist"))
            .read(&storage)
            .await
            .unwrap_err();
        assert!(matches!(
            lineage,
            Error::DomainLineageMissingWorkingDirectory
        ));
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
                    working_directory: DomainWorkingDir::new(working_dir),
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
                                    hash: calculate_sha256_chunked_checksum(
                                        bytes,
                                        bytes.len() as u64,
                                    )
                                    .await
                                    .unwrap(),
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

        assert_eq!(
            lineage.working_directory.get()?,
            &PathBuf::from("/tmp/working_dir")
        );

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
