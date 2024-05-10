use serde::Deserialize;
use serde::Serialize;

use crate::paths;
use crate::uri::Namespace;
use crate::uri::S3Uri;

/// URI for manifest.
/// Manifests are stored in immutable files.
/// They are s3-unversioned but have hash.
///
/// This manifest URI is for manifest file in Parquet format.
#[derive(Clone, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestUri {
    pub bucket: String,
    pub namespace: Namespace,
    pub hash: String,
}

impl From<ManifestUri> for S3Uri {
    fn from(remote: ManifestUri) -> S3Uri {
        S3Uri {
            bucket: remote.bucket,
            key: paths::get_manifest_key(&remote.hash),
            version: None,
        }
    }
}

impl From<&ManifestUri> for S3Uri {
    fn from(remote: &ManifestUri) -> S3Uri {
        remote.clone().into()
    }
}

/// The same as `ManifestUri` but for legacy JSONL format
#[derive(Clone, Debug)]
pub struct ManifestUriLegacy {
    pub bucket: String,
    pub namespace: Namespace,
    pub hash: String,
}

impl From<ManifestUriLegacy> for S3Uri {
    fn from(remote: ManifestUriLegacy) -> S3Uri {
        S3Uri {
            bucket: remote.bucket,
            key: paths::get_manifest_key_legacy(&remote.hash),
            version: None,
        }
    }
}

impl From<&ManifestUriLegacy> for S3Uri {
    fn from(remote: &ManifestUriLegacy) -> S3Uri {
        remote.clone().into()
    }
}

impl From<ManifestUri> for ManifestUriLegacy {
    fn from(manifest_uri: ManifestUri) -> Self {
        ManifestUriLegacy {
            bucket: manifest_uri.bucket,
            namespace: manifest_uri.namespace,
            hash: manifest_uri.hash,
        }
    }
}

impl From<&ManifestUri> for ManifestUriLegacy {
    fn from(manifest_uri: &ManifestUri) -> Self {
        manifest_uri.clone().into()
    }
}
