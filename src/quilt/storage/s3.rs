use aws_sdk_s3::primitives::ByteStream;
use tokio::io::AsyncReadExt;
use url::Url;

use crate::Error;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct S3Uri {
    pub bucket: String,
    pub key: String,
    pub version: Option<String>,
}

impl S3Uri {
    pub async fn get_contents(&self) -> Result<String, Error> {
        get_object_contents(self).await
    }

    pub async fn put_contents(&self, contents: impl Into<ByteStream>) -> Result<(), Error> {
        put_object_contents(self, contents).await
    }
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

impl std::str::FromStr for S3Uri {
    type Err = Error;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        S3Uri::try_from(input)
    }
}

pub async fn get_object_bytes(uri: &S3Uri) -> Result<Vec<u8>, Error> {
    // real impl
    let client = crate::s3_utils::get_client_for_bucket(&uri.bucket).await?;

    let result = client.get_object().bucket(&uri.bucket).key(&uri.key);

    let result = match &uri.version {
        Some(version) => result.version_id(version),
        None => result,
    };

    let result = result
        .send()
        .await
        .map_err(|err| Error::S3(err.to_string()))?;

    let mut contents = Vec::new();

    result
        .body
        .into_async_read()
        .read_to_end(&mut contents)
        .await?;

    Ok(contents)

    // TODO: fake impl
}

pub async fn get_object_contents(uri: &S3Uri) -> Result<String, Error> {
    let bytes = get_object_bytes(uri).await?;
    String::from_utf8(bytes).map_err(|err| Error::Utf8(err.utf8_error()))
}

pub async fn put_object_contents(
    uri: &S3Uri,
    contents: impl Into<ByteStream>,
) -> Result<(), Error> {
    let client = crate::s3_utils::get_client_for_bucket(&uri.bucket).await?;
    client
        .put_object()
        .bucket(&uri.bucket)
        .key(&uri.key)
        .body(contents.into())
        .send()
        .await
        .map_err(|err| Error::S3(err.to_string()))?;

    Ok(())
}

// pub type MemoryBuckets = HashMap<String, MemoryFS>;
//
// pub struct FakeS3Storage<'a> {
//     buckets: &'a MemoryBuckets,
// }
//
// async fn get_object_contents(uri: &S3Uri) -> Result<String, Error> {
//     // TODO: support versioning?
//     self.buckets.get(&uri.bucket).ok_or("bucket not found")?.get(&uri.key).ok_or(String::from("key not found")).cloned()
// }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_incorrect_scheme() -> Result<(), Error> {
        let uri = S3Uri::try_from("https://bucket/foo/bar");
        assert_eq!(
            uri.unwrap_err().to_string(),
            "Invalid URI scheme: Expected s3:// scheme".to_string(),
        );
        Ok(())
    }

    #[test]
    fn test_no_bucket() -> Result<(), Error> {
        let uri = S3Uri::try_from("s3://");
        assert_eq!(
            uri.unwrap_err().to_string(),
            "Invalid S3 URI: missing bucket".to_string(),
        );
        Ok(())
    }

    #[test]
    fn test_unversioned_uri() -> Result<(), Error> {
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
    fn test_versioned() -> Result<(), Error> {
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
    fn test_incorrect_query() -> Result<(), Error> {
        let uri = S3Uri::try_from("s3://bucket/foo/bar?another=query");
        assert_eq!(
            uri.unwrap_err().to_string(),
            "Invalid S3 URI: Unknown query parameter. Only single versionId is allowed".to_string(),
        );
        Ok(())
    }

    #[test]
    fn test_spaces_in_path() -> Result<(), Error> {
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
    fn test_multiple_version_id() -> Result<(), Error> {
        let uri = S3Uri::try_from("s3://bucket/foo  bar?versionId=query&versionId=another");
        assert_eq!(
            uri.unwrap_err().to_string(),
            "Invalid S3 URI: Too many query parameters. Only single versionId is allowed"
                .to_string(),
        );
        Ok(())
    }

    #[test]
    fn test_implicit_parsing() -> Result<(), Error> {
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
