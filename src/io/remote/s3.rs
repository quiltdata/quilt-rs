use std::path::Path;

use aws_sdk_s3::error::DisplayErrorContext;
use aws_sdk_s3::error::SdkError;
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::types::ChecksumAlgorithm;
use aws_sdk_s3::types::CompletedMultipartUpload;
use aws_sdk_s3::types::CompletedPart;
use aws_smithy_types::byte_stream::Length;
use multihash::Multihash;

use crate::checksum::calculate_sha256_checksum;
use crate::checksum::get_checksum_chunksize_and_parts;
use crate::checksum::ContentHash;
use crate::checksum::MULTIPART_THRESHOLD;
use crate::io::remote::get_client_for_bucket;
use crate::io::remote::EntriesStream;
use crate::io::remote::GetObject;
use crate::io::remote::HeadObject;
use crate::io::remote::Remote;
use crate::uri::S3Uri;
use crate::Error;
use crate::Res;

mod attributes;
mod list;

async fn get_object_stream(client: &aws_sdk_s3::Client, s3_uri: &S3Uri) -> Res<GetObject> {
    let result = client.get_object().bucket(&s3_uri.bucket).key(&s3_uri.key);
    let result = match &s3_uri.version {
        Some(version) => result.version_id(version),
        None => result,
    };

    let result = result
        .send()
        .await
        .map_err(|err| Error::S3(DisplayErrorContext(err).to_string()))?;

    let size = match u64::try_from(result.content_length.unwrap()) {
        Err(_) => {
            let msg = "Failed to convert content length to u64";
            return Err(Error::S3HeadObject(msg.into()));
        }
        Ok(size) => size,
    };
    let version = result.version_id().map(|v| v.to_string());
    let head = HeadObject { size, version };

    Ok(GetObject {
        head,
        stream: result.body,
    })
}

async fn put_object_and_checksum(
    source_path: impl AsRef<Path>,
    dest_uri: &S3Uri,
    size: u64,
) -> Res<(S3Uri, Multihash<256>)> {
    let client = get_client_for_bucket(&dest_uri.bucket).await?;
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
    source_path: impl AsRef<Path>,
    dest_uri: &S3Uri,
    size: u64,
) -> Res<(S3Uri, Multihash<256>)> {
    let (chunksize, num_chunks) = get_checksum_chunksize_and_parts(size);
    let client = get_client_for_bucket(&dest_uri.bucket).await?;
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
#[derive(Clone, Debug)]
pub struct RemoteS3 {}

impl Default for RemoteS3 {
    fn default() -> Self {
        Self::new()
    }
}

impl RemoteS3 {
    pub fn new() -> Self {
        RemoteS3 {}
    }
}

impl Remote for RemoteS3 {
    async fn exists(&self, s3_uri: &S3Uri) -> Res<bool> {
        match self.head_object(s3_uri).await {
            Ok(_) => Ok(true),
            Err(Error::ObjectNotFound(_s3_uri)) => Ok(false),
            Err(err) => Err(Error::S3(DisplayErrorContext(err).to_string())),
        }
    }

    async fn head_object(&self, s3_uri: &S3Uri) -> Res<HeadObject> {
        let client = get_client_for_bucket(&s3_uri.bucket).await?;
        let result = client.head_object().bucket(&s3_uri.bucket).key(&s3_uri.key);
        let result = match &s3_uri.version {
            Some(version) => result.version_id(version),
            None => result,
        };
        match result.send().await {
            Err(SdkError::ServiceError(err)) if err.err().is_not_found() => {
                Err(Error::ObjectNotFound(s3_uri.clone()))
            }
            Err(err) => Err(Error::S3(DisplayErrorContext(err).to_string())),
            Ok(head) => {
                let size = match u64::try_from(head.content_length.unwrap()) {
                    Err(_) => {
                        let msg = "Failed to convert content length to u64";
                        return Err(Error::S3HeadObject(msg.into()));
                    }
                    Ok(size) => size,
                };
                let version = head.version_id().map(|v| v.to_string());
                Ok(HeadObject { size, version })
            }
        }
    }

    async fn get_object(&self, s3_uri: &S3Uri) -> Res<GetObject> {
        let client = get_client_for_bucket(&s3_uri.bucket).await?;
        get_object_stream(&client, s3_uri).await
    }

    async fn list_entries(&self, listing_uri: S3Uri) -> impl EntriesStream {
        list::stream(self, listing_uri).await
    }

    async fn put_object(&self, s3_uri: &S3Uri, contents: impl Into<ByteStream>) -> Res {
        let client = get_client_for_bucket(&s3_uri.bucket).await?;
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
        if size < MULTIPART_THRESHOLD {
            put_object_and_checksum(source_path, dest_uri, size).await
        } else {
            multipart_upload_and_checksum(source_path, dest_uri, size).await
        }
    }
}
