use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use url::{form_urlencoded, Url};

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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct S3PackageURI {
    pub bucket: String,
    pub namespace: String,
    pub revision: RevisionPointer,
    pub path: Option<String>,
}

impl TryFrom<&str> for S3PackageURI {
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
            Some((namespace, top_hash)) => (
                namespace.to_string(),
                RevisionPointer::Hash(top_hash.into()),
            ),
            None => (pkg_spec, RevisionPointer::default()),
        };

        let path = params.remove("path");

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
            namespace,
            path,
            revision,
        })
    }
}

impl std::str::FromStr for S3PackageURI {
    type Err = Error;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        S3PackageURI::try_from(input)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_implicit_str_parsing() -> Result<(), Error> {
        let uri: S3PackageURI = "quilt+s3://bucket#package=foo/bar@latest".parse()?;
        assert_eq!(
            uri,
            S3PackageURI {
                bucket: "bucket".to_string(),
                namespace: "foo/bar".to_string(),
                revision: RevisionPointer::Hash("latest".to_string()),
                path: None,
            }
        );
        Ok(())
    }

    #[test]
    fn test_implicit_string_parsing() -> Result<(), Error> {
        let uri: S3PackageURI = "quilt+s3://bucket#package=foo/bar@latest"
            .to_string()
            .parse()?;
        assert_eq!(
            uri,
            S3PackageURI {
                bucket: "bucket".to_string(),
                namespace: "foo/bar".to_string(),
                revision: RevisionPointer::Hash("latest".to_string()),
                path: None,
            }
        );
        Ok(())
    }

    #[test]
    fn test_incorrect_scheme() -> Result<(), Error> {
        let uri = S3PackageURI::try_from("s3://bucket#packagefoo/bar");
        assert_eq!(
            uri.unwrap_err().to_string(),
            "Invalid package URI: expected quilt+s3, got s3".to_string(),
        );
        Ok(())
    }

    #[test]
    fn test_no_fragment() -> Result<(), Error> {
        let uri = S3PackageURI::try_from("quilt+s3://bucket");
        assert_eq!(
            uri.unwrap_err().to_string(),
            "Invalid package URI: S3 package URI must contain a fragment: quilt+s3://bucket"
                .to_string(),
        );
        Ok(())
    }

    #[test]
    fn test_no_package() -> Result<(), Error> {
        let uri = S3PackageURI::try_from("quilt+s3://bucket#foo=bar");
        assert_eq!(
            uri.unwrap_err().to_string(),
            "Invalid package URI: missing package in fragment".to_string(),
        );
        Ok(())
    }

    #[test]
    fn test_unknown_paramter() -> Result<(), Error> {
        let uri = S3PackageURI::try_from("quilt+s3://bucket#package=a/b&foo=bar");
        assert_eq!(
            uri.unwrap_err().to_string(),
            r#"Invalid package URI: unexpected parameters in fragment: {"foo": "bar"}"#.to_string(),
        );
        Ok(())
    }

    #[test]
    fn test_no_bucket() -> Result<(), Error> {
        let uri = S3PackageURI::try_from("quilt+s3://#package=a/b");
        assert_eq!(
            uri.unwrap_err().to_string(),
            r#"Invalid package URI: expected host in S3 package URI, got "#.to_string(),
        );
        Ok(())
    }

    #[test]
    fn test_path() -> Result<(), Error> {
        let uri: S3PackageURI =
            "quilt+s3://bucket#package=foo/bar@latest&path=read/me.md".parse()?;
        assert_eq!(
            uri,
            S3PackageURI {
                bucket: "bucket".to_string(),
                namespace: "foo/bar".to_string(),
                revision: RevisionPointer::Hash("latest".to_string()),
                path: Some("read/me.md".to_string()),
            }
        );
        Ok(())
    }

    #[test]
    fn test_latest() -> Result<(), Error> {
        let uri: S3PackageURI = "quilt+s3://bucket#package=foo/bar&path=read/me.md".parse()?;
        assert_eq!(
            uri,
            S3PackageURI {
                bucket: "bucket".to_string(),
                namespace: "foo/bar".to_string(),
                revision: RevisionPointer::Tag("latest".to_string()),
                path: Some("read/me.md".to_string()),
            }
        );
        Ok(())
    }
}
