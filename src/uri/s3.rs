use std::fmt;
use std::str::Chars;

use url::Url;

const DEFAULT_QUILT_HOST: &str = "https://open.quilt.bio";

use crate::uri::Host;
use crate::Error;

fn head_str(mut chars: Chars<'_>) -> (Option<char>, &str) {
    let leading_char = chars.next();
    let rest = chars.as_str();
    (leading_char, rest)
}

fn extract_path_relative_to_bucket(path: &str) -> Result<&str, Error> {
    let (leading_char, rest) = head_str(path.chars());

    match leading_char {
        None => {
            return Err(Error::S3Uri("Path does not exist".to_string()));
        }
        Some('/') => (),
        Some(_) => {
            return Err(Error::S3Uri(
                "Expected path starting with slash".to_string(),
            ));
        }
    }

    if rest.is_empty() {
        return Err(Error::S3Uri("Path does not exist".to_string()));
    }

    Ok(rest)
}

/// struct representation of the generic `s3://url`
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct S3Uri {
    pub bucket: String,
    pub key: String,
    pub version: Option<String>,
}

impl S3Uri {
    pub fn display_for_host(&self, host: &Option<Host>) -> String {
        let hostname = match host {
            Some(host_value) => format!("https://{}", host_value),
            None => DEFAULT_QUILT_HOST.to_string(),
        };
        let unversioned_url = format!("{}/b/{}/tree/{}", hostname, self.bucket, self.key);
        match self.version {
            Some(ref version) => unversioned_url + format!("?version={}", version).as_str(),
            None => unversioned_url,
        }
    }
}

impl TryFrom<&str> for S3Uri {
    type Error = Error;

    fn try_from(input: &str) -> Result<Self, Self::Error> {
        let parsed_url = Url::parse(input)?;
        if parsed_url.scheme() != "s3" {
            return Err(Error::InvalidScheme(format!(
                "Expected s3:// scheme in {}",
                input
            )));
        }
        let bucket = parsed_url
            .host_str()
            .ok_or(Error::S3Uri(format!("Missing bucket in {}", input)))?;

        let path = extract_path_relative_to_bucket(parsed_url.path()).map_err(|err| {
            if let Error::S3Uri(msg) = err {
                Error::S3Uri(format!("{} in {}", msg, input))
            } else {
                err
            }
        })?;

        let key = percent_encoding::percent_decode_str(path).decode_utf8()?;
        let queries = parsed_url.query_pairs().into_owned().collect::<Vec<_>>();
        if queries.len() > 1 {
            return Err(Error::S3Uri(format!(
                "Too many query parameters in {}. Only single versionId is allowed",
                input
            )));
        }

        let version = match queries.first() {
            None => None,
            Some((key, value)) => {
                if key == "versionId" {
                    Some(value.to_string())
                } else {
                    return Err(Error::S3Uri(format!(
                        "Unknown query parameter in {}. Only single versionId is allowed",
                        input
                    )));
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
            "Invalid URI scheme: Expected s3:// scheme in https://bucket/foo/bar".to_string(),
        );
        Ok(())
    }

    #[test]
    fn test_no_bucket() -> Res {
        let uri = S3Uri::try_from("s3://");
        assert_eq!(
            uri.unwrap_err().to_string(),
            "Invalid S3 URI: Missing bucket in s3://".to_string(),
        );
        Ok(())
    }

    #[test]
    fn test_no_path() -> Res {
        let uri = S3Uri::try_from("s3://bucket");
        assert_eq!(
            uri.unwrap_err().to_string(),
            "Invalid S3 URI: Path does not exist in s3://bucket".to_string(),
        );
        Ok(())
    }

    #[test]
    fn test_no_path_trailing_slash() -> Res {
        let uri = S3Uri::try_from("s3://bucket/");
        assert_eq!(
            uri.unwrap_err().to_string(),
            "Invalid S3 URI: Path does not exist in s3://bucket/".to_string(),
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
            "Invalid S3 URI: Unknown query parameter in s3://bucket/foo/bar?another=query. Only single versionId is allowed".to_string(),
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
            "Invalid S3 URI: Too many query parameters in s3://bucket/foo  bar?versionId=query&versionId=another. Only single versionId is allowed"
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

    #[test]
    fn test_display_for_host() -> Res {
        let uri = S3Uri {
            bucket: "bucket".to_string(),
            key: "foo/bar".to_string(),
            version: None,
        };
        assert_eq!(
            uri.display_for_host(&None),
            "https://open.quilt.bio/b/bucket/tree/foo/bar"
        );

        let uri_with_version = S3Uri {
            bucket: "bucket".to_string(),
            key: "foo/bar".to_string(),
            version: Some("abc".to_string()),
        };
        assert_eq!(
            uri_with_version.display_for_host(&None),
            "https://open.quilt.bio/b/bucket/tree/foo/bar?version=abc"
        );

        let host = "custom.host".parse()?;
        assert_eq!(
            uri.display_for_host(&Some(host)),
            "https://custom.host/b/bucket/tree/foo/bar"
        );
        Ok(())
    }
}
