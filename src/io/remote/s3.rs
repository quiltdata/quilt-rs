use std::path::Path;

use aws_sdk_s3::error::DisplayErrorContext;
use aws_sdk_s3::error::SdkError;
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::types::ChecksumAlgorithm;
use aws_sdk_s3::types::CompletedMultipartUpload;
use aws_sdk_s3::types::CompletedPart;
use aws_smithy_types::byte_stream::Length;
use base64::prelude::BASE64_STANDARD;
use base64::Engine;
use tokio::io::AsyncRead;
use tokio::io::AsyncReadExt;

use crate::io::remote::Remote;
use crate::io::s3::S3Uri;
use crate::quilt4::checksum;
use crate::Error;

use crate::s3_utils::get_client_for_bucket;

pub async fn bytestream_to_string(bytestream: ByteStream) -> Result<String, Error> {
    let mut reader = bytestream.into_async_read();
    let mut contents = Vec::new();
    reader.read_to_end(&mut contents).await?;
    String::from_utf8(contents).map_err(|err| Error::Utf8(err.utf8_error()))
}

async fn get_object_stream(
    client: &aws_sdk_s3::Client,
    s3_uri: &S3Uri,
) -> Result<ByteStream, Error> {
    let result = client.get_object().bucket(&s3_uri.bucket).key(&s3_uri.key);
    let result = match &s3_uri.version {
        Some(version) => result.version_id(version),
        None => result,
    };

    let result = result
        .send()
        .await
        .map_err(|err| Error::S3(DisplayErrorContext(err).to_string()))?;
    Ok(result.body)
}

async fn get_object(client: &aws_sdk_s3::Client, s3_uri: &S3Uri) -> Result<impl AsyncRead, Error> {
    Ok(get_object_stream(client, s3_uri).await?.into_async_read())
}

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
    async fn get_object(&self, s3_uri: &S3Uri) -> Result<impl AsyncRead + Send + Unpin, Error> {
        let client = get_client_for_bucket(&s3_uri.bucket).await?;
        get_object(&client, s3_uri).await
    }

    async fn get_object_stream(&self, s3_uri: &S3Uri) -> Result<ByteStream, Error> {
        let client = get_client_for_bucket(&s3_uri.bucket).await?;
        get_object_stream(&client, s3_uri).await
    }

    async fn exists(&self, s3_uri: &S3Uri) -> Result<bool, Error> {
        let client = get_client_for_bucket(&s3_uri.bucket).await?;
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

    async fn put_object(
        &self,
        s3_uri: &S3Uri,
        contents: impl Into<ByteStream>,
    ) -> Result<(), Error> {
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

    async fn put_object_and_checksum(
        &self,
        s3_uri: &S3Uri,
        contents: impl Into<ByteStream>,
        size: u64,
    ) -> Result<(Option<String>, Vec<u8>), Error> {
        let client = get_client_for_bucket(&s3_uri.bucket).await?;
        let response = client
            .put_object()
            .bucket(&s3_uri.bucket)
            .key(&s3_uri.key)
            .body(contents.into())
            .checksum_algorithm(ChecksumAlgorithm::Sha256)
            .send()
            .await
            .map_err(|err| Error::S3(DisplayErrorContext(err).to_string()))?;
        let s3_checksum_b64 = response
            .checksum_sha256
            .ok_or(Error::Checksum("missing checksum".to_string()))?;
        let s3_checksum = BASE64_STANDARD.decode(s3_checksum_b64)?;
        let checksum = if size == 0 {
            // Edge case: a 0-byte upload is treated as an empty list of chunks, rather than
            // a list of a 0-byte chunk. Its checksum is sha256(''), NOT sha256(sha256('')).
            s3_checksum
        } else {
            checksum::calculate_sha256_checksum(s3_checksum.as_ref())
                .await?
                .to_vec()
        };

        Ok((response.version_id, checksum))
    }

    async fn multipart_upload_and_checksum(
        &self,
        s3_uri: &S3Uri,
        file_path: impl AsRef<Path>,
        size: u64,
    ) -> Result<(Option<String>, Vec<u8>), Error> {
        let (chunksize, num_chunks) = checksum::get_checksum_chunksize_and_parts(size);
        let client = get_client_for_bucket(&s3_uri.bucket).await?;
        let upload_id = client
            .create_multipart_upload()
            .bucket(&s3_uri.bucket)
            .key(&s3_uri.key)
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
            let file = tokio::fs::File::open(&file_path).await?;
            let chunk_body = ByteStream::read_from()
                .file(file)
                .offset(offset)
                .length(Length::Exact(length)) // https://github.com/awslabs/aws-sdk-rust/issues/821
                .build()
                .await?;
            let part_response = client
                .upload_part()
                .bucket(&s3_uri.bucket)
                .key(&s3_uri.key)
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
            .bucket(&s3_uri.bucket)
            .key(&s3_uri.key)
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
        let checksum = BASE64_STANDARD.decode(checksum_b64)?;

        Ok((response.version_id, checksum))
    }
}
