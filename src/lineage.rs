//!
//! Module that contains various structs and helpers to work with `.quilt/lineage.json`.

use std::collections::BTreeMap;
use std::path::PathBuf;

use multihash::Multihash;
use serde::de::Error as DeserializeError;
use serde::Deserialize;
use serde::Deserializer;
use serde::Serialize;
use serde::Serializer;

#[cfg(test)]
pub mod mocks;

use crate::io::storage::Storage;
use crate::manifest::Row;
use crate::uri::ManifestUri;
use crate::uri::Namespace;
use crate::Error;
use crate::Res;

/// Describes modified states of a file
#[derive(Debug, PartialEq)]
pub enum Change {
    // TODO: Use Row
    Modified(PackageFileFingerprint), // modified to what
    // TODO: Use Row
    Added(PackageFileFingerprint), // added what
    Removed(Row),                  // removed what
}

/// Map of all changed files
pub type ChangeSet = BTreeMap<PathBuf, Change>;

/// State of the local package relative to the remote package
#[derive(Debug, PartialEq, Eq, Default, Serialize)]
pub enum UpstreamState {
    #[default]
    UpToDate,
    Behind,
    Ahead,
    Diverged,
}

impl From<PackageLineage> for UpstreamState {
    fn from(lineage: PackageLineage) -> Self {
        let behind = lineage.base_hash != lineage.latest_hash;
        let ahead = lineage.base_hash != lineage.current_hash();
        match (ahead, behind) {
            (false, false) => Self::UpToDate,
            (false, true) => Self::Behind,
            (true, false) => Self::Ahead,
            (true, true) => Self::Diverged,
        }
    }
}

/// Some auxiliary struct that we use instead of `Row` when the file is not yet commited
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PackageFileFingerprint {
    // FIXME: re-use Row
    pub size: u64,
    pub hash: Multihash<256>,
}

/// Status of the package and workding directory of the pakage
#[derive(Debug, PartialEq, Default)]
pub struct InstalledPackageStatus {
    /// Current commit vs upstream state
    pub upstream_state: UpstreamState,
    /// File changes vs current commit
    pub changes: ChangeSet,
    // XXX: meta?
}

impl InstalledPackageStatus {
    pub fn new(upstream_state: UpstreamState, changes: ChangeSet) -> Self {
        Self {
            upstream_state,
            changes,
        }
    }
}

/// What is the latest commit and what are previous commits if present
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct CommitState {
    /// When the last commit was done
    pub timestamp: chrono::DateTime<chrono::Utc>,
    /// What is the hash of the latest commit
    pub hash: String, // TODO: use multihash?
    /// What are the previous comit hashes
    #[serde(default = "Vec::new")]
    pub prev_hashes: Vec<String>, // TODO: use multihashes?
}

/// State of the file tracked in lineage
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PathState {
    /// Last "modified" date.
    /// Last time it was installed or commited.
    pub timestamp: chrono::DateTime<chrono::Utc>,
    /// Last tracked hash.
    /// We don't track files modifications in real time.
    /// We calculate hash when we commit or install file.
    #[serde(
        serialize_with = "multihash_to_str",
        deserialize_with = "str_to_multihash"
    )]
    pub hash: Multihash<256>,
}

fn multihash_to_str<S: Serializer>(
    hash: &Multihash<256>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    let s = hex::encode(hash.to_bytes());
    serializer.serialize_str(&s)
}

fn str_to_multihash<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Multihash<256>, D::Error> {
    let s = String::deserialize(deserializer)?;
    let bytes = hex::decode(s).map_err(DeserializeError::custom)?;
    Multihash::from_bytes(&bytes).map_err(DeserializeError::custom)
}

/// A map of paths to their state
///
/// The key is the name of the path, and the value is the state of the path
pub type LineagePaths = BTreeMap<PathBuf, PathState>;

/// Stores lineage (installation/modification history) of the package read from lineage.json file
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct PackageLineage {
    /// Local commits
    pub commit: Option<CommitState>,
    /// Where we intsalled this package from
    pub remote: ManifestUri,
    /// TODO: I understand yet how and why we use it
    pub base_hash: String,
    /// Latest tracked hash. In other words, what was the remote hash when we last checked.
    /// It can be different from the `remote.hash`, because we can install not the latest package.
    pub latest_hash: String,
    /// Installed paths (or files in other words)
    #[serde(default = "BTreeMap::new")]
    pub paths: LineagePaths,
}

impl PackageLineage {
    pub fn from_remote(remote: ManifestUri, latest_hash: String) -> Self {
        Self {
            base_hash: remote.hash.clone(),
            remote,
            latest_hash,
            commit: None,
            paths: BTreeMap::new(),
        }
    }

    pub fn current_hash(&self) -> &str {
        self.commit.as_ref().map_or(&self.remote.hash, |c| &c.hash)
    }

    pub fn update_latest(&mut self, manifest_uri: ManifestUri) {
        let new_latest_hash = manifest_uri.hash;
        self.latest_hash.clone_from(&new_latest_hash);
        self.base_hash.clone_from(&new_latest_hash);
    }
}

impl From<ManifestUri> for PackageLineage {
    fn from(uri: ManifestUri) -> Self {
        Self {
            base_hash: uri.hash.clone(),
            remote: uri.clone(),
            latest_hash: uri.hash.clone(),
            commit: None,
            paths: BTreeMap::new(),
        }
    }
}

/// It's essentially just a map of `PackageLineage`.
/// Represents the contents of `.quilt/lineage.json`
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct DomainLineage {
    #[serde(default = "BTreeMap::new")]
    pub packages: BTreeMap<Namespace, PackageLineage>,
}

impl TryFrom<&str> for DomainLineage {
    type Error = Error;

    fn try_from(input: &str) -> Result<Self, Self::Error> {
        serde_json::from_str(input).map_err(Error::LineageParse)
    }
}

impl TryFrom<Vec<u8>> for DomainLineage {
    type Error = Error;

    fn try_from(input: Vec<u8>) -> Result<Self, Self::Error> {
        serde_json::from_slice(&input).map_err(Error::LineageParse)
    }
}

/// Wrapper for reading and writing `DomainLineage`
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DomainLineageIo {
    path: PathBuf,
}

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

    pub async fn read(&self, storage: &impl Storage) -> Res<PackageLineage> {
        let domain_lineage = self.domain_lineage.read(storage).await?;
        let namespace = domain_lineage.packages.get(&self.namespace);

        match namespace {
            Some(ns) => Ok(ns.clone()),
            None => Err(Error::PackageNotInstalled(self.namespace.clone())),
        }
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

    use crate::checksum::calculate_sha256_chunked_checksum;
    use crate::mocks;
    use base64::prelude::BASE64_STANDARD;
    use base64::Engine;

    #[test]
    fn test_syntax_error() {
        assert_eq!(
            DomainLineage::try_from("err").unwrap_err().to_string(),
            "Failed to parse lineage file: expected value at line 1 column 1".to_string()
        );
    }

    #[test]
    fn test_wrong_key() {
        // NOTE: @fiskus I don't think this is developer friendly
        //       I'd like to remove serde(default), so this test fails
        assert_eq!(
            DomainLineage::try_from(r#"{"notkey": 123}"#).unwrap(),
            DomainLineage {
                packages: BTreeMap::new(),
            }
        );
    }

    #[test]
    fn test_wrong_value() {
        assert!(DomainLineage::try_from(r#"{"packages": 123}"#)
            .unwrap_err()
            .to_string()
            .starts_with("Failed to parse lineage file: invalid type:"));
    }

    #[test]
    fn test_parsing_json_ok() {
        assert_eq!(
            DomainLineage::try_from(r###"{"packages":{}}"###).unwrap(),
            DomainLineage {
                packages: BTreeMap::new(),
            }
        )
    }

    #[test]
    fn test_vec8_parsing_json_ok() {
        assert_eq!(
            DomainLineage::try_from(r###"{"packages":{}}"###.as_bytes().to_vec()).unwrap(),
            DomainLineage {
                packages: BTreeMap::new(),
            }
        )
    }

    #[tokio::test]
    async fn test_domain_lineage_from_file() -> Res {
        let storage = mocks::storage::MockStorage::default();
        let file_path = PathBuf::from("foo");
        storage
            .write_file(&file_path, br###"{"packages":{}}"###.as_ref())
            .await?;
        let lineage = DomainLineageIo::new(file_path).read(&storage).await?;
        assert_eq!(lineage, DomainLineage::default());
        Ok(())
    }

    #[tokio::test]
    async fn test_domain_lineage_from_nothing() -> Res {
        let storage = mocks::storage::MockStorage::default();
        let lineage = DomainLineageIo::new(PathBuf::from("does-not-exist"))
            .read(&storage)
            .await?;
        assert_eq!(lineage, DomainLineage::default());
        Ok(())
    }

    #[tokio::test]
    async fn test_domain_lineage_write() -> Res {
        let storage = mocks::storage::MockStorage::default();
        let file_path = PathBuf::from("foo");
        assert!(!storage.exists(&file_path).await);
        let bytes = "0123456789abcdef".as_bytes();
        DomainLineageIo::new(file_path.clone())
            .write(
                &storage,
                DomainLineage {
                    packages: BTreeMap::from([(
                        ("foo", "bar").into(),
                        PackageLineage {
                            commit: None,
                            remote: ManifestUri {
                                bucket: "bucket".to_string(),
                                namespace: ("foo", "bar").into(),
                                hash: "abcdef".to_string(),
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
        let manifest = br###"{
  "packages": {
    "foo/bar": {
      "commit": null,
      "remote": {
        "bucket": "bucket",
        "namespace": "foo/bar",
        "hash": "abcdef"
      },
      "base_hash": "abcdef",
      "latest_hash": "abcdef",
      "paths": {
        "foo": {
          "timestamp": "2025-01-16T12:50:20.534Z",
          "hash": "90ea02205dbd4f6e325e5a87f8cc3ef3b8773d3c8eec2e2cff6248f882986569912ddf10"
        }
      }
    }
  }
}"###
            .to_vec();
        assert_eq!(
            String::from_utf8(storage.read_file(&file_path).await?).unwrap(),
            String::from_utf8(manifest.clone()).unwrap()
        );
        let lineage = DomainLineage::try_from(manifest)?;
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
