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

use crate::quilt::storage::Storage;
use crate::quilt::uri::Namespace;
use crate::Error;

use super::RemoteManifest;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct CommitState {
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub hash: String,
    #[serde(default = "Vec::new")]
    pub prev_hashes: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PathState {
    pub timestamp: chrono::DateTime<chrono::Utc>,
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

#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct PackageLineage {
    pub commit: Option<CommitState>,
    pub remote: RemoteManifest,
    pub base_hash: String,
    pub latest_hash: String,
    // installed paths
    #[serde(default = "BTreeMap::new")]
    pub paths: LineagePaths,
}

impl PackageLineage {
    pub fn from_remote(remote: RemoteManifest, latest_hash: String) -> Self {
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
}

#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct DomainLineage {
    pub packages: BTreeMap<Namespace, PackageLineage>,
}

#[derive(Serialize, Deserialize)]
pub struct DomainLineageJson {
    #[serde(default = "BTreeMap::new")]
    pub packages: BTreeMap<String, PackageLineage>,
}

impl TryFrom<DomainLineageJson> for DomainLineage {
    type Error = Error;

    fn try_from(input: DomainLineageJson) -> Result<Self, Self::Error> {
        let mut packages = BTreeMap::new();
        for (key, value) in input.packages.iter() {
            packages.insert(Namespace::try_from(key.to_string())?, value.clone());
        }
        Ok(DomainLineage { packages })
    }
}

impl From<&DomainLineage> for DomainLineageJson {
    fn from(input: &DomainLineage) -> Self {
        let mut packages = BTreeMap::new();
        for (key, value) in input.packages.iter() {
            packages.insert(key.to_string(), value.clone());
        }
        DomainLineageJson { packages }
    }
}

impl TryFrom<&str> for DomainLineage {
    type Error = Error;

    fn try_from(input: &str) -> Result<Self, Self::Error> {
        serde_json::from_str::<DomainLineageJson>(input)
            .map_err(Error::LineageParse)?
            .try_into()
    }
}

impl TryFrom<Vec<u8>> for DomainLineage {
    type Error = Error;

    fn try_from(input: Vec<u8>) -> Result<Self, Self::Error> {
        serde_json::from_slice::<DomainLineageJson>(&input)
            .map_err(Error::LineageParse)?
            .try_into()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DomainLineageIo {
    path: PathBuf,
}

impl DomainLineageIo {
    pub fn new(path: PathBuf) -> Self {
        DomainLineageIo { path }
    }

    pub async fn read(&self, storage: &impl Storage) -> Result<DomainLineage, Error> {
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
    ) -> Result<DomainLineage, Error> {
        // Ok(serde_json::to_string_pretty(&lineage)?)
        let contents = serde_json::to_string_pretty(&lineage)?;
        storage
            .write_file(self.path.clone(), contents.as_bytes())
            .await?;
        Ok(lineage)
        // let contents = serde_json::to_string_pretty(&DomainLineageJson::from(&lineage))?;
        // storage
        //     .write_file(self.path.clone(), contents.as_bytes())
        //     .await?;
        // Ok(lineage)
    }

    pub fn create_package_lineage(&self, namespace: Namespace) -> PackageLineageIo {
        PackageLineageIo::new(self.clone(), namespace)
    }
}

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

    pub async fn read(&self, storage: &impl Storage) -> Result<PackageLineage, Error> {
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
    ) -> Result<PackageLineage, Error> {
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

    use crate::quilt::storage::mock_storage::MockStorage;

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
    async fn test_domain_lineage_from_file() -> Result<(), Error> {
        let storage = MockStorage::default();
        let file_path = PathBuf::from("foo");
        storage
            .write_file(&file_path, br###"{"packages":{}}"###.as_ref())
            .await?;
        let lineage = DomainLineageIo::new(file_path).read(&storage).await?;
        assert_eq!(lineage, DomainLineage::default());
        Ok(())
    }

    #[tokio::test]
    async fn test_domain_lineage_from_nothing() -> Result<(), Error> {
        let storage = MockStorage::default();
        let lineage = DomainLineageIo::new(PathBuf::from("does-not-exist"))
            .read(&storage)
            .await?;
        assert_eq!(lineage, DomainLineage::default());
        Ok(())
    }

    #[tokio::test]
    async fn test_domain_lineage_write() -> Result<(), Error> {
        let storage = MockStorage::default();
        let file_path = PathBuf::from("foo");
        assert!(!storage.exists(&file_path).await);
        DomainLineageIo::new(file_path.clone())
            .write(&storage, DomainLineage::default())
            .await?;
        assert!(storage.exists(&file_path).await);
        let manifest = br###"{
  "packages": {}
}"###
            .to_vec();
        assert_eq!(storage.read_file(&file_path).await?, manifest);
        Ok(())
    }

    #[tokio::test]
    async fn test_domain_lineage_create_package_lineage() -> Result<(), Error> {
        let namespace = Namespace::from(("foo", "bar"));
        let domain_lineage = DomainLineageIo::default();
        let lineage = domain_lineage.create_package_lineage(namespace.clone());
        assert_eq!(
            lineage,
            PackageLineageIo {
                namespace,
                domain_lineage,
            }
        );
        Ok(())
    }
}
