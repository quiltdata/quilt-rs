use std::cmp::Ord;
use std::cmp::Ordering;
use std::cmp::PartialOrd;
use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;

use serde::de;
use serde::de::Visitor;
use serde::Deserialize;
use serde::Deserializer;
use serde::Serialize;
use serde::Serializer;
use url::form_urlencoded;
use url::Host;
use url::Url;

use crate::uri::ManifestUri;
use crate::Error;

const LATEST_TAG: &str = "latest";

/// This is the revision (or "hash") of the package.
/// "Package" itself is a handle, but each package has revision.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "_tag", content = "value")]
pub enum RevisionPointer {
    Hash(String),
    Tag(String),
}

impl Default for RevisionPointer {
    fn default() -> Self {
        Self::Tag(String::from(LATEST_TAG))
    }
}

/// In theory namespace is just a string.
/// But in practice we use "prefix/name".
/// For ease of serializing/deserializing and for validation we put it to a struct.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Namespace {
    prefix: String,
    name: String,
}

impl Ord for Namespace {
    fn cmp(&self, other: &Self) -> Ordering {
        match self.prefix.cmp(&other.prefix) {
            Ordering::Equal => self.name.cmp(&other.name),
            Ordering::Less => Ordering::Less,
            Ordering::Greater => Ordering::Greater,
        }
    }
}

impl PartialOrd for Namespace {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for Namespace {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}/{}", self.prefix, self.name)
    }
}

impl From<(String, String)> for Namespace {
    fn from((prefix, name): (String, String)) -> Self {
        Namespace { prefix, name }
    }
}

impl From<(&str, &str)> for Namespace {
    fn from((prefix, name): (&str, &str)) -> Self {
        (prefix.to_string(), name.to_string()).into()
    }
}

impl TryFrom<&str> for Namespace {
    type Error = Error;

    fn try_from(input: &str) -> Result<Self, Self::Error> {
        input
            .split_once('/')
            .ok_or(Error::Namespace("Failed to parse namespace".to_string()))
            .map(|x| x.into())
    }
}

impl TryFrom<String> for Namespace {
    type Error = Error;

    fn try_from(input: String) -> Result<Self, Self::Error> {
        input.as_str().try_into()
    }
}

impl Serialize for Namespace {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

struct NamespaceVisitor;

impl Visitor<'_> for NamespaceVisitor {
    type Value = Namespace;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a string prefix and a string name divided with /")
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Namespace::try_from(value).map_err(|e| E::custom(format!("Failed parse namespace {}", e)))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Namespace::try_from(value).map_err(|e| E::custom(format!("Failed parse namespace {}", e)))
    }
}

impl<'de> Deserialize<'de> for Namespace {
    fn deserialize<D>(deserializer: D) -> Result<Namespace, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_string(NamespaceVisitor)
    }
}

/// This is kinda URI for the package without revisions.
/// You can use it when you don't know or don't care about revision of the package.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct S3PackageHandle {
    pub bucket: String,
    pub namespace: Namespace,
}

impl From<S3PackageUri> for S3PackageHandle {
    fn from(uri: S3PackageUri) -> S3PackageHandle {
        S3PackageHandle {
            bucket: uri.bucket,
            namespace: uri.namespace,
        }
    }
}

impl From<&S3PackageUri> for S3PackageHandle {
    fn from(uri: &S3PackageUri) -> S3PackageHandle {
        uri.clone().into()
    }
}

impl From<ManifestUri> for S3PackageHandle {
    fn from(uri: ManifestUri) -> S3PackageHandle {
        S3PackageHandle {
            bucket: uri.bucket,
            namespace: uri.namespace,
        }
    }
}

impl From<&ManifestUri> for S3PackageHandle {
    fn from(uri: &ManifestUri) -> S3PackageHandle {
        uri.clone().into()
    }
}

/// Struct representation of the general `quilt+s3://url`
/// Package handle + revision is a package.
/// Also, this URI has path, so you can use it as an URI for referencing files in package.
/// You can use this URL for both packages and files in packages.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct S3PackageUri {
    pub bucket: String,
    pub namespace: Namespace,
    pub revision: RevisionPointer,
    pub path: Option<PathBuf>,
    pub catalog: Option<Host>,
}

// TODO: consider using S3Uri
impl TryFrom<&str> for S3PackageUri {
    type Error = Error;

    fn try_from(input: &str) -> Result<Self, Self::Error> {
        let parsed_url = Url::parse(input)?;
        if parsed_url.scheme() != "quilt+s3" {
            return Err(Error::PackageURI(format!(
                "expected quilt+s3, got {}",
                parsed_url.scheme()
            )));
        }

        let fragment = parsed_url.fragment().ok_or(Error::PackageURI(format!(
            "S3 package URI must contain a fragment: {}",
            input
        )))?;
        let mut params: HashMap<_, _> = form_urlencoded::parse(fragment.as_bytes())
            .into_owned()
            .collect();

        let pkg_spec = params
            .remove("package")
            .ok_or(Error::PackageURI("missing package in fragment".to_string()))?;

        let (namespace, revision) = match pkg_spec.split_once('@') {
            Some((namespace, top_hash)) => {
                (namespace.into(), RevisionPointer::Hash(top_hash.into()))
            }
            None => (pkg_spec, RevisionPointer::default()),
        };

        let path = params.remove("path").map(PathBuf::from);

        let catalog = params.remove("catalog").map(Host::Domain);

        if !params.is_empty() {
            return Err(Error::PackageURI(format!(
                "unexpected parameters in fragment: {:?}",
                params
            )));
        }

        let bucket = parsed_url.host_str().ok_or(Error::PackageURI(format!(
            "expected host in S3 package URI, got {}",
            parsed_url.host_str().unwrap_or_default()
        )))?;

        Ok(Self {
            bucket: bucket.to_string(),
            catalog,
            namespace: namespace.try_into()?,
            path,
            revision,
        })
    }
}

impl fmt::Display for S3PackageUri {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let hash = match &self.revision {
            RevisionPointer::Tag(h) => {
                if h == "latest" {
                    "".to_string()
                } else {
                    format!("@{}", h)
                }
            }
            RevisionPointer::Hash(h) => format!("@{}", h),
        };
        let path_part = match &self.path {
            Some(p) => format!("&path={}", p.display()),
            None => "".to_string(),
        };
        let catalog_part = match &self.catalog {
            Some(p) => format!("&catalog={}", p),
            None => "".to_string(),
        };
        write!(
            f,
            "quilt+s3://{}#package={}{}{}{}",
            self.bucket, self.namespace, hash, path_part, catalog_part
        )
    }
}

impl std::str::FromStr for S3PackageUri {
    type Err = Error;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        S3PackageUri::try_from(input)
    }
}

impl From<ManifestUri> for S3PackageUri {
    fn from(uri: ManifestUri) -> S3PackageUri {
        S3PackageUri {
            bucket: uri.bucket,
            catalog: None,
            namespace: uri.namespace,
            path: None,
            revision: RevisionPointer::Hash(uri.hash),
        }
    }
}

impl From<&ManifestUri> for S3PackageUri {
    fn from(uri: &ManifestUri) -> S3PackageUri {
        S3PackageUri::from(uri.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::Res;

    #[test]
    fn test_implicit_str_parsing() -> Res {
        let uri: S3PackageUri = "quilt+s3://bucket#package=foo/bar@latest".parse()?;
        assert_eq!(
            uri,
            S3PackageUri {
                bucket: "bucket".to_string(),
                catalog: None,
                namespace: ("foo", "bar").into(),
                revision: RevisionPointer::Hash("latest".to_string()),
                path: None,
            }
        );
        Ok(())
    }

    #[test]
    fn test_implicit_string_parsing() -> Res {
        let uri: S3PackageUri = "quilt+s3://bucket#package=foo/bar@latest"
            .to_string()
            .parse()?;
        assert_eq!(
            uri,
            S3PackageUri {
                bucket: "bucket".to_string(),
                catalog: None,
                namespace: ("foo", "bar").into(),
                revision: RevisionPointer::Hash("latest".to_string()),
                path: None,
            }
        );
        Ok(())
    }

    #[test]
    fn test_incorrect_scheme() -> Res {
        let uri = S3PackageUri::try_from("s3://bucket#packagefoo/bar");
        assert_eq!(
            uri.unwrap_err().to_string(),
            "Invalid package URI: expected quilt+s3, got s3".to_string(),
        );
        Ok(())
    }

    #[test]
    fn test_no_fragment() -> Res {
        let uri = S3PackageUri::try_from("quilt+s3://bucket");
        assert_eq!(
            uri.unwrap_err().to_string(),
            "Invalid package URI: S3 package URI must contain a fragment: quilt+s3://bucket"
                .to_string(),
        );
        Ok(())
    }

    #[test]
    fn test_no_package() -> Res {
        let uri = S3PackageUri::try_from("quilt+s3://bucket#foo=bar");
        assert_eq!(
            uri.unwrap_err().to_string(),
            "Invalid package URI: missing package in fragment".to_string(),
        );
        Ok(())
    }

    #[test]
    fn test_unknown_paramter() -> Res {
        let uri = S3PackageUri::try_from("quilt+s3://bucket#package=a/b&foo=bar");
        assert_eq!(
            uri.unwrap_err().to_string(),
            r#"Invalid package URI: unexpected parameters in fragment: {"foo": "bar"}"#.to_string(),
        );
        Ok(())
    }

    #[test]
    fn test_no_bucket() -> Res {
        let uri = S3PackageUri::try_from("quilt+s3://#package=a/b");
        assert_eq!(
            uri.unwrap_err().to_string(),
            r#"Invalid package URI: expected host in S3 package URI, got "#.to_string(),
        );
        Ok(())
    }

    #[test]
    fn test_path() -> Res {
        let uri: S3PackageUri =
            "quilt+s3://bucket#package=foo/bar@latest&path=read/me.md".parse()?;
        assert_eq!(
            uri,
            S3PackageUri {
                bucket: "bucket".to_string(),
                catalog: None,
                namespace: ("foo", "bar").into(),
                revision: RevisionPointer::Hash("latest".to_string()),
                path: Some(PathBuf::from("read/me.md")),
            }
        );
        Ok(())
    }

    #[test]
    fn test_latest() -> Res {
        let uri: S3PackageUri = "quilt+s3://bucket#package=foo/bar&path=read/me.md".parse()?;
        assert_eq!(
            uri,
            S3PackageUri {
                bucket: "bucket".to_string(),
                catalog: None,
                namespace: ("foo", "bar").into(),
                revision: RevisionPointer::Tag("latest".to_string()),
                path: Some(PathBuf::from("read/me.md")),
            }
        );
        Ok(())
    }

    #[test]
    fn test_catalog() -> Res {
        let uri: S3PackageUri =
            "quilt+s3://bucket#package=foo/bar&path=read/me.md&catalog=do.ma.in".parse()?;
        assert_eq!(
            uri,
            S3PackageUri {
                bucket: "bucket".to_string(),
                catalog: Some(Host::Domain("do.ma.in".to_string())),
                namespace: ("foo", "bar").into(),
                revision: RevisionPointer::Tag("latest".to_string()),
                path: Some(PathBuf::from("read/me.md")),
            }
        );
        Ok(())
    }

    #[test]
    fn test_stringify_with_latest() -> Res {
        let uri = S3PackageUri {
            bucket: "bucket".to_string(),
            catalog: Some(Host::Domain("do.ma.in".to_string())),
            namespace: ("foo", "bar").into(),
            revision: RevisionPointer::Tag("latest".to_string()),
            path: Some(PathBuf::from("read/me.md")),
        };
        assert_eq!(
            uri.to_string(),
            "quilt+s3://bucket#package=foo/bar&path=read/me.md&catalog=do.ma.in"
        );
        Ok(())
    }

    #[test]
    fn test_stringify_with_hash() -> Res {
        let uri = S3PackageUri {
            bucket: "bucket".to_string(),
            catalog: None,
            namespace: ("foo", "bar").into(),
            revision: RevisionPointer::Hash("abc123".to_string()),
            path: None,
        };
        assert_eq!(uri.to_string(), "quilt+s3://bucket#package=foo/bar@abc123");
        Ok(())
    }

    #[test]
    fn test_stringify_with_tag() -> Res {
        let uri = S3PackageUri {
            bucket: "bucket".to_string(),
            catalog: None,
            namespace: ("foo", "bar").into(),
            revision: RevisionPointer::Tag("foobar".to_string()),
            path: None,
        };
        assert_eq!(uri.to_string(), "quilt+s3://bucket#package=foo/bar@foobar");
        Ok(())
    }

    #[test]
    fn test_from_manifest_uri() -> Res {
        let manifest_uri = ManifestUri {
            bucket: "test-bucket".to_string(),
            namespace: ("foo", "bar").into(),
            hash: "abc123".to_string(),
        };

        assert_eq!(
            S3PackageUri::from(manifest_uri),
            S3PackageUri {
                bucket: "test-bucket".to_string(),
                catalog: None,
                namespace: ("foo", "bar").into(),
                path: None,
                revision: RevisionPointer::Hash("abc123".to_string()),
            }
        );
        Ok(())
    }

    #[test]
    fn test_namespace_ordering_greater() -> Res {
        let ns1 = Namespace::from(("z", "a"));
        let ns2 = Namespace::from(("a", "b"));

        assert!(ns1 > ns2);
        assert_eq!(ns1.cmp(&ns2), Ordering::Greater);

        let ns3 = Namespace::from(("same", "z"));
        let ns4 = Namespace::from(("same", "a"));

        assert!(ns3 > ns4);
        assert_eq!(ns3.cmp(&ns4), Ordering::Greater);

        Ok(())
    }

    #[test]
    fn test_namespace_partial_ordering() -> Res {
        let ns1 = Namespace::from(("a", "b"));
        let ns2 = Namespace::from(("a", "b"));
        let ns3 = Namespace::from(("c", "d"));

        // Test equality
        assert!(!(ns1 < ns2));
        assert!(!(ns1 > ns2));
        assert!(ns1 <= ns2);
        assert!(ns1 >= ns2);

        // Test less than
        assert!(ns1 < ns3);
        assert!(ns1 <= ns3);
        assert!(!(ns1 > ns3));
        assert!(!(ns1 >= ns3));

        // Test greater than
        assert!(ns3 > ns1);
        assert!(ns3 >= ns1);
        assert!(!(ns3 < ns1));
        assert!(!(ns3 <= ns1));

        Ok(())
    }
}
