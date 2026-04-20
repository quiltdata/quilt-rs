use std::fmt;

use serde::Deserialize;
use serde::Serialize;

use crate::error::UriError;
use crate::paths;
use crate::uri::Host;
use crate::uri::Namespace;
use crate::uri::RevisionPointer;
use crate::uri::S3PackageUri;
use crate::uri::S3Uri;
use crate::Error;

/// URI for manifest.
/// Manifests are stored in immutable files.
/// They are s3-unversioned but have hash.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ManifestUri {
    pub origin: Option<Host>,
    pub bucket: String,
    pub namespace: Namespace,
    pub hash: String,
}

impl TryFrom<S3PackageUri> for ManifestUri {
    type Error = Error;
    fn try_from(uri: S3PackageUri) -> Result<Self, Self::Error> {
        Ok(ManifestUri {
            bucket: uri.bucket,
            origin: uri.catalog,
            namespace: uri.namespace,
            hash: match uri.revision {
                RevisionPointer::Hash(top_hash) => top_hash,
                RevisionPointer::Tag(_) => {
                    return Err(UriError::Package(
                        "Hash is required for that conversion".to_string(),
                    )
                    .into())
                }
            },
        })
    }
}

impl From<ManifestUri> for S3Uri {
    fn from(remote: ManifestUri) -> S3Uri {
        S3Uri {
            bucket: remote.bucket,
            key: paths::get_manifest_key_legacy(&remote.hash),
            version: None,
        }
    }
}

impl fmt::Display for ManifestUri {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let uri = S3PackageUri::from(self);
        write!(f, "{uri}")
    }
}

impl ManifestUri {
    pub fn display(&self) -> String {
        S3PackageUri::from(self).display()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::Res;

    #[test]
    fn test_manifest_uri_try_from_package_uri_with_tag() -> Res {
        let package_uri = S3PackageUri {
            bucket: "foo".to_string(),
            namespace: ("bar", "baz").into(),
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

    #[test]
    fn test_manifest_uri_try_from_package_uri_with_hash() -> Res {
        assert_eq!(
            ManifestUri::try_from(S3PackageUri {
                bucket: "test-bucket".to_string(),
                namespace: ("foo", "bar").into(),
                revision: RevisionPointer::Hash("abc123".to_string()),
                path: None,
                catalog: None,
            })?,
            ManifestUri {
                bucket: "test-bucket".to_string(),
                namespace: ("foo", "bar").into(),
                hash: "abc123".to_string(),
                origin: None,
            }
        );
        Ok(())
    }

    #[test]
    fn test_manifest_uri_to_s3uri() {
        assert_eq!(
            S3Uri::from(ManifestUri {
                origin: None,
                bucket: "test-bucket".to_string(),
                namespace: ("ignored", "ignored").into(),
                hash: "abc123".to_string(),
            }),
            S3Uri {
                bucket: "test-bucket".to_string(),
                key: ".quilt/packages/abc123".to_string(),
                version: None,
            }
        );
    }
}
