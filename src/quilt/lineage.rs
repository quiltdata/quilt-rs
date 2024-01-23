use std::collections::BTreeMap;

use multihash::Multihash;
use serde::{Deserialize, Serialize, Deserializer, de::Error, Serializer};

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
    #[serde(serialize_with = "multihash_to_str", deserialize_with = "str_to_multihash")]
    pub hash: Multihash<256>,
}

fn multihash_to_str<S: Serializer>(hash: &Multihash<256>, serializer: S) -> Result<S::Ok, S::Error> {
    let s = hex::encode(hash.digest());
    serializer.serialize_str(&s)
}

fn str_to_multihash<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Multihash<256>, D::Error> {
    let s = String::deserialize(deserializer)?;
    let bytes = hex::decode(s).map_err(Error::custom)?;
    Multihash::from_bytes(&bytes).map_err(Error::custom)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DomainLineage {
    #[serde(default = "BTreeMap::new")]
    pub packages: BTreeMap<String, PackageLineage>,
}

impl TryFrom<&str> for DomainLineage {
    type Error = String;

    fn try_from(input: &str) -> Result<Self, Self::Error> {
        let parsed: Self = serde_json::from_str(input)
            .map_err(|err| format!("Failed to parse the lineage file: {}", err.to_string()))?;
        Ok(parsed)
    }
}
