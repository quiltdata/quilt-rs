use std::path::PathBuf;

use aws_sdk_s3::error::DisplayErrorContext;
use aws_sdk_s3::operation::get_object_attributes::GetObjectAttributesOutput;
use aws_sdk_s3::types::Object;
use parquet::data_type::AsBytes;
use tracing::log;

use crate::checksum::calculate_sha256_chunked_checksum;
use crate::checksum::get_compliant_chunked_checksum;
use crate::checksum::MPU_MAX_PARTS;
use crate::checksum::MULTIHASH_SHA256_CHUNKED;
use crate::io::remote::get_client_for_bucket;
use crate::io::remote::Remote;
use crate::io::Entry;
use crate::uri::S3Uri;
use crate::Error;
use crate::Res;
use multihash::Multihash;

fn get_relative_name(listing_uri: &S3Uri, object_uri: &S3Uri) -> PathBuf {
    let prefix_len = listing_uri.key.len();
    PathBuf::from(object_uri.key[prefix_len..].to_string())
}

fn convert_to_entry(
    listing_uri: &S3Uri,
    object_key: impl AsRef<str>,
    attrs: GetObjectAttributesOutput,
) -> Res<Entry> {
    if attrs.delete_marker.is_some() {
        // Can happen if object is removed after it was listed but before attributes retrieved.
        return Err(Error::S3("Object is a delete marker".to_string()));
    }

    let checksum = get_compliant_chunked_checksum(&attrs).unwrap();
    let hash = Multihash::wrap(MULTIHASH_SHA256_CHUNKED, checksum.as_bytes())?;

    let size = attrs.object_size.expect("ObjectSize must be requested") as u64;

    let version = attrs.version_id.expect("VersionId must be requested");

    let object_uri = S3Uri {
        bucket: listing_uri.bucket.clone(),
        key: object_key.as_ref().to_string(),
        version: Some(version),
    };
    let prefix_len = listing_uri.key.len();
    let name = PathBuf::from(object_uri.key[prefix_len..].to_string());

    Ok(Entry {
        name,
        place: object_uri.into(),
        size,
        hash,
    })
}

async fn fetch_object_attributes(listing_uri: &S3Uri, object_key: impl AsRef<str>) -> Res<Entry> {
    let client = get_client_for_bucket(&listing_uri.bucket).await?;
    let key = object_key.as_ref();
    log::debug!(
        "Getting attributes for bucket {} key {}",
        &listing_uri.bucket,
        key
    );
    let attrs = client
        .get_object_attributes()
        .bucket(&listing_uri.bucket)
        .key(key)
        .object_attributes(aws_sdk_s3::types::ObjectAttributes::Checksum)
        .object_attributes(aws_sdk_s3::types::ObjectAttributes::ObjectParts)
        .object_attributes(aws_sdk_s3::types::ObjectAttributes::ObjectSize)
        .max_parts(MPU_MAX_PARTS as i32)
        .send()
        .await
        .map_err(|err| Error::S3(DisplayErrorContext(err).to_string()))?;
    // TODO: retry if error?
    convert_to_entry(listing_uri, object_key, attrs)
}

async fn fetch_object_and_calculate_attributes(
    remote: &impl Remote, // FIXME: pass an object stream
    listing_uri: &S3Uri,
    object_key: impl AsRef<str>,
) -> Res<Entry> {
    let object_uri = S3Uri {
        bucket: listing_uri.bucket.clone(),
        key: object_key.as_ref().to_string(),
        version: None, // FIXME: Where is version?
    };
    let object_stream = remote.get_object(&object_uri).await?;
    let size = object_stream.head.size;
    let object = object_stream.stream.into_async_read();
    let name = get_relative_name(listing_uri, &object_uri);

    let hash = calculate_sha256_chunked_checksum(object, size).await?;
    Ok(Entry {
        name,
        place: object_uri.into(),
        size,
        hash,
    })
}

pub async fn get_object_attributes(
    remote: &impl Remote,
    listing_uri: &S3Uri,
    object: Res<Object>,
) -> Res<Entry> {
    let object_key = object?.key.expect("object key expected to be present");
    match fetch_object_attributes(listing_uri, &object_key).await {
        Err(err) => {
            // TODO: Something is broken or Stack doesn't have GetObjectAttribute?
            // TODO: Fallback only if error is: no GetObjectAttribute in stack
            log::warn!("Error getting attributes: {}", err);
            fetch_object_and_calculate_attributes(remote, listing_uri, &object_key).await
        }
        other => other,
    }
}
