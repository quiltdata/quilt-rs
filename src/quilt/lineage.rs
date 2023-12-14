use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::{manifest::ContentHash, RemoteManifest};

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
    pub hash: ContentHash,
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
