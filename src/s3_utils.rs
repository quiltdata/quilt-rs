use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::sync::RwLock;

use aws_config::BehaviorVersion;
use aws_sdk_s3::error::DisplayErrorContext;
use aws_sdk_s3::error::SdkError;
use aws_sdk_s3::operation::get_object_attributes::GetObjectAttributesOutput;
use aws_types::region::Region;
use base64::prelude::BASE64_STANDARD;
use base64::Engine;
use lazy_static::lazy_static;
use multihash::Multihash;
use parquet::data_type::AsBytes;
use sha2::Digest;
use sha2::Sha256;
use tokio::io::AsyncRead;
use tracing::log;

use crate::quilt::manifest::MULTIHASH_SHA256_CHUNKED;
use crate::quilt::remote::Remote;
use crate::quilt::s3;
use crate::quilt4::checksum::get_checksum_chunksize_and_parts;
use crate::Error;

pub async fn find_bucket_region(client: &reqwest::Client, bucket: &str) -> Result<String, Error> {
    let response = client
        .head(format!("https://s3.amazonaws.com/{bucket}"))
        .send()
        .await?;

    match response.headers().get("x-amz-bucket-region") {
        Some(location) => Ok(location.to_str()?.into()),
        None => Err(Error::MissingHTTPHeader("x-amz-bucket-region".to_string())),
    }
}

pub async fn get_object(
    client: &aws_sdk_s3::Client,
    bucket: &str,
    key: &str,
) -> Result<impl AsyncRead, Error> {
    let result = client
        .get_object()
        .bucket(bucket)
        .key(key)
        .send()
        .await
        .map_err(|err| Error::S3(DisplayErrorContext(err).to_string()))?;
    let contents = result.body.into_async_read();
    Ok(contents)
}

lazy_static! {
    static ref HTTP_CLIENT: reqwest::Client = reqwest::Client::new();
    static ref BUCKET_REGIONS: RwLock<HashMap<String, Region>> = RwLock::new(HashMap::new());
    static ref REGION_CLIENTS: RwLock<HashMap<Region, aws_sdk_s3::Client>> =
        RwLock::new(HashMap::new());
}

pub async fn get_region_for_bucket(bucket: &str) -> Result<Region, Error> {
    {
        let map = BUCKET_REGIONS.read().unwrap();
        if let Some(region) = map.get(bucket) {
            return Ok(region.clone());
        }
    }

    let region = find_bucket_region(&HTTP_CLIENT, bucket).await?;

    let mut map = BUCKET_REGIONS.write().unwrap();
    match map.entry(bucket.to_owned()) {
        Entry::Occupied(entry) => Ok(entry.get().clone()),
        Entry::Vacant(entry) => Ok(entry.insert(Region::new(region)).clone()),
    }
}

pub async fn get_client_for_region(region: aws_types::region::Region) -> aws_sdk_s3::Client {
    {
        let map = REGION_CLIENTS.read().unwrap();
        if let Some(client) = map.get(&region) {
            return client.clone();
        }
    }

    let config = aws_config::defaults(BehaviorVersion::latest())
        .region(region.clone())
        .load()
        .await;
    let client = aws_sdk_s3::Client::new(&config);

    let mut map = REGION_CLIENTS.write().unwrap();

    match map.entry(region) {
        Entry::Occupied(entry) => entry.get().clone(),
        Entry::Vacant(entry) => entry.insert(client).clone(),
    }
}

pub async fn get_client_for_bucket(bucket: &str) -> Result<aws_sdk_s3::Client, Error> {
    let region = get_region_for_bucket(bucket).await?.clone();
    Ok(get_client_for_region(region).await)
}

pub fn get_compliant_chunked_checksum(attrs: &GetObjectAttributesOutput) -> Option<Vec<u8>> {
    let checksum = attrs.checksum.as_ref()?;
    let checksum_sha256 = checksum.checksum_sha256.as_ref()?;
    // XXX: defer decoding until we know it's compatible?
    let checksum_sha256_decoded = BASE64_STANDARD
        .decode(checksum_sha256.as_bytes())
        .expect("AWS checksum must be valid base64");
    let object_size = attrs.object_size.expect("ObjectSize must be requested");
    if (object_size as u64) < s3::MULTIPART_THRESHOLD {
        if let Some(object_parts) = &attrs.object_parts {
            if object_parts
                .total_parts_count
                .expect("ObjectParts is expected to have TotalParts")
                == 1
            {
                return Some(checksum_sha256_decoded);
            }
        }
        return Some(Sha256::digest(checksum_sha256_decoded).as_slice().into());
    } else if let Some(object_parts) = &attrs.object_parts {
        let parts = object_parts.parts();
        // Make sure we requested all parts.
        assert_eq!(
            parts.len(),
            object_parts
                .total_parts_count
                .expect("ObjectParts is expected to have TotalParts") as usize
        );
        let expected_chunk_size = get_checksum_chunksize_and_parts(object_size as u64).0;
        if parts[..parts.len() - 1]
            .iter()
            .all(|p| p.size.expect("Part is expected to have size") as u64 == expected_chunk_size)
        {
            return Some(checksum_sha256_decoded);
        }
    }
    None
}

pub struct S3Attributes {
    pub key: String,
    pub hash: Multihash<256>,
    pub size: u64,
    pub version_id: Option<String>,
}

pub async fn get_attrs_for_key(
    client: aws_sdk_s3::Client,
    bucket: &str,
    key: &str,
) -> Result<S3Attributes, Error> {
    log::debug!("Getting attributes for bucket {} key {}", bucket, key);
    let attr_result = client
        .get_object_attributes()
        .bucket(bucket)
        .key(key)
        .object_attributes(aws_sdk_s3::types::ObjectAttributes::Checksum)
        .object_attributes(aws_sdk_s3::types::ObjectAttributes::ObjectParts)
        .object_attributes(aws_sdk_s3::types::ObjectAttributes::ObjectSize)
        .max_parts(s3::MPU_MAX_PARTS as i32)
        .send()
        .await;
    let attrs = match attr_result {
        Ok(attrs) => attrs,
        Err(e) => {
            log::warn!("Error getting attributes: {}", DisplayErrorContext(e));
            return calculate_attrs_for_key(client, bucket, key).await;
        }
    };

    log::debug!("Got attributes: {:?}", attrs);
    match attrs.delete_marker {
        // Can happen if object is removed after it was listed but before attributes retrieved.
        Some(true) => Err(Error::S3("Object is a delete marker".to_string())),
        _ => {
            let checksum = get_compliant_chunked_checksum(&attrs).unwrap();
            let hash = Multihash::wrap(MULTIHASH_SHA256_CHUNKED, checksum.as_bytes())?;
            let size = attrs.object_size.expect("ObjectSize must be requested") as u64;
            Ok(S3Attributes {
                key: key.to_string(),
                hash,
                size,
                version_id: Some(attrs.version_id.expect("VersionId must be requested")),
            })
        }
    }
}

pub async fn calculate_attrs_for_key(
    client: aws_sdk_s3::Client,
    bucket: &str,
    key: &str,
) -> Result<S3Attributes, Error> {
    log::debug!("Trying again with client {:?}", client);
    Err(Error::S3(format!(
        "Error getting attributes for s3://{}/{}",
        bucket, key,
    )))
}

#[derive(Clone)]
pub struct RemoteS3 {}

impl RemoteS3 {
    pub fn new() -> Self {
        RemoteS3 {}
    }
}

impl Remote for RemoteS3 {
    async fn get_object(&self, bucket: &str, key: &str) -> Result<impl AsyncRead + Send + Unpin, Error> {
        let client = get_client_for_bucket(bucket).await?;
        get_object(&client, &bucket, &key).await
    }

    async fn exists(&self, bucket: &str, key: &str) -> Result<bool, Error> {
        let client = get_client_for_bucket(bucket).await?;
        match client.head_object().bucket(bucket).key(key).send().await {
            Ok(_) => Ok(true),
            Err(SdkError::ServiceError(err)) if err.err().is_not_found() => Ok(false),
            Err(err) => Err(Error::S3(DisplayErrorContext(err).to_string())),
        }
    }
}

#[cfg(test)]
mod tests {

    use aws_sdk_s3::types::Checksum;
    use aws_sdk_s3::types::GetObjectAttributesParts;
    use aws_sdk_s3::types::ObjectPart;

    use super::*;

    #[test]
    fn test_get_compliant_chunked_checksum() {
        fn b64decode(data: &str) -> Vec<u8> {
            BASE64_STANDARD.decode(data.as_bytes()).unwrap()
        }

        fn sha256(data: Vec<u8>) -> Vec<u8> {
            Sha256::digest(data).as_slice().into()
        }

        let builder = GetObjectAttributesOutput::builder;
        let test_data = [
            (builder(), None),
            (
                builder().checksum(
                    Checksum::builder()
                        .checksum_sha1("X94czmA+ZWbSDagRyci8zLBE1K4=")
                        .build(),
                ),
                None,
            ),
            (
                builder()
                    .checksum(
                        Checksum::builder()
                            .checksum_sha256("MOFJVevxNSJm3C/4Bn5oEEYH51CrudOzZYK4r5Cfy1g=")
                            .build(),
                    )
                    .object_size(1048576), // below the threshold
                Some(sha256(b64decode(
                    "MOFJVevxNSJm3C/4Bn5oEEYH51CrudOzZYK4r5Cfy1g=",
                ))),
            ),
            (
                builder()
                    .checksum(
                        Checksum::builder()
                            .checksum_sha256("vWr41JZ9PL656FAGy906ysrYj/8ccoMUWHT0xEXRftA=")
                            .build(),
                    )
                    .object_parts(
                        GetObjectAttributesParts::builder()
                            .total_parts_count(1)
                            .parts(
                                ObjectPart::builder()
                                    .size(5242880)
                                    .checksum_sha256("wDbLt1U6kJ+LiHfURhkkMH8n7LZs/5KO7q/VacOIfik=")
                                    .build(),
                            )
                            .build(),
                    )
                    .object_size(5242880), // below the threshold
                Some(b64decode("vWr41JZ9PL656FAGy906ysrYj/8ccoMUWHT0xEXRftA=")),
            ),
            (
                builder()
                    .checksum(
                        Checksum::builder()
                            .checksum_sha256("La6x82CVtEsxhBCz9Oi12Yncx7sCPRQmxJLasKMFPnQ=")
                            .build(),
                    )
                    .object_size(8388608), // above the threshold
                None,
            ),
            (
                builder()
                    .checksum(
                        Checksum::builder()
                            .checksum_sha256("MIsGKY+ykqN4CPj3gGGu4Gv03N7OWKWpsZqEf+OrGJs=")
                            .build(),
                    )
                    .object_parts(
                        GetObjectAttributesParts::builder()
                            .total_parts_count(1)
                            .parts(
                                ObjectPart::builder()
                                    .size(8388608)
                                    .checksum_sha256("La6x82CVtEsxhBCz9Oi12Yncx7sCPRQmxJLasKMFPnQ=")
                                    .build(),
                            )
                            .build(),
                    )
                    .object_size(8388608), // above the threshold
                Some(b64decode("MIsGKY+ykqN4CPj3gGGu4Gv03N7OWKWpsZqEf+OrGJs=")),
            ),
            (
                builder()
                    .checksum(
                        Checksum::builder()
                            .checksum_sha256("nlR6I2vcFqpTXrJSmMglmCYoByajfADbDQ6kESbPIlE=")
                            .build(),
                    )
                    .object_parts(
                        GetObjectAttributesParts::builder()
                            .total_parts_count(2)
                            .parts(
                                ObjectPart::builder()
                                    .size(5242880)
                                    .checksum_sha256("wDbLt1U6kJ+LiHfURhkkMH8n7LZs/5KO7q/VacOIfik=")
                                    .build(),
                            )
                            .parts(
                                ObjectPart::builder()
                                    .size(8388608)
                                    .checksum_sha256("La6x82CVtEsxhBCz9Oi12Yncx7sCPRQmxJLasKMFPnQ=")
                                    .build(),
                            )
                            .build(),
                    )
                    .object_size(13631488), // above the threshold
                None,
            ),
            (
                builder()
                    .checksum(
                        Checksum::builder()
                            .checksum_sha256("bGeobZC1xyakKeDkOLWP9khl+vuOditELvPQhrT/R9M=")
                            .build(),
                    )
                    .object_parts(
                        GetObjectAttributesParts::builder()
                            .total_parts_count(2)
                            .parts(
                                ObjectPart::builder()
                                    .size(8388608)
                                    .checksum_sha256("La6x82CVtEsxhBCz9Oi12Yncx7sCPRQmxJLasKMFPnQ=")
                                    .build(),
                            )
                            .parts(
                                ObjectPart::builder()
                                    .size(5242880)
                                    .checksum_sha256("wDbLt1U6kJ+LiHfURhkkMH8n7LZs/5KO7q/VacOIfik=")
                                    .build(),
                            )
                            .build(),
                    )
                    .object_size(13631488), // above the threshold
                Some(b64decode("bGeobZC1xyakKeDkOLWP9khl+vuOditELvPQhrT/R9M=")),
            ),
        ];

        for (attrs, expected) in test_data {
            assert_eq!(get_compliant_chunked_checksum(&attrs.build()), expected);
        }
    }
}
