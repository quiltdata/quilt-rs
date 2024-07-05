use std::path::PathBuf;

use aws_sdk_s3::operation::get_object_attributes::GetObjectAttributesOutput;
use multihash::Multihash;
use parquet::data_type::AsBytes;

use crate::checksum::get_compliant_chunked_checksum;
use crate::checksum::MULTIHASH_SHA256_CHUNKED;
use crate::manifest::Place;
use crate::manifest::Row;
use crate::uri::S3Uri;
use crate::Error;
use crate::Res;

/// We use it for getting hashes in files listings when we create new packages from S3 directory.
/// Also, we re-use this struct for calculating hashes locally when S3-checksums are disabled.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RowUnmaterialized {
    pub name: PathBuf,
    pub place: Place,
    pub size: u64,
    pub hash: Multihash<256>,
}

impl From<RowUnmaterialized> for Row {
    fn from(row: RowUnmaterialized) -> Self {
        Row {
            info: serde_json::Value::Null,
            meta: serde_json::Value::Null,
            ..row.into()
        }
    }
}

impl RowUnmaterialized {
    pub fn from_get_object_attributes(
        listing_uri: &S3Uri,
        object_key: impl AsRef<str>,
        attrs: GetObjectAttributesOutput,
    ) -> Res<Self> {
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

        Ok(RowUnmaterialized {
            name,
            place: object_uri.into(),
            size,
            hash,
        })
    }
}

pub fn get_relative_name(listing_uri: &S3Uri, object_uri: &S3Uri) -> PathBuf {
    let prefix_len = listing_uri.key.len();
    PathBuf::from(object_uri.key[prefix_len..].to_string())
}
