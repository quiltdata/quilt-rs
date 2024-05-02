use serde::Deserialize;
use serde::Serialize;

use crate::paths;
use crate::uri::Namespace;
use crate::uri::S3Uri;

#[derive(Clone, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestUri {
    pub bucket: String,
    pub namespace: Namespace,
    pub hash: String,
}

impl From<&ManifestUri> for S3Uri {
    fn from(remote: &ManifestUri) -> S3Uri {
        S3Uri {
            bucket: remote.bucket.clone(),
            key: paths::get_manifest_key(&remote.hash),
            version: None,
        }
    }
}
