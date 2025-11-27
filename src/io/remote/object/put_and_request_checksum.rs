use std::path::Path;

use aws_sdk_s3::error::DisplayErrorContext;
use aws_sdk_s3::operation::put_object::PutObjectOutput;
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::types::ChecksumAlgorithm;

use crate::checksum::Crc64Hash;
use crate::checksum::ObjectHash;
use crate::checksum::Sha256ChunkedHash;
use crate::error::S3Error;
use crate::io::remote::HostChecksums;
use crate::io::remote::HostConfig;
use crate::uri::S3Uri;
use crate::Error;
use crate::Res;

impl TryFrom<PutObjectOutput> for Sha256ChunkedHash {
    type Error = crate::Error;

    fn try_from(output: PutObjectOutput) -> Result<Self, Self::Error> {
        match output.checksum_sha256 {
            Some(checksum_in_b64) => checksum_in_b64.as_str().try_into(),
            None => Err(Error::ChecksumMissing(HostChecksums::Sha256Chunked)),
        }
    }
}

impl TryFrom<PutObjectOutput> for Crc64Hash {
    type Error = crate::Error;

    fn try_from(output: PutObjectOutput) -> Result<Self, Self::Error> {
        match output.checksum_crc64_nvme {
            Some(checksum_in_b64) => checksum_in_b64.as_str().try_into(),
            None => Err(Error::ChecksumMissing(HostChecksums::Crc64)),
        }
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
            Error::S3(
                host_config.host.clone(),
                S3Error::UploadFile(DisplayErrorContext(err).to_string()),
            )
        })?;
    let uri = S3Uri {
        version: response.version_id.clone(),
        ..dest_uri.clone()
    };
    let checksum = match host_config.checksums {
        HostChecksums::Sha256Chunked => Sha256ChunkedHash::try_from(response)?.into(),
        HostChecksums::Crc64 => Crc64Hash::try_from(response)?.into(),
    };
    Ok((uri, checksum))
}
