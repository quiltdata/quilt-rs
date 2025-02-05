use serde::Deserialize;
use serde::Serialize;

use crate::paths;
use crate::uri::Namespace;
use crate::uri::RevisionPointer;
use crate::uri::S3PackageUri;
use crate::uri::S3Uri;
use crate::Error;

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

impl TryFrom<S3PackageUri> for ManifestUri {
    type Error = Error;
    fn try_from(uri: S3PackageUri) -> Result<Self, Self::Error> {
        Ok(ManifestUri {
            bucket: uri.bucket,
            namespace: uri.namespace,
            hash: match uri.revision {
                RevisionPointer::Hash(top_hash) => top_hash,
                RevisionPointer::Tag(_) => {
                    return Err(Error::PackageURI(
                        "Hash is required for that conversion".to_string(),
                    ))
                }
            },
        })
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Res;

    #[test]
    fn test_manifest_uri_try_from_package_uri_with_tag() -> Res {
        let package_uri = S3PackageUri {
            bucket: "test-bucket".to_string(),
            namespace: ("test", "package").into(),
            revision: RevisionPointer::Tag("latest".to_string()),
            path: None,
            catalog: None,
        };

        let result = ManifestUri::try_from(package_uri);
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            "Invalid package URI: Hash is required for that conversion"
        );
        Ok(())
    }
}
