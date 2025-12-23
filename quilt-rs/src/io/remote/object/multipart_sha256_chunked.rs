use std::path::Path;

use aws_sdk_s3::error::DisplayErrorContext;
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::types::ChecksumAlgorithm;
use aws_sdk_s3::types::CompletedMultipartUpload;
use aws_sdk_s3::types::CompletedPart;
use aws_smithy_types::byte_stream::Length;

use crate::checksum::get_checksum_chunksize_and_parts;
use crate::checksum::ObjectHash;
use crate::checksum::Sha256ChunkedHash;
use crate::uri::S3Uri;
use crate::Error;
use crate::Res;

pub async fn multipart_upload_and_sha256_chunksum(
    client: aws_sdk_s3::Client,
    source_path: impl AsRef<Path>,
    dest_uri: &S3Uri,
    size: u64,
) -> Res<(S3Uri, ObjectHash)> {
    let (chunksize, num_chunks) = get_checksum_chunksize_and_parts(size);
    let upload_id = client
        .create_multipart_upload()
        .bucket(&dest_uri.bucket)
        .key(&dest_uri.key)
        .checksum_algorithm(ChecksumAlgorithm::Sha256)
        .send()
        .await
        .map_err(|err| Error::S3Raw(DisplayErrorContext(err).to_string()))?
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
            .map_err(|err| Error::S3Raw(DisplayErrorContext(err).to_string()))?;
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
        .map_err(|err| Error::S3Raw(DisplayErrorContext(err).to_string()))?;

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
        Sha256ChunkedHash::try_from(checksum_b64)?.into(),
    ))
}
