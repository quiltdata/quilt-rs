use std::collections::BTreeMap;
use std::path::PathBuf;
use tokio::fs;

use multihash::Multihash;
use serde::{de::Error as DeserializeError, Deserialize, Deserializer, Serialize, Serializer};

use crate::Error;

use super::RemoteManifest;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PackageLineage {
    pub commit: Option<CommitState>,
    pub remote: RemoteManifest,
    pub base_hash: String,
    pub latest_hash: String,
    // installed paths
    #[serde(default = "BTreeMap::new")]
    pub paths: BTreeMap<String, PathState>,
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

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct DomainLineage {
    #[serde(default = "BTreeMap::new")]
    pub packages: BTreeMap<String, PackageLineage>,
}

impl TryFrom<&str> for DomainLineage {
    type Error = Error;

    fn try_from(input: &str) -> Result<Self, Self::Error> {
        serde_json::from_str(input).map_err(Error::LineageParse)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DomainLineageIo {
    path: PathBuf,
}

impl DomainLineageIo {
    pub fn new(path: PathBuf) -> Self {
        DomainLineageIo { path }
    }

    pub async fn read(&self) -> Result<DomainLineage, Error> {
        let contents = fs::read_to_string(&self.path).await.or_else(|err| {
            if err.kind() == std::io::ErrorKind::NotFound {
                Ok("{}".into())
            } else {
                Err(err)
            }
        })?;

        DomainLineage::try_from(&contents[..])
    }

    pub async fn write(&self, lineage: &DomainLineage) -> Result<(), Error> {
        let contents = serde_json::to_string_pretty(lineage)?;
        fs::write(&self.path, contents.as_bytes()).await?;
        Ok(())
    }

    pub fn create_package_lineage(&self, namespace: String) -> PackageLineageIo {
        PackageLineageIo::new(self.clone(), namespace)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageLineageIo {
    domain_lineage: DomainLineageIo,
    namespace: String,
}

impl PackageLineageIo {
    pub fn new(domain_lineage: DomainLineageIo, namespace: String) -> Self {
        PackageLineageIo {
            domain_lineage,
            namespace,
        }
    }

    pub async fn read(&self) -> Result<PackageLineage, Error> {
        let domain_lineage = self.domain_lineage.read().await?;
        let namespace = domain_lineage.packages.get(&self.namespace);

        match namespace {
            Some(ns) => Ok(ns.clone()),
            None => Err(Error::PackageNotInstalled(self.namespace.clone())),
        }
    }

    pub async fn write(&self, lineage: PackageLineage) -> Result<(), Error> {
        let mut domain_lineage = self.domain_lineage.read().await?;
        domain_lineage
            .packages
            .insert(self.namespace.clone(), lineage);
        self.domain_lineage.write(&domain_lineage).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
