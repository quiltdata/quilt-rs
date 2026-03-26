use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;

use multihash::Multihash;
use serde::de;
use serde::ser;
use serde::Deserialize;
use serde::Serialize;

use crate::lineage::status::UpstreamState;
use crate::uri::Host;
use crate::uri::ManifestUri;
use crate::uri::RevisionPointer;
use crate::uri::S3PackageHandle;
use crate::uri::S3PackageUri;

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
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct RemotePackage {
    pub origin: Option<Host>,
    pub bucket: String,
    pub namespace: crate::uri::Namespace,
}

impl RemotePackage {
    pub fn display(&self) -> String {
        S3PackageUri {
            catalog: self.origin.clone(),
            bucket: self.bucket.clone(),
            namespace: self.namespace.clone(),
            revision: RevisionPointer::default(),
            path: None,
        }
        .display()
    }

    pub fn manifest_uri(&self, hash: impl Into<String>) -> ManifestUri {
        ManifestUri {
            origin: self.origin.clone(),
            bucket: self.bucket.clone(),
            namespace: self.namespace.clone(),
            hash: hash.into(),
        }
    }

    pub fn package_handle(&self) -> S3PackageHandle {
        S3PackageHandle {
            bucket: self.bucket.clone(),
            namespace: self.namespace.clone(),
        }
    }
}

impl fmt::Display for RemotePackage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.display())
    }
}

impl From<ManifestUri> for RemotePackage {
    fn from(uri: ManifestUri) -> Self {
        Self {
            origin: uri.origin,
            bucket: uri.bucket,
            namespace: uri.namespace,
        }
    }
}

impl From<&ManifestUri> for RemotePackage {
    fn from(uri: &ManifestUri) -> Self {
        uri.clone().into()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct PackageLineage {
    /// Local commits
    pub commit: Option<CommitState>,
    /// Where this package lives remotely
    pub remote: RemotePackage,
    /// The currently installed or pushed remote revision, if any
    pub remote_hash: Option<String>,
    // TODO: I don't understand yet how and why we use it
    pub base_hash: Option<String>,
    /// Latest tracked hash. In other words, what was the remote hash when we last checked.
    /// It can be different from the current remote revision, because we can install not the latest package.
    pub latest_hash: Option<String>,
    /// Installed paths (or files in other words)
    #[serde(default = "BTreeMap::new")]
    pub paths: LineagePaths,
}

impl From<PackageLineage> for UpstreamState {
    fn from(lineage: PackageLineage) -> Self {
        let behind = lineage.base_hash != lineage.latest_hash;
        let ahead = lineage.base_hash.as_deref() != lineage.current_hash();
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
            base_hash: Some(remote.hash.clone()),
            remote_hash: Some(remote.hash.clone()),
            remote: (&remote).into(),
            latest_hash: Some(latest_hash),
            commit: None,
            paths: BTreeMap::new(),
        }
    }

    pub fn from_package(remote: RemotePackage) -> Self {
        Self {
            commit: None,
            remote,
            remote_hash: None,
            base_hash: None,
            latest_hash: None,
            paths: BTreeMap::new(),
        }
    }

    pub fn current_hash(&self) -> Option<&str> {
        self.commit
            .as_ref()
            .map(|c| c.hash.as_str())
            .or(self.remote_hash.as_deref())
    }

    pub fn remote_manifest_uri(&self) -> Option<ManifestUri> {
        self.remote_hash
            .as_ref()
            .map(|hash| self.remote.manifest_uri(hash.clone()))
    }

    pub fn latest_manifest_uri(&self) -> Option<ManifestUri> {
        self.latest_hash
            .as_ref()
            .map(|hash| self.remote.manifest_uri(hash.clone()))
    }

    pub fn update_latest(&mut self, manifest_uri: ManifestUri) {
        let new_latest_hash = manifest_uri.hash.clone();
        self.remote = (&manifest_uri).into();
        self.remote_hash = Some(new_latest_hash.clone());
        self.latest_hash = Some(new_latest_hash.clone());
        self.base_hash = Some(new_latest_hash);
    }
}

impl From<ManifestUri> for PackageLineage {
    fn from(uri: ManifestUri) -> Self {
        Self {
            base_hash: Some(uri.hash.clone()),
            remote: (&uri).into(),
            remote_hash: Some(uri.hash.clone()),
            latest_hash: Some(uri.hash.clone()),
            commit: None,
            paths: BTreeMap::new(),
        }
    }
}
