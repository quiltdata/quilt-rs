use std::path::Path;

use aws_sdk_s3::error::DisplayErrorContext;
use aws_sdk_s3::operation::put_object::PutObjectOutput;
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::types::ChecksumAlgorithm;

use crate::checksum::Crc64Hash;
use crate::checksum::ObjectHash;
use crate::checksum::Sha256ChunkedHash;
use crate::error::S3Error;
use crate::error::S3ErrorKind;
use crate::io::remote::HostChecksums;
use crate::io::remote::HostConfig;
use crate::uri::S3Uri;
use crate::Error;
use crate::Res;
use crate::error::ChecksumError;

fn extract_sha256_checksum(output: &PutObjectOutput) -> Res<Option<ObjectHash>> {
    match &output.checksum_sha256 {
        Some(checksum_in_b64) => Ok(Some(Sha256ChunkedHash::try_from(checksum_in_b64)?.into())),
        None => Ok(None),
    }
}

fn extract_crc64_checksum(output: &PutObjectOutput) -> Res<Option<ObjectHash>> {
    match &output.checksum_crc64_nvme {
        Some(checksum_in_b64) => Ok(Some(Crc64Hash::try_from(checksum_in_b64)?.into())),
        None => Ok(None),
    }
}

impl From<&HostChecksums> for ChecksumAlgorithm {
    fn from(requested_checksum: &HostChecksums) -> Self {
        match requested_checksum {
            HostChecksums::Sha256Chunked => ChecksumAlgorithm::Sha256,
            HostChecksums::Crc64 => ChecksumAlgorithm::Crc64Nvme,
        }
    }
}

pub async fn put_and_request_checksum(
    client: aws_sdk_s3::Client,
    source_path: impl AsRef<Path>,
    dest_uri: &S3Uri,
    host_config: &HostConfig,
) -> Res<(S3Uri, ObjectHash)> {
    let response = client
        .put_object()
        .bucket(&dest_uri.bucket)
        .key(&dest_uri.key)
        .body(ByteStream::from_path(source_path).await?)
        .checksum_algorithm((&host_config.checksums).into())
        .send()
        .await
        .map_err(|err| {
            Error::S3(S3Error {
                host: host_config.host.clone(),
                kind: S3ErrorKind::UploadFile(DisplayErrorContext(err).to_string()),
            })
        })?;
    let checksum = match host_config.checksums {
        HostChecksums::Sha256Chunked => extract_sha256_checksum(&response)?,
        HostChecksums::Crc64 => extract_crc64_checksum(&response)?,
    };
    let uri = S3Uri {
        version: response.version_id,
        ..dest_uri.clone()
    };

    match checksum {
        Some(hash) => Ok((uri, hash)),
        None => Err(Error::Checksum(ChecksumError::Missing(
            host_config.checksums.clone(),
        ))),
    }
}
