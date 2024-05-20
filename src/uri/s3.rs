use std::fmt;

use url::Url;

use crate::Error;

/// struct representation of the generic `s3://url`
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct S3Uri {
    pub bucket: String,
    pub key: String,
    pub version: Option<String>,
}

impl TryFrom<&str> for S3Uri {
    type Error = Error;

    fn try_from(input: &str) -> Result<Self, Self::Error> {
        let parsed_url = Url::parse(input)?;
        if parsed_url.scheme() != "s3" {
            return Err(Error::InvalidScheme("Expected s3:// scheme".to_string()));
        }
        let bucket = parsed_url
            .host_str()
            .ok_or(Error::S3Uri("missing bucket".to_string()))?;
        let key = percent_encoding::percent_decode_str(&parsed_url.path()[1..]).decode_utf8()?;
        let queries = parsed_url.query_pairs().into_owned().collect::<Vec<_>>();
        if queries.len() > 1 {
            return Err(Error::S3Uri(
                "Too many query parameters. Only single versionId is allowed".to_string(),
            ));
        }

        let version = match queries.first() {
            None => None,
            Some((key, value)) => {
                if key == "versionId" {
                    Some(value.to_string())
                } else {
                    return Err(Error::S3Uri(
                        "Unknown query parameter. Only single versionId is allowed".to_string(),
                    ));
                }
            }
        };

        Ok(Self {
            bucket: bucket.to_string(),
            key: key.to_string(),
            version,
        })
    }
}

impl fmt::Display for S3Uri {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut remote_url = Url::parse("s3://").unwrap();
        remote_url
            .set_host(Some(&self.bucket))
            .expect("failed to set bucket");
        remote_url.set_path(&self.key);
        if let Some(version_id) = &self.version {
            remote_url
                .query_pairs_mut()
                .append_pair("versionId", version_id);
        };
        write!(f, "{}", remote_url)
    }
}

impl std::str::FromStr for S3Uri {
    type Err = Error;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        S3Uri::try_from(input)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::Res;

    #[test]
    fn test_incorrect_scheme() -> Res {
        let uri = S3Uri::try_from("https://bucket/foo/bar");
        assert_eq!(
            uri.unwrap_err().to_string(),
            "Invalid URI scheme: Expected s3:// scheme".to_string(),
        );
        Ok(())
    }

    #[test]
    fn test_no_bucket() -> Res {
        let uri = S3Uri::try_from("s3://");
        assert_eq!(
            uri.unwrap_err().to_string(),
            "Invalid S3 URI: missing bucket".to_string(),
        );
        Ok(())
    }

    #[test]
    fn test_unversioned_uri() -> Res {
        let uri = S3Uri::try_from("s3://bucket/foo/bar")?;
        assert_eq!(
            uri,
            S3Uri {
                bucket: "bucket".to_string(),
                key: "foo/bar".to_string(),
                version: None,
            }
        );
        Ok(())
    }

    #[test]
    fn test_versioned() -> Res {
        let uri = S3Uri::try_from("s3://bucket/foo/bar?versionId=abc")?;
        assert_eq!(
            uri,
            S3Uri {
                bucket: "bucket".to_string(),
                key: "foo/bar".to_string(),
                version: Some("abc".to_string()),
            }
        );
        Ok(())
    }

    #[test]
    fn test_incorrect_query() -> Res {
        let uri = S3Uri::try_from("s3://bucket/foo/bar?another=query");
        assert_eq!(
            uri.unwrap_err().to_string(),
            "Invalid S3 URI: Unknown query parameter. Only single versionId is allowed".to_string(),
        );
        Ok(())
    }

    #[test]
    fn test_spaces_in_path() -> Res {
        let uri = S3Uri::try_from("s3://bucket/foo  bar?versionId=abc")?;
        assert_eq!(
            uri,
            S3Uri {
                bucket: "bucket".to_string(),
                key: "foo  bar".to_string(),
                version: Some("abc".to_string()),
            }
        );
        Ok(())
    }

    #[test]
    fn test_multiple_version_id() -> Res {
        let uri = S3Uri::try_from("s3://bucket/foo  bar?versionId=query&versionId=another");
        assert_eq!(
            uri.unwrap_err().to_string(),
            "Invalid S3 URI: Too many query parameters. Only single versionId is allowed"
                .to_string(),
        );
        Ok(())
    }

    #[test]
    fn test_implicit_parsing() -> Res {
        let uri: S3Uri = "s3://bucket/foo/bar?versionId=abc".parse()?;
        assert_eq!(
            uri,
            S3Uri {
                bucket: "bucket".to_string(),
                key: "foo/bar".to_string(),
                version: Some("abc".to_string()),
            }
        );
        Ok(())
    }
}
