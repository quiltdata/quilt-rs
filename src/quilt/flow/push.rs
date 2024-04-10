use std::path::PathBuf;

use aws_sdk_s3::error::DisplayErrorContext;
use aws_sdk_s3::types::ChecksumAlgorithm;
use aws_sdk_s3::types::CompletedMultipartUpload;
use aws_sdk_s3::types::CompletedPart;
use aws_smithy_types::byte_stream::ByteStream;
use aws_smithy_types::byte_stream::Length;
use base64::prelude::BASE64_STANDARD;
use base64::Engine;
use multihash::Multihash;
use tracing::log;
use url::Url;

use crate::paths;
use crate::quilt::flow::browse::browse_remote_manifest;
use crate::quilt::flow::browse::cache_manifest;
use crate::quilt::lineage::PackageLineage;
use crate::quilt::manifest;
use crate::quilt::manifest_handle;
use crate::quilt::remote::Remote;
use crate::quilt::storage;
use crate::quilt::storage::s3::S3Uri;
use crate::quilt::storage::Storage;
use crate::quilt::Error;
use crate::quilt4::checksum;

pub async fn push_package(
    mut lineage: PackageLineage,
    manifest: &(impl manifest_handle::ReadableManifest + Sync),
    paths: &paths::DomainPaths,
    storage: &mut impl Storage,
    remote: &mut impl Remote,
    namespace: String,
) -> Result<PackageLineage, Error> {
    let commit = match lineage.commit {
        None => return Ok(lineage), // nothing to commit
        Some(commit) => commit,
    };

    let remote_manifest_address = &lineage.remote;

    let mut local_manifest = manifest.read(storage).await?;
    let remote_manifest =
        browse_remote_manifest(paths, storage, remote, remote_manifest_address).await?;

    // ## copy data
    // Copy each of the _modified_ paths from their local_key to remote_key,
    // keeping track of the resulting versionIds
    //
    // TODO: FAIL if the remote bucket does NOT support versioning (as it would be destructive)

    // ignore removed items, upload changed and new items
    for row in local_manifest.records.values_mut() {
        if let Some(remote_row) = remote_manifest.records.get(&row.name) {
            if remote_row.eq(row) {
                row.place = remote_row.place.to_owned();
                continue;
            }
        }

        let local_url = Url::parse(&row.place)?;
        let file_path: PathBuf = local_url.to_file_path().unwrap();

        let s3_key = format!("{}/{}", namespace, row.name);
        let s3_uri = S3Uri {
            bucket: remote_manifest_address.bucket.to_string(),
            key: format!("{}/{}", namespace, row.name),
            version: None,
        };
        log::debug!("Uploading to S3: {}", s3_uri);

        // TODO: upload in parallel. use a stream?
        let (version_id, checksum) = if row.size < storage::s3::MULTIPART_THRESHOLD {
            let body = ByteStream::read_from().path(&file_path).build().await?;

            remote
                .put_object_and_checksum(&s3_uri, body, row.size)
                .await?
        } else {
            let (chunksize, num_chunks) = checksum::get_checksum_chunksize_and_parts(row.size);
            let client =
                crate::s3_utils::get_client_for_bucket(&remote_manifest_address.bucket).await?;
            let upload_id = client
                .create_multipart_upload()
                .bucket(&remote_manifest_address.bucket)
                .key(&s3_key)
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
                let length = chunksize.min(row.size - offset);
                let chunk_body = ByteStream::read_from()
                    .path(&file_path)
                    .offset(offset)
                    .length(Length::Exact(length)) // https://github.com/awslabs/aws-sdk-rust/issues/821
                    .build()
                    .await?;
                let part_response = client
                    .upload_part()
                    .bucket(&remote_manifest_address.bucket)
                    .key(&s3_key)
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
                .bucket(&remote_manifest_address.bucket)
                .key(&s3_key)
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

            (response.version_id, checksum)
        };

        // Update the manifest with the sha2-256-chunked checksum.
        row.hash = Multihash::wrap(manifest::MULTIHASH_SHA256_CHUNKED, checksum.as_ref())?;

        let remote_url = storage::s3::make_s3_url(
            &remote_manifest_address.bucket,
            &s3_key,
            version_id.as_deref(),
        );
        log::debug!("got remote url: {}", remote_url);

        // "Relax" the manifest by using those new remote keys
        row.place = remote_url.to_string();
    }

    let top_hash = local_manifest.top_hash();
    let new_remote = manifest_handle::RemoteManifest {
        hash: top_hash.clone(),
        ..remote_manifest_address.clone()
    };

    // Cache the relaxed manifest
    let cache_path = cache_manifest(
        paths,
        storage,
        &local_manifest,
        &new_remote.bucket,
        &new_remote.hash,
    )
    .await?;

    // Push the (cached) relaxed manifest to the remote, don't tag it yet
    new_remote.upload_from(storage, remote, &cache_path).await?;

    // Upload a quilt3 manifest for backward compatibility.
    new_remote.upload_legacy(remote, &local_manifest).await?;

    log::debug!("uploaded remote manifest: {new_remote:?}");

    // Tag the new commit.
    // If {self.commit.tag} does not already exist at
    // {self.remote}/.quilt/named_packages/{self.namespace},
    // create it with the value of {self.commit.hash}
    // TODO: Otherwise try again with the current timestamp as the tag
    // (e.g., try five times with exponential backoff, then Error)
    new_remote
        .put_timestamp_tag(remote, commit.timestamp, &new_remote.hash)
        .await?;

    // Check the hash of remote's latest manifest
    lineage.latest_hash = new_remote.resolve_latest(remote).await?;
    lineage.remote = new_remote;

    // Reset the commit state.
    lineage.commit = None;

    // FIXME: use flow::certify_latest
    // Try certifying latest if tracking
    if lineage.base_hash == lineage.latest_hash {
        // remote latest has not been updated, certifying the new latest
        lineage.remote.update_latest(remote, &top_hash).await?;
        lineage.latest_hash = top_hash.clone();
        lineage.base_hash = top_hash.clone();
    }

    Ok(lineage)
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::HashMap;

    use crate::quilt::lineage::CommitState;
    use crate::quilt::lineage::PackageLineage;
    use crate::quilt::manifest_handle::RemoteManifest;
    use crate::quilt::mocks;
    use crate::quilt::remote::mock_remote::MockRemote;
    use crate::quilt::storage::mock_storage::MockStorage;
    use crate::quilt::S3PackageUri;
    use crate::utils::local_uri_parquet;

    #[tokio::test]
    async fn test_no_push_if_no_commit() -> Result<(), Error> {
        let mut storage = MockStorage::default();
        let mut remote = MockRemote::default();
        let lineage = push_package(
            PackageLineage::default(),
            &mocks::manifest::default(),
            &paths::DomainPaths::default(),
            &mut storage,
            &mut remote,
            String::default(),
        )
        .await?;
        assert_eq!(lineage, PackageLineage::default());
        Ok(())
    }

    #[tokio::test]
    #[ignore]
    async fn test_push() -> Result<(), Error> {
        let remote_manifest: RemoteManifest =
            S3PackageUri::try_from("quilt+s3://b#package=a@__FOO__")?.into();
        let lineage = PackageLineage {
            commit: Some(CommitState::default()),
            remote: remote_manifest,
            ..PackageLineage::default()
        };
        let jsonl = std::fs::read(local_uri_parquet())?;
        let manifest_key =
            ".quilt/packages/b/770459d4230273fd44b272c552d1204458175e7d7cb26fcd601c662cf5f72d05";
        let mut storage = MockStorage {
            registry: HashMap::from([(PathBuf::from(manifest_key), jsonl.clone())]),
        };
        let mut remote = MockRemote {
            registry: HashMap::from([(
                "s3://b/.quilt/packages/1220__FOO__.parquet".to_string(),
                jsonl,
            )]),
        };
        let lineage = push_package(
            lineage,
            &mocks::manifest::default(),
            &paths::DomainPaths::default(),
            &mut storage,
            &mut remote,
            String::default(),
        )
        .await?;
        assert_eq!(lineage, PackageLineage::default());
        Ok(())
    }
}
