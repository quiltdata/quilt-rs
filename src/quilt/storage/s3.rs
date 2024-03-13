use aws_sdk_s3::primitives::ByteStream;
use tokio::io::AsyncReadExt;
use url::Url;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct S3Uri {
    pub bucket: String,
    pub key: String,
    pub version: Option<String>,
}

impl S3Uri {
    pub async fn get_contents(&self) -> Result<String, String> {
        get_object_contents(self).await
    }

    pub async fn put_contents(&self, contents: impl Into<ByteStream>) -> Result<(), String> {
        put_object_contents(self, contents).await
    }
}

impl TryFrom<&str> for S3Uri {
    type Error = String;

    fn try_from(input: &str) -> Result<Self, Self::Error> {
        let parsed_url = Url::parse(input).map_err(|err| err.to_string())?;
        if parsed_url.scheme() != "s3" {
            return Err("invalid scheme".into());
        }
        let bucket = parsed_url.host_str().ok_or("missing bucket")?.to_string();
        let key: String = parsed_url.path().chars().skip(1).collect();
        let version = None; // FIXME
        Ok(Self {
            bucket,
            key,
            version,
        })
    }
}

pub async fn get_object_bytes(uri: &S3Uri) -> Result<Vec<u8>, String> {
    // real impl
    let client = crate::s3_utils::get_client_for_bucket(&uri.bucket).await?;

    let result = client.get_object().bucket(&uri.bucket).key(&uri.key);

    let result = match &uri.version {
        Some(version) => result.version_id(version),
        None => result,
    };

    let result = result.send().await.map_err(|err| {
        err.into_service_error()
            .meta()
            .message()
            .unwrap_or("failed to download s3 object")
            .to_string()
    })?;

    let mut contents = Vec::new();

    result
        .body
        .into_async_read()
        .read_to_end(&mut contents)
        .await
        .map_err(|err| err.to_string())?;

    Ok(contents)

    // TODO: fake impl
}

pub async fn get_object_contents(uri: &S3Uri) -> Result<String, String> {
    let bytes = get_object_bytes(uri).await?;
    String::from_utf8(bytes).map_err(|err| err.to_string())
}

pub async fn put_object_contents(
    uri: &S3Uri,
    contents: impl Into<ByteStream>,
) -> Result<(), String> {
    let client = crate::s3_utils::get_client_for_bucket(&uri.bucket).await?;
    client
        .put_object()
        .bucket(&uri.bucket)
        .key(&uri.key)
        .body(contents.into())
        .send()
        .await
        .map_err(|err| {
            err.into_service_error()
                .meta()
                .message()
                .unwrap_or("failed to upload s3 object")
                .to_string()
        })?;

    Ok(())
}

// pub type MemoryBuckets = HashMap<String, MemoryFS>;
//
// pub struct FakeS3Storage<'a> {
//     buckets: &'a MemoryBuckets,
// }
//
// async fn get_object_contents(uri: &S3Uri) -> Result<String, String> {
//     // TODO: support versioning?
//     self.buckets.get(&uri.bucket).ok_or("bucket not found")?.get(&uri.key).ok_or(String::from("key not found")).cloned()
// }
