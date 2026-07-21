use std::collections::BTreeMap;
use std::path::PathBuf;

use multihash::Multihash;
use serde::Deserialize;
use serde::Serialize;
use serde::de;
use serde::ser;

use crate::Error;
use crate::Res;
use crate::error::LineageError;
use crate::lineage::status::UpstreamState;
use quilt_uri::ManifestUri;

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
///
/// TODO: migrate `hash` and `prev_hashes` to a `TopHash` newtype around
/// `Multihash<256>` that validates the SHA-256 multicodec on
/// construction (pairing with the existing `TopHasher` builder in
/// `manifest::top_hasher`). A bare `Multihash<256>` is not enough:
///
/// - The on-disk format in `data.json` is hex-of-digest only (no
///   multicodec prefix), so the multicodec must be re-attached on
///   deserialization.
/// - The codebase also uses CRC64 and SHA-256-chunked multihashes
///   elsewhere, so any helper that strips the multicodec for
///   serialization must guarantee SHA-256 — otherwise a wrong-codec
///   multihash passes through silently and writes a corrupt hash.
///
/// Best done together with the adjacent `String` hashes
/// (`PackageLineage::base_hash`, `latest_hash`, `ManifestUri::hash`);
/// migrating only `CommitState` turns every comparison with those
/// fields into a conversion boundary and leaves the type-safety win
/// partial.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct CommitState {
    /// When the last commit was done
    pub timestamp: chrono::DateTime<chrono::Utc>,
    /// What is the hash of the latest commit
    pub hash: String,
    /// What are the previous commit hashes
    #[serde(default = "Vec::new")]
    pub prev_hashes: Vec<String>,
}

/// Stores lineage (installation/modification history) of the package read from `data.json` file
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct PackageLineage {
    /// Local commits
    pub commit: Option<CommitState>,
    /// Where we installed this package from. `None` for local-only packages.
    #[serde(default, rename = "remote", skip_serializing_if = "Option::is_none")]
    pub remote_uri: Option<ManifestUri>,
    // TODO: I don't understand yet how and why we use it
    pub base_hash: String,
    /// Latest tracked hash. In other words, what was the remote hash when we last checked.
    /// It can be different from the `remote.hash`, because we can install not the latest package.
    ///
    /// TODO: pair with `checked_at: DateTime<Utc>`. The classifier treats
    /// this as authoritative for `Behind`/`Diverged` with no notion of
    /// staleness.
    pub latest_hash: String,
    /// Installed paths (or files in other words)
    #[serde(default = "BTreeMap::new")]
    pub paths: LineagePaths,
}

impl From<PackageLineage> for UpstreamState {
    fn from(lineage: PackageLineage) -> Self {
        let Some(remote) = lineage.remote_uri.as_ref() else {
            return Self::Local;
        };
        // `set_remote` rejects empty buckets, so on a production-written
        // `data.json` this branch is unreachable. It exists to keep
        // hand-edited or test-fixture lineages from falling through to
        // the `hash.is_empty()` arm below, which would mis-classify a
        // bucket-less remote with a non-empty `latest_hash` as
        // `Diverged`. Covered by
        // `test_empty_bucket_is_local_even_with_latest_hash`.
        if remote.bucket.is_empty() {
            return Self::Local;
        }
        if remote.hash.is_empty() {
            // `remote.hash` empty + `latest_hash` empty: truly first push,
            // the remote bucket has no revision for this namespace yet.
            // `remote.hash` empty + `latest_hash` non-empty: a teammate has
            // already published under this namespace, so we are not really
            // local — autosync must refuse to push and surface the conflict.
            //
            // Note: this classification deliberately ignores `origin`. A
            // bucket-only (no-catalog) CLI push leaves `origin = None` but
            // is otherwise a normal remote-tracking package. The autosync
            // watcher applies its own `origin.is_none()` skip rule when
            // deciding whether to act on a package; the state classifier
            // here just reports what is.
            return if lineage.latest_hash.is_empty() {
                Self::Local
            } else {
                Self::Diverged
            };
        }
        let behind = lineage.base_hash != lineage.latest_hash;
        let ahead = lineage.base_hash != lineage.current_hash().unwrap_or_default();
        match (ahead, behind) {
            (false, false) => Self::UpToDate,
            (false, true) => Self::Behind,
            (true, false) => Self::Ahead,
            (true, true) => Self::Diverged,
        }
    }
}

impl PackageLineage {
    /// Returns the remote `ManifestUri`, or an error if this is a local-only package.
    pub fn remote(&self) -> Res<&ManifestUri> {
        self.remote_uri
            .as_ref()
            .ok_or(Error::Lineage(LineageError::NoRemote))
    }

    /// Returns a mutable reference to the remote `ManifestUri`,
    /// or an error if this is a local-only package.
    pub fn remote_mut(&mut self) -> Res<&mut ManifestUri> {
        self.remote_uri
            .as_mut()
            .ok_or(Error::Lineage(LineageError::NoRemote))
    }

    #[must_use]
    pub fn from_remote(remote: ManifestUri, latest_hash: String) -> Self {
        Self {
            base_hash: remote.hash.clone(),
            remote_uri: Some(remote),
            latest_hash,
            commit: None,
            paths: BTreeMap::new(),
        }
    }

    #[must_use]
    pub fn current_hash(&self) -> Option<&str> {
        self.commit
            .as_ref()
            .map(|c| c.hash.as_str())
            .or(self.remote_uri.as_ref().map(|r| r.hash.as_str()))
            .or(if self.base_hash.is_empty() {
                None
            } else {
                Some(self.base_hash.as_str())
            })
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
            remote_uri: Some(uri.clone()),
            latest_hash: uri.hash.clone(),
            commit: None,
            paths: BTreeMap::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_is_local() {
        assert_eq!(
            UpstreamState::from(PackageLineage::default()),
            UpstreamState::Local
        );
    }

    #[test]
    fn test_remote_configured_but_never_pushed_is_local() {
        let lineage = PackageLineage {
            remote_uri: Some(ManifestUri {
                hash: String::new(),
                bucket: "test-bucket".to_string(),
                namespace: ("foo", "bar").into(),
                ..ManifestUri::default()
            }),
            ..PackageLineage::default()
        };
        assert_eq!(UpstreamState::from(lineage), UpstreamState::Local);
    }

    #[test]
    fn test_empty_bucket_is_local_even_with_latest_hash() {
        // `set_remote` rejects empty buckets at the only production
        // write site, so this state should not occur from normal use.
        // The guard exists for hand-edited `data.json` files: without
        // it, the next arm would classify a bucket-less remote with a
        // non-empty `latest_hash` as `Diverged`, which is nonsense (no
        // real remote to be diverged from).
        let lineage = PackageLineage {
            remote_uri: Some(ManifestUri {
                hash: String::new(),
                bucket: String::new(),
                namespace: ("foo", "bar").into(),
                origin: Some("catalog.dev".parse().unwrap()),
            }),
            latest_hash: "abc".to_string(),
            ..PackageLineage::default()
        };
        assert_eq!(UpstreamState::from(lineage), UpstreamState::Local);
    }

    #[test]
    fn test_foreign_remote_with_latest_is_diverged() {
        let lineage = PackageLineage {
            remote_uri: Some(ManifestUri {
                hash: String::new(),
                bucket: "test-bucket".to_string(),
                namespace: ("foo", "bar").into(),
                origin: Some("catalog.dev".parse().unwrap()),
            }),
            latest_hash: "abc".to_string(),
            ..PackageLineage::default()
        };
        assert_eq!(UpstreamState::from(lineage), UpstreamState::Diverged);
    }

    #[test]
    fn test_remote_returns_no_remote_error() {
        let lineage = PackageLineage::default();
        assert!(matches!(
            lineage.remote(),
            Err(Error::Lineage(LineageError::NoRemote))
        ));
    }

    #[test]
    fn test_current_hash_without_remote() {
        let lineage = PackageLineage::default();
        assert_eq!(lineage.current_hash(), None);
    }
}
