use std::fmt;

use serde::Deserialize;
use serde::Serialize;

use crate::Host;
use crate::Namespace;
use crate::RevisionPointer;
use crate::S3PackageUri;
use crate::S3Uri;
use crate::UriError;
use crate::package::check_top_hash;
use crate::paths;
use crate::s3::check_bucket;

/// URI for manifest.
/// Manifests are stored in immutable files.
/// They are s3-unversioned but have hash.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawManifestUri")]
#[cfg_attr(any(test, feature = "test-support"), derive(Default))]
pub struct ManifestUri {
    pub origin: Option<Host>,
    pub bucket: String,
    pub namespace: Namespace,
    pub hash: String,
}

/// The serialized form of [`ManifestUri`], read before its bucket and hash are
/// checked.
#[derive(Deserialize)]
struct RawManifestUri {
    origin: Option<Host>,
    bucket: String,
    namespace: Namespace,
    hash: String,
}

impl TryFrom<RawManifestUri> for ManifestUri {
    type Error = UriError;

    fn try_from(raw: RawManifestUri) -> Result<Self, Self::Error> {
        check_bucket(&raw.bucket)?;
        // An empty hash is a remote that is set but not pushed yet.
        if !raw.hash.is_empty() {
            check_top_hash(&raw.hash)?;
        }
        Ok(Self {
            origin: raw.origin,
            bucket: raw.bucket,
            namespace: raw.namespace,
            hash: raw.hash,
        })
    }
}

impl TryFrom<S3PackageUri> for ManifestUri {
    type Error = UriError;
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
                    ));
                }
            },
        })
    }
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

impl fmt::Display for ManifestUri {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let uri = S3PackageUri::from(self);
        write!(f, "{uri}")
    }
}

impl ManifestUri {
    #[must_use]
    pub fn display(&self) -> String {
        S3PackageUri::from(self).display()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Tag;
    use crate::fixtures;

    type Res<T = ()> = Result<T, UriError>;

    #[test]
    fn test_manifest_uri_try_from_package_uri_with_tag() {
        let package_uri = S3PackageUri {
            bucket: "foo".to_string(),
            namespace: ("bar", "baz").into(),
            revision: RevisionPointer::Tag(Tag::Latest),
            path: None,
            catalog: None,
        };

        let result = ManifestUri::try_from(package_uri);
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            "Invalid package URI: Hash is required for that conversion"
        );
    }

    #[test]
    fn test_manifest_uri_try_from_package_uri_with_hash() -> Res {
        assert_eq!(
            ManifestUri::try_from(S3PackageUri {
                bucket: "test-bucket".to_string(),
                namespace: ("foo", "bar").into(),
                revision: RevisionPointer::Hash(fixtures::TOP_HASH.to_string()),
                path: None,
                catalog: None,
            })?,
            ManifestUri {
                bucket: "test-bucket".to_string(),
                namespace: ("foo", "bar").into(),
                hash: fixtures::TOP_HASH.to_string(),
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
                hash: fixtures::TOP_HASH.to_string(),
            }),
            S3Uri {
                bucket: "test-bucket".to_string(),
                key: format!(".quilt/packages/{}", fixtures::TOP_HASH),
                version: None,
            }
        );
    }

    #[test]
    fn test_manifest_uri_display() {
        let uri = ManifestUri {
            origin: None,
            bucket: "test-bucket".to_string(),
            namespace: ("foo", "bar").into(),
            hash: fixtures::TOP_HASH.to_string(),
        };
        assert_eq!(
            uri.to_string(),
            format!(
                "quilt+s3://test-bucket#package=foo/bar@{}",
                fixtures::TOP_HASH
            )
        );
    }

    #[test]
    fn test_manifest_uri_display_method_with_origin() {
        let uri = ManifestUri {
            origin: Some(fixtures::host()),
            bucket: "test-bucket".to_string(),
            namespace: ("foo", "bar").into(),
            hash: fixtures::TOP_HASH.to_string(),
        };
        assert_eq!(
            uri.display(),
            "quilt+s3://test-bucket#package=foo/bar@012345...abcdef&catalog=quilt.test"
        );
    }

    #[test]
    fn test_manifest_uri_serde_round_trip() {
        let uri = ManifestUri {
            origin: Some(fixtures::host()),
            bucket: "test-bucket".to_string(),
            namespace: ("foo", "bar").into(),
            hash: fixtures::TOP_HASH.to_string(),
        };
        let json = serde_json::to_string(&uri).unwrap();
        let parsed: ManifestUri = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, uri);
    }

    /// Deserializing applies the parser's bucket and top hash rules.
    #[test]
    fn a_deserialized_manifest_uri_gets_the_parser_checks() {
        let good = ManifestUri {
            origin: None,
            bucket: "test-bucket".to_string(),
            namespace: ("foo", "bar").into(),
            hash: fixtures::TOP_HASH.to_string(),
        };
        for uri in [
            ManifestUri {
                bucket: "../../x".to_string(),
                ..good.clone()
            },
            ManifestUri {
                bucket: "a b".to_string(),
                ..good.clone()
            },
            ManifestUri {
                hash: "../../y".to_string(),
                ..good.clone()
            },
            ManifestUri {
                hash: "abc123".to_string(),
                ..good.clone()
            },
        ] {
            let json = serde_json::to_string(&uri).unwrap();
            assert!(
                serde_json::from_str::<ManifestUri>(&json).is_err(),
                "{json} should not deserialize"
            );
        }
    }

    /// A lineage holds an empty hash for a remote that is set but not pushed
    /// yet, so that still reads.
    #[test]
    fn a_manifest_uri_not_pushed_yet_reads() {
        let uri = ManifestUri {
            origin: None,
            bucket: "test-bucket".to_string(),
            namespace: ("foo", "bar").into(),
            hash: String::new(),
        };
        let json = serde_json::to_string(&uri).unwrap();
        assert_eq!(serde_json::from_str::<ManifestUri>(&json).unwrap(), uri);
    }

    /// The JSON form is what lineage files hold, so it stays as is.
    #[test]
    fn a_manifest_uri_keeps_its_json_form() {
        let json = format!(
            r#"{{"origin":"quilt.test","bucket":"test-bucket","namespace":"foo/bar","hash":"{}"}}"#,
            fixtures::TOP_HASH
        );
        let uri = ManifestUri {
            origin: Some(fixtures::host()),
            bucket: "test-bucket".to_string(),
            namespace: ("foo", "bar").into(),
            hash: fixtures::TOP_HASH.to_string(),
        };
        assert_eq!(serde_json::to_string(&uri).unwrap(), json);
        assert_eq!(serde_json::from_str::<ManifestUri>(&json).unwrap(), uri);
    }
}
