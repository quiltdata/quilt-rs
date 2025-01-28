use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::path::Path;
use std::sync::RwLock;
use tracing::log;

use async_stream::try_stream;
use aws_config::BehaviorVersion;
use aws_sdk_s3::error::DisplayErrorContext;
use aws_sdk_s3::error::SdkError;
use aws_sdk_s3::operation::get_object_attributes::GetObjectAttributesOutput;
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::types::ChecksumAlgorithm;
use aws_sdk_s3::types::CompletedMultipartUpload;
use aws_sdk_s3::types::CompletedPart;
use aws_sdk_s3::types::Object;
use aws_smithy_types::byte_stream::Length;
use aws_types::region::Region;
use parquet::data_type::AsBytes;

use multihash::Multihash;
use tokio::io::AsyncRead;

use crate::checksum::calculate_sha256_checksum;
use crate::checksum::get_checksum_chunksize_and_parts;
use crate::checksum::get_compliant_chunked_checksum;
use crate::checksum::ContentHash;
use crate::checksum::MPU_MAX_PARTS;
use crate::checksum::MULTIHASH_SHA256_CHUNKED;
use crate::io::remote::ObjectsStream;
use crate::io::remote::Remote;
use crate::uri::S3Uri;
use crate::Error;
use crate::Res;

const LIST_OBJECTS_V2_MAX_KEYS: i32 = 1_00;

use crate::io::remote::RemoteObjectStream;
use crate::io::remote::S3Attributes;

struct S3AttributesWrapper {
    pub hash: Multihash<256>,
    pub size: u64,
    pub version: String,
}

impl TryFrom<GetObjectAttributesOutput> for S3AttributesWrapper {
    type Error = Error;
    fn try_from(attrs: GetObjectAttributesOutput) -> Result<Self, Self::Error> {
        if attrs.delete_marker.is_some() {
            // Can happen if object is removed after it was listed but before attributes retrieved.
            return Err(Error::S3("Object is a delete marker".to_string()));
        }

        let checksum = match get_compliant_chunked_checksum(&attrs) {
            Some(c) => c,
            None => return Err(Error::Checksum("missing checksum".to_string())),
        };
        let hash = Multihash::wrap(MULTIHASH_SHA256_CHUNKED, checksum.as_bytes())?;
        let size = attrs.object_size.expect("ObjectSize must be requested") as u64;
        Ok(S3AttributesWrapper {
            version: attrs.version_id.expect("VersionId must be requested"),
            hash,
            size,
        })
    }
}

async fn find_bucket_region(client: &reqwest::Client, bucket: &str) -> Res<String> {
    let response = client
        .head(format!("https://s3.amazonaws.com/{bucket}"))
        .send()
        .await?;

    match response.headers().get("x-amz-bucket-region") {
        Some(location) => Ok(location.to_str()?.into()),
        None => Err(Error::MissingHTTPHeader("x-amz-bucket-region".to_string())),
    }
}

async fn get_object_stream(client: &aws_sdk_s3::Client, s3_uri: &S3Uri) -> Res<RemoteObjectStream> {
    let result = client.get_object().bucket(&s3_uri.bucket).key(&s3_uri.key);
    let result = match &s3_uri.version {
        Some(version) => result.version_id(version),
        None => result,
    };

    let result = result
        .send()
        .await
        .map_err(|err| Error::S3(DisplayErrorContext(err).to_string()))?;
    let uri_versioned = S3Uri {
        version: result.version_id,
        ..s3_uri.clone()
    };
    Ok(RemoteObjectStream {
        body: result.body,
        uri: uri_versioned,
    })
}

async fn get_object(client: &aws_sdk_s3::Client, s3_uri: &S3Uri) -> Res<impl AsyncRead> {
    Ok(get_object_stream(client, s3_uri)
        .await?
        .body
        .into_async_read())
}

async fn put_object_and_checksum(
    client: aws_sdk_s3::Client,
    source_path: impl AsRef<Path>,
    dest_uri: &S3Uri,
    size: u64,
) -> Res<(S3Uri, Multihash<256>)> {
    // let client = get_client_for_bucket(&dest_uri.bucket).await?;
    let response = client
        .put_object()
        .bucket(&dest_uri.bucket)
        .key(&dest_uri.key)
        .body(ByteStream::from_path(source_path).await?)
        .checksum_algorithm(ChecksumAlgorithm::Sha256)
        .send()
        .await
        .map_err(|err| Error::S3(DisplayErrorContext(err).to_string()))?;
    let s3_checksum_b64 = response
        .checksum_sha256
        .ok_or(Error::Checksum("missing checksum".to_string()))?;
    // let s3_checksum = BASE64_STANDARD.decode(s3_checksum_b64)?;
    let hash: Multihash<256> =
        ContentHash::SHA256Chunked(s3_checksum_b64.to_string()).try_into()?;
    let checksum = if size == 0 {
        // Edge case: a 0-byte upload is treated as an empty list of chunks, rather than
        // a list of a 0-byte chunk. Its checksum is sha256(''), NOT sha256(sha256('')).
        hash
    } else {
        // NOTE: we're calculating checksum of checksums here,
        //       not a checksum of the file
        // NOTE: in the current design, we're not using this checksum
        calculate_sha256_checksum(hash.digest()).await?
    };

    Ok((
        S3Uri {
            version: response.version_id,
            ..dest_uri.clone()
        },
        checksum,
    ))
}

async fn multipart_upload_and_checksum(
    client: aws_sdk_s3::Client,
    source_path: impl AsRef<Path>,
    dest_uri: &S3Uri,
    size: u64,
) -> Res<(S3Uri, Multihash<256>)> {
    let (chunksize, num_chunks) = get_checksum_chunksize_and_parts(size);
    //let client = get_client_for_bucket(&dest_uri.bucket).await?;
    let upload_id = client
        .create_multipart_upload()
        .bucket(&dest_uri.bucket)
        .key(&dest_uri.key)
        .checksum_algorithm(ChecksumAlgorithm::Sha256)
        .send()
        .await
        .map_err(|err| Error::S3(DisplayErrorContext(err).to_string()))?
        .upload_id
        .ok_or(Error::UploadId("failed to get an UploadId".to_string()))?;

    let mut parts: Vec<CompletedPart> = Vec::new();
    for chunk_idx in 0..num_chunks {
        let part_number = chunk_idx as i32 + 1;
        let offset = chunk_idx * chunksize;
        let length = chunksize.min(size - offset);
        let chunk_body = ByteStream::read_from()
            .path(source_path.as_ref())
            .offset(offset)
            .length(Length::Exact(length)) // https://github.com/awslabs/aws-sdk-rust/issues/821
            .build()
            .await?;
        let part_response = client
            .upload_part()
            .bucket(&dest_uri.bucket)
            .key(&dest_uri.key)
            .upload_id(&upload_id)
            .part_number(part_number)
            .checksum_algorithm(ChecksumAlgorithm::Sha256)
            .body(chunk_body)
            .send()
            .await
            .map_err(|err| Error::S3(DisplayErrorContext(err).to_string()))?;
        parts.push(
            CompletedPart::builder()
                .part_number(part_number)
                .e_tag(part_response.e_tag.unwrap_or_default())
                .checksum_sha256(part_response.checksum_sha256.unwrap_or_default())
                .build(),
        );
    }

    let response = client
        .complete_multipart_upload()
        .bucket(&dest_uri.bucket)
        .key(&dest_uri.key)
        .upload_id(&upload_id)
        .multipart_upload(
            CompletedMultipartUpload::builder()
                .set_parts(Some(parts))
                .build(),
        )
        .send()
        .await
        .map_err(|err| Error::S3(DisplayErrorContext(err).to_string()))?;

    let s3_checksum = response
        .checksum_sha256
        .ok_or(Error::Checksum("missing checksum".to_string()))?;
    let (checksum_b64, _) = s3_checksum
        .split_once('-')
        .ok_or(Error::Checksum("unexpected checksum".to_string()))?;

    Ok((
        S3Uri {
            version: response.version_id,
            ..dest_uri.clone()
        },
        ContentHash::SHA256Chunked(checksum_b64.to_string()).try_into()?,
    ))
}

/// Implementation of the `Remote` trait for S3
#[derive(Debug)]
pub struct RemoteS3 {
    http: reqwest::Client,
    s3: RwLock<HashMap<Region, aws_sdk_s3::Client>>,
    regions: RwLock<HashMap<String, Region>>,
}

impl std::clone::Clone for RemoteS3 {
    fn clone(&self) -> Self {
        RemoteS3 {
            http: self.http.clone(),
            s3: RwLock::new(self.s3.read().unwrap().clone()),
            regions: RwLock::new(self.regions.read().unwrap().clone()),
        }
    }
}

impl Default for RemoteS3 {
    fn default() -> Self {
        Self::new()
    }
}

impl RemoteS3 {
    pub fn new() -> Self {
        RemoteS3 {
            http: reqwest::Client::new(),
            s3: RwLock::new(HashMap::new()),
            regions: RwLock::new(HashMap::new()),
        }
    }

    async fn get_region_for_bucket(&self, bucket: &str) -> Res<Region> {
        {
            if let Some(region) = self.regions.read().unwrap().get(bucket) {
                return Ok(region.clone());
            }
        }

        let region = find_bucket_region(&self.http, bucket).await?;

        let mut map = self.regions.write().unwrap();
        match map.entry(bucket.to_owned()) {
            Entry::Occupied(entry) => Ok(entry.get().clone()),
            Entry::Vacant(entry) => Ok(entry.insert(Region::new(region)).clone()),
        }
    }

    async fn get_client_for_region(&self, region: aws_types::region::Region) -> aws_sdk_s3::Client {
        {
            let map = self.s3.read().unwrap();
            if let Some(client) = map.get(&region) {
                return client.clone();
            }
        }

        let config = aws_config::defaults(BehaviorVersion::latest())
            .region(region.clone())
            .load()
            .await;
        let client = aws_sdk_s3::Client::new(&config);

        let mut map = self.s3.write().unwrap();

        match map.entry(region) {
            Entry::Occupied(entry) => entry.get().clone(),
            Entry::Vacant(entry) => entry.insert(client).clone(),
        }
    }

    async fn get_client_for_bucket(&self, bucket: &str) -> Res<aws_sdk_s3::Client> {
        let region = self.get_region_for_bucket(bucket).await?.clone();
        Ok(self.get_client_for_region(region).await)
    }
}

impl Remote for RemoteS3 {
    async fn exists(&self, s3_uri: &S3Uri) -> Res<bool> {
        let client = self.get_client_for_bucket(&s3_uri.bucket).await?;
        let result = client.head_object().bucket(&s3_uri.bucket).key(&s3_uri.key);
        let result = match &s3_uri.version {
            Some(version) => result.version_id(version),
            None => result,
        };
        match result.send().await {
            Ok(_) => Ok(true),
            Err(SdkError::ServiceError(err)) if err.err().is_not_found() => Ok(false),
            Err(err) => Err(Error::S3(DisplayErrorContext(err).to_string())),
        }
    }

    async fn get_object(&self, s3_uri: &S3Uri) -> Res<impl AsyncRead + Send + Unpin> {
        let client = self.get_client_for_bucket(&s3_uri.bucket).await?;
        get_object(&client, s3_uri).await
    }

    async fn get_object_attributes(
        &self,
        listing_uri: &S3Uri,
        object: &Object,
    ) -> Res<S3Attributes> {
        let client = self.get_client_for_bucket(&listing_uri.bucket).await?;
        let key = object.key.clone().ok_or(Error::ObjectKey)?;
        log::debug!(
            "Getting attributes for bucket {} key {}",
            &listing_uri.bucket,
            key
        );
        let attrs = client
            .get_object_attributes()
            .bucket(&listing_uri.bucket)
            .key(key.clone())
            .object_attributes(aws_sdk_s3::types::ObjectAttributes::Checksum)
            .object_attributes(aws_sdk_s3::types::ObjectAttributes::ObjectParts)
            .object_attributes(aws_sdk_s3::types::ObjectAttributes::ObjectSize)
            .max_parts(MPU_MAX_PARTS as i32)
            .send()
            .await
            .map_err(|err| Error::S3(DisplayErrorContext(err).to_string()))?;

        let S3AttributesWrapper {
            size,
            hash,
            version,
        } = attrs.try_into()?;
        Ok(S3Attributes {
            listing_uri: listing_uri.clone(),
            object_uri: S3Uri {
                bucket: listing_uri.bucket.clone(),
                key: key.to_string(),
                version: Some(version),
            },
            hash,
            size,
        })
    }

    async fn get_object_stream(&self, s3_uri: &S3Uri) -> Res<RemoteObjectStream> {
        let client = self.get_client_for_bucket(&s3_uri.bucket).await?;
        get_object_stream(&client, s3_uri).await
    }

    async fn list_objects(&self, listing_uri: S3Uri) -> impl ObjectsStream {
        try_stream! {
            let client = self.get_client_for_bucket(&listing_uri.bucket).await?;
            let mut paginated_stream = client
                .list_objects_v2()
                .bucket(&listing_uri.bucket)
                .prefix(&listing_uri.key)
                .into_paginator()
                .page_size(LIST_OBJECTS_V2_MAX_KEYS) // XXX: this is to limit concurrency
                .send();
            while let Some(page) = paginated_stream.next().await {
                yield page
                    .map_err(|err| Error::S3(DisplayErrorContext(err).to_string()))?
                    .contents
                    .into_iter()
                    .flatten()
                    .map(Ok)
                    .collect::<Vec<_>>();
            }
        }
    }

    async fn put_object(&self, s3_uri: &S3Uri, contents: impl Into<ByteStream>) -> Res {
        let client = self.get_client_for_bucket(&s3_uri.bucket).await?;
        client
            .put_object()
            .bucket(&s3_uri.bucket)
            .key(&s3_uri.key)
            .body(contents.into())
            .send()
            .await
            .map_err(|err| Error::S3(DisplayErrorContext(err).to_string()))?;

        Ok(())
    }

    async fn upload_file(
        &self,
        source_path: impl AsRef<Path>,
        dest_uri: &S3Uri,
        size: u64,
    ) -> Res<(S3Uri, Multihash<256>)> {
        let client = self.get_client_for_bucket(&dest_uri.bucket).await?;
        if size == 0 {
            put_object_and_checksum(client, source_path, dest_uri, size).await
        } else {
            multipart_upload_and_checksum(client, source_path, dest_uri, size).await
        }
    }
}
