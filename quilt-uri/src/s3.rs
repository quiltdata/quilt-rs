use std::fmt;

use url::Url;

use crate::Host;
use crate::UriError;

/// A bucket name under S3's legacy rule: 1 to 255 of `A-Z a-z 0-9 . _ -`,
/// and not `.` or `..`. Every name S3 allows passes, and each one is also a
/// valid URL host.
///
/// # Errors
///
/// Returns [`UriError::Bucket`] for any other name.
pub fn check_bucket(bucket: &str) -> Result<(), UriError> {
    let valid = (1..=255).contains(&bucket.len())
        && bucket != "."
        && bucket != ".."
        && bucket
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'));
    if valid {
        Ok(())
    } else {
        Err(UriError::Bucket(format!(
            "bucket must be 1 to 255 letters, digits, `.`, `_` or `-`, and not `.` or `..`; got {bucket:?}"
        )))
    }
}

/// struct representation of the generic `s3://url`
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(Default))]
pub struct S3Uri {
    pub bucket: String,
    pub key: String,
    pub version: Option<String>,
}

impl S3Uri {
    /// Renders this S3 URI as a catalog web URL on `host`.
    ///
    /// # Errors
    ///
    /// Returns [`UriError::S3`] if the bucket or a piece of the key is `.` or
    /// `..`, which no catalog URL can keep.
    pub fn display_for_host(&self, host: &Host) -> Result<url::Url, UriError> {
        let segments = ["b", &self.bucket, "tree"]
            .into_iter()
            .chain(self.key.split('/'));
        let mut url = host.web_url(segments).map_err(UriError::S3)?;
        if let Some(ref version) = self.version {
            url.query_pairs_mut().append_pair("version", version);
        }
        Ok(url)
    }
}

impl TryFrom<&str> for S3Uri {
    type Error = UriError;

    fn try_from(input: &str) -> Result<Self, Self::Error> {
        let parsed_url = Url::parse(input)?;
        if parsed_url.scheme() != "s3" {
            return Err(UriError::Scheme(format!(
                "Expected s3:// scheme in {input}"
            )));
        }
        let bucket = parsed_url.host_str().unwrap_or_default();
        if !parsed_url.username().is_empty() || parsed_url.password().is_some() {
            return Err(UriError::S3(format!(
                "A user or password is not allowed before the bucket {bucket:?}"
            )));
        }
        if let Some(port) = parsed_url.port() {
            return Err(UriError::S3(format!(
                "A port is not allowed after the bucket {bucket:?}; got {port}"
            )));
        }
        if parsed_url.fragment().is_some() {
            return Err(UriError::S3(format!(
                "A fragment is not allowed in {input}; write `#` in a key as `%23`"
            )));
        }
        let bucket = parsed_url
            .host_str()
            .ok_or(UriError::S3(format!("Missing bucket in {input}")))?;
        check_bucket(bucket)?;

        let path = parsed_url
            .path()
            .strip_prefix('/')
            .filter(|rest| !rest.is_empty())
            .ok_or_else(|| UriError::S3(format!("Missing object key in {input}")))?;

        let key = percent_encoding::percent_decode_str(path).decode_utf8()?;
        let queries = parsed_url.query_pairs().into_owned().collect::<Vec<_>>();
        if queries.len() > 1 {
            return Err(UriError::S3(format!(
                "Too many query parameters in {input}. Only single versionId is allowed"
            )));
        }

        let version = match queries.into_iter().next() {
            None => None,
            Some((key, value)) if key == "versionId" => Some(value),
            Some(_) => {
                return Err(UriError::S3(format!(
                    "Unknown query parameter in {input}. Only single versionId is allowed"
                )));
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
        // A bucket the parser would refuse is written escaped, so it is never
        // read as a user, port or path.
        let bucket = if check_bucket(&self.bucket).is_ok() {
            self.bucket.clone()
        } else {
            percent_encoding::utf8_percent_encode(&self.bucket, percent_encoding::NON_ALPHANUMERIC)
                .to_string()
        };
        let mut remote_url = Url::parse("s3://").map_err(|_| fmt::Error)?;
        remote_url.set_host(Some(&bucket)).map_err(|_| fmt::Error)?;
        remote_url.set_path(&self.key);
        if let Some(version_id) = &self.version {
            remote_url
                .query_pairs_mut()
                .append_pair("versionId", version_id);
        }
        write!(f, "{remote_url}")
    }
}

impl std::str::FromStr for S3Uri {
    type Err = UriError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        S3Uri::try_from(input)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures;

    use test_log::test;

    type Res<T = ()> = Result<T, UriError>;

    #[test]
    fn test_incorrect_scheme() {
        let uri = S3Uri::try_from("https://bucket/foo/bar");
        assert_eq!(
            uri.unwrap_err().to_string(),
            "Invalid URI scheme: Expected s3:// scheme in https://bucket/foo/bar".to_string(),
        );
    }

    #[test]
    fn test_no_bucket() {
        let uri = S3Uri::try_from("s3://");
        assert_eq!(
            uri.unwrap_err().to_string(),
            "Invalid S3 URI: Missing bucket in s3://".to_string(),
        );
    }

    #[test]
    fn test_no_path() {
        let uri = S3Uri::try_from("s3://bucket");
        assert_eq!(
            uri.unwrap_err().to_string(),
            "Invalid S3 URI: Missing object key in s3://bucket".to_string(),
        );
    }

    #[test]
    fn test_no_path_trailing_slash() {
        let uri = S3Uri::try_from("s3://bucket/");
        assert_eq!(
            uri.unwrap_err().to_string(),
            "Invalid S3 URI: Missing object key in s3://bucket/".to_string(),
        );
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
    fn test_incorrect_query() {
        let uri = S3Uri::try_from("s3://bucket/foo/bar?another=query");
        assert_eq!(
            uri.unwrap_err().to_string(),
            "Invalid S3 URI: Unknown query parameter in s3://bucket/foo/bar?another=query. Only single versionId is allowed".to_string(),
        );
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
    fn test_multiple_version_id() {
        let uri = S3Uri::try_from("s3://bucket/foo  bar?versionId=query&versionId=another");
        assert_eq!(
            uri.unwrap_err().to_string(),
            "Invalid S3 URI: Too many query parameters in s3://bucket/foo  bar?versionId=query&versionId=another. Only single versionId is allowed"
                .to_string(),
        );
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
        let host = fixtures::host();
        let uri = S3Uri {
            bucket: "bucket".to_string(),
            key: "foo/bar".to_string(),
            version: None,
        };
        assert_eq!(
            uri.display_for_host(&host)?.as_str(),
            "https://quilt.test/b/bucket/tree/foo/bar"
        );

        let uri_with_version = S3Uri {
            bucket: "bucket".to_string(),
            key: "foo/bar".to_string(),
            version: Some("abc".to_string()),
        };
        assert_eq!(
            uri_with_version.display_for_host(&host)?.as_str(),
            "https://quilt.test/b/bucket/tree/foo/bar?version=abc"
        );
        Ok(())
    }

    /// Every part of the address is read or refused: a user, a password, a
    /// port or a fragment is an error, not dropped.
    #[test]
    fn a_uri_refuses_parts_it_would_drop() {
        for input in [
            "s3://bucket@other-bucket/key",
            "s3://user:pass@bucket/key",
            "s3://:pass@bucket/key",
            "s3://bucket:1/key",
            "s3://bucket/a#b",
            "s3://bucket/a#",
            "s3://bucket/a?versionId=v#b",
        ] {
            let err = S3Uri::try_from(input).expect_err(&format!("{input} should not parse"));
            assert!(
                matches!(err, UriError::S3(_)),
                "{input}: expected an S3 URI error, got {err}"
            );
        }
    }

    /// A key holding `#` is written `%23`, and reads back whole.
    #[test]
    fn a_key_holding_a_hash_sign_round_trips() -> Res {
        let uri = S3Uri::try_from("s3://bucket/a%23b")?;
        assert_eq!(uri.key, "a#b");
        assert_eq!(uri.to_string(), "s3://bucket/a%23b");
        Ok(())
    }

    /// The fields are public, so a bucket the parser would refuse can be set
    /// by hand; the address still renders, with the bucket escaped.
    #[test]
    fn a_hand_built_bad_bucket_still_renders() {
        for (bucket, expected) in [
            ("a b", "s3://a%20b/key"),
            ("a/b", "s3://a%2Fb/key"),
            ("a#b", "s3://a%23b/key"),
            ("a:b", "s3://a%3Ab/key"),
            ("", "s3:///key"),
        ] {
            let uri = S3Uri {
                bucket: bucket.to_string(),
                key: "key".to_string(),
                version: None,
            };
            assert_eq!(uri.to_string(), expected, "{bucket:?}");
        }
    }

    /// Each piece of the key is one path segment of the catalog URL, so `?`,
    /// `#`, `%` and spaces stay in the key.
    #[test]
    fn a_catalog_url_keeps_every_key_character_in_its_path() -> Res {
        let host = fixtures::host();
        for (key, version, expected) in [
            (
                "a?x=1#frag/b c%2F.csv",
                None,
                "https://quilt.test/b/bucket/tree/a%3Fx=1%23frag/b%20c%252F.csv",
            ),
            (
                "dir/a?x.csv",
                Some("v1"),
                "https://quilt.test/b/bucket/tree/dir/a%3Fx.csv?version=v1",
            ),
            ("dir/", None, "https://quilt.test/b/bucket/tree/dir/"),
            ("a..b/.x", None, "https://quilt.test/b/bucket/tree/a..b/.x"),
        ] {
            let uri = S3Uri {
                bucket: "bucket".to_string(),
                key: key.to_string(),
                version: version.map(String::from),
            };
            let url = uri.display_for_host(&host)?;
            assert_eq!(url.as_str(), expected, "{key:?}");
            assert!(url.path().starts_with("/b/bucket/tree/"), "{url}");
            assert_eq!(url.fragment(), None, "{url}");
        }
        Ok(())
    }

    /// A `.` or `..` segment has no catalog URL a browser keeps, so it is an
    /// error.
    #[test]
    fn a_catalog_url_refuses_a_dot_segment() {
        let host = fixtures::host();
        for (bucket, key) in [
            ("bucket", "../../../admin?x=1#frag"),
            ("bucket", "a/./b"),
            ("bucket", "a/.."),
            ("..", "key"),
        ] {
            let uri = S3Uri {
                bucket: bucket.to_string(),
                key: key.to_string(),
                version: None,
            };
            let err = uri
                .display_for_host(&host)
                .expect_err(&format!("{bucket:?} {key:?} should not render"));
            assert!(matches!(err, UriError::S3(_)), "{key:?}: got {err}");
        }
    }

    /// The error for a user or password names the bucket, never the user or
    /// password themselves.
    #[test]
    fn the_error_for_a_user_does_not_repeat_it() {
        for input in ["s3://KEY:secret@bucket/k", "s3://user:secret@bucket:1/k"] {
            let err = S3Uri::try_from(input).expect_err(input).to_string();
            assert!(err.contains("bucket"), "{err}");
            assert!(!err.contains("secret"), "{err}");
            assert!(!err.contains("KEY") && !err.contains("user:"), "{err}");
        }
    }

    /// The parser refuses a bucket name S3 never allowed.
    #[test]
    fn a_uri_refuses_a_bucket_name_s3_does_not_allow() {
        for bucket in ["..", "Bad_Bucket!", "a%20b"] {
            let input = format!("s3://{bucket}/key");
            let err =
                S3Uri::try_from(input.as_str()).expect_err(&format!("{bucket} should not parse"));
            assert!(
                matches!(err, UriError::Bucket(_)),
                "{bucket}: expected a bucket error, got {err}"
            );
        }
    }
}
