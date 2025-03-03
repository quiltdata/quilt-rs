use std::collections::BTreeMap;
use std::path::PathBuf;

use multihash::Multihash;
use serde::de;
use serde::ser;
use serde::Deserialize;
use serde::Serialize;

use crate::lineage::status::UpstreamState;
use crate::uri::ManifestUri;

fn multihash_to_str<S: ser::Serializer>(
    hash: &Multihash<256>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    let s = hex::encode(hash.to_bytes());
    serializer.serialize_str(&s)
}

fn str_to_multihash<'de, D: de::Deserializer<'de>>(
    deserializer: D,
) -> Result<Multihash<256>, D::Error> {
    let s = String::deserialize(deserializer)?;
    let bytes = hex::decode(s).map_err(de::Error::custom)?;
    Multihash::from_bytes(&bytes).map_err(de::Error::custom)
}

/// State of the file tracked in lineage
#[derive(Default, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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

/// A map of paths to their state
///
/// The key is the name of the path, and the value is the state of the path
pub type LineagePaths = BTreeMap<PathBuf, PathState>;

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

/// Stores lineage (installation/modification history) of the package read from `data.json` file
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct PackageLineage {
    /// Local commits
    pub commit: Option<CommitState>,
    /// Where we installed this package from
    pub remote: ManifestUri,
    // TODO: I don't understand yet how and why we use it
    pub base_hash: String,
    /// Latest tracked hash. In other words, what was the remote hash when we last checked.
    /// It can be different from the `remote.hash`, because we can install not the latest package.
    pub latest_hash: String,
    /// Installed paths (or files in other words)
    #[serde(default = "BTreeMap::new")]
    pub paths: LineagePaths,
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
