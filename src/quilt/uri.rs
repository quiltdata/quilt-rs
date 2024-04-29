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
use url::Url;

use crate::quilt::manifest_handle::RemoteManifest;
use crate::Error;

const LATEST_TAG: &str = "latest";

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

impl<'de> Visitor<'de> for NamespaceVisitor {
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct S3PackageUri {
    pub bucket: String,
    pub namespace: Namespace,
    pub revision: RevisionPointer,
    pub path: Option<PathBuf>,
}

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
            namespace: namespace.try_into()?,
            path,
            revision,
        })
    }
}

impl std::str::FromStr for S3PackageUri {
    type Err = Error;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        S3PackageUri::try_from(input)
    }
}

impl From<S3PackageUri> for RemoteManifest {
    fn from(uri: S3PackageUri) -> RemoteManifest {
        RemoteManifest {
            bucket: uri.bucket,
            namespace: uri.namespace,
            hash: match uri.revision {
                RevisionPointer::Hash(h) => h,
                RevisionPointer::Tag(h) => h,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_implicit_str_parsing() -> Result<(), Error> {
        let uri: S3PackageUri = "quilt+s3://bucket#package=foo/bar@latest".parse()?;
        assert_eq!(
            uri,
            S3PackageUri {
                bucket: "bucket".to_string(),
                namespace: ("foo", "bar").into(),
                revision: RevisionPointer::Hash("latest".to_string()),
                path: None,
            }
        );
        Ok(())
    }

    #[test]
    fn test_implicit_string_parsing() -> Result<(), Error> {
        let uri: S3PackageUri = "quilt+s3://bucket#package=foo/bar@latest"
            .to_string()
            .parse()?;
        assert_eq!(
            uri,
            S3PackageUri {
                bucket: "bucket".to_string(),
                namespace: ("foo", "bar").into(),
                revision: RevisionPointer::Hash("latest".to_string()),
                path: None,
            }
        );
        Ok(())
    }

    #[test]
    fn test_incorrect_scheme() -> Result<(), Error> {
        let uri = S3PackageUri::try_from("s3://bucket#packagefoo/bar");
        assert_eq!(
            uri.unwrap_err().to_string(),
            "Invalid package URI: expected quilt+s3, got s3".to_string(),
        );
        Ok(())
    }

    #[test]
    fn test_no_fragment() -> Result<(), Error> {
        let uri = S3PackageUri::try_from("quilt+s3://bucket");
        assert_eq!(
            uri.unwrap_err().to_string(),
            "Invalid package URI: S3 package URI must contain a fragment: quilt+s3://bucket"
                .to_string(),
        );
        Ok(())
    }

    #[test]
    fn test_no_package() -> Result<(), Error> {
        let uri = S3PackageUri::try_from("quilt+s3://bucket#foo=bar");
        assert_eq!(
            uri.unwrap_err().to_string(),
            "Invalid package URI: missing package in fragment".to_string(),
        );
        Ok(())
    }

    #[test]
    fn test_unknown_paramter() -> Result<(), Error> {
        let uri = S3PackageUri::try_from("quilt+s3://bucket#package=a/b&foo=bar");
        assert_eq!(
            uri.unwrap_err().to_string(),
            r#"Invalid package URI: unexpected parameters in fragment: {"foo": "bar"}"#.to_string(),
        );
        Ok(())
    }

    #[test]
    fn test_no_bucket() -> Result<(), Error> {
        let uri = S3PackageUri::try_from("quilt+s3://#package=a/b");
        assert_eq!(
            uri.unwrap_err().to_string(),
            r#"Invalid package URI: expected host in S3 package URI, got "#.to_string(),
        );
        Ok(())
    }

    #[test]
    fn test_path() -> Result<(), Error> {
        let uri: S3PackageUri =
            "quilt+s3://bucket#package=foo/bar@latest&path=read/me.md".parse()?;
        assert_eq!(
            uri,
            S3PackageUri {
                bucket: "bucket".to_string(),
                namespace: ("foo", "bar").into(),
                revision: RevisionPointer::Hash("latest".to_string()),
                path: Some(PathBuf::from("read/me.md")),
            }
        );
        Ok(())
    }

    #[test]
    fn test_latest() -> Result<(), Error> {
        let uri: S3PackageUri = "quilt+s3://bucket#package=foo/bar&path=read/me.md".parse()?;
        assert_eq!(
            uri,
            S3PackageUri {
                bucket: "bucket".to_string(),
                namespace: ("foo", "bar").into(),
                revision: RevisionPointer::Tag("latest".to_string()),
                path: Some(PathBuf::from("read/me.md")),
            }
        );
        Ok(())
    }
}
