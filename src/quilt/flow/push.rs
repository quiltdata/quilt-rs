use std::path::PathBuf;

use aws_smithy_types::byte_stream::ByteStream;
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
use crate::quilt::uri::Namespace;
use crate::quilt::Error;

pub async fn push_package(
    mut lineage: PackageLineage,
    manifest: &(impl manifest_handle::ReadableManifest + Sync),
    paths: &paths::DomainPaths,
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    namespace: Namespace,
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

        let s3_key = format!("{}/{}", namespace, row.name.display());
        let s3_uri = S3Uri {
            bucket: remote_manifest_address.bucket.to_string(),
            key: format!("{}/{}", namespace, row.name.display()),
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
            remote
                .multipart_upload_and_checksum(&s3_uri, file_path, row.size)
                .await?
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

    use crate::quilt::lineage::CommitState;
    use crate::quilt::lineage::PackageLineage;
    use crate::quilt::manifest_handle::RemoteManifest;
    use crate::quilt::mocks;
    use crate::quilt::remote::mock_remote::MockRemote;
    use crate::quilt::storage::mock_storage::MockStorage;
    use crate::quilt::S3PackageUri;
    use crate::utils::local_uri_parquet_checksummed;
    use crate::Row4;

    #[tokio::test]
    async fn test_no_push_if_no_commit() -> Result<(), Error> {
        let storage = MockStorage::default();
        let remote = MockRemote::default();
        let lineage = push_package(
            PackageLineage::default(),
            &mocks::manifest::default(),
            &paths::DomainPaths::default(),
            &storage,
            &remote,
            Namespace::default(),
        )
        .await?;
        assert_eq!(lineage, PackageLineage::default());
        Ok(())
    }

    #[tokio::test]
    async fn test_no_entries_push() -> Result<(), Error> {
        let remote_manifest: RemoteManifest =
            S3PackageUri::try_from("quilt+s3://b#package=a/c@__FOO__")?.into();
        let lineage = PackageLineage {
            commit: Some(CommitState::default()),
            remote: remote_manifest,
            ..PackageLineage::default()
        };
        let jsonl = std::fs::read(local_uri_parquet_checksummed())?;
        let manifest_key =
            ".quilt/packages/b/770459d4230273fd44b272c552d1204458175e7d7cb26fcd601c662cf5f72d05";
        let storage = MockStorage::default();
        storage
            .write_file(PathBuf::from(manifest_key), &jsonl)
            .await?;

        let remote = MockRemote::default();
        remote
            .put_object(
                &S3Uri::try_from("s3://b/.quilt/packages/1220__FOO__.parquet")?,
                jsonl,
            )
            .await?;
        remote
            .put_object(
                &S3Uri::try_from("s3://b/.quilt/named_packages/a/c/latest")?,
                b"abcdef".to_vec(),
            )
            .await?;
        let lineage = push_package(
            lineage,
            &mocks::manifest::default(),
            &paths::DomainPaths::default(),
            &storage,
            &remote,
            Namespace::default(),
        )
        .await?;
        let result_remote_manifest: RemoteManifest = S3PackageUri::try_from("quilt+s3://b#package=a/c@770459d4230273fd44b272c552d1204458175e7d7cb26fcd601c662cf5f72d05")?.into();
        assert_eq!(
            lineage,
            PackageLineage {
                remote: result_remote_manifest,
                base_hash: "".to_string(), // Huh?
                latest_hash: "abcdef".to_string(),
                ..PackageLineage::default()
            }
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_single_chunk_push() -> Result<(), Error> {
        let remote_manifest: RemoteManifest =
            S3PackageUri::try_from("quilt+s3://b#package=f/a@__FOO__")?.into();
        let lineage = PackageLineage {
            commit: Some(CommitState::default()),
            remote: remote_manifest,
            ..PackageLineage::default()
        };
        let jsonl = std::fs::read(local_uri_parquet_checksummed())?;
        let temp_dir = tempfile::tempdir()?;
        let manifest_key =
            ".quilt/packages/b/0f85671863dadacf3a0e62212f1b9151a11f72228e4c82ed86ff27d46ec31d87";
        let storage = MockStorage::default();
        storage
            .write_file(PathBuf::from(manifest_key), &jsonl)
            .await?;
        let remote = MockRemote::default();
        remote
            .put_object(
                &S3Uri::try_from("s3://b/.quilt/packages/1220__FOO__.parquet")?,
                jsonl,
            )
            .await?;
        remote
            .put_object(
                &S3Uri::try_from("s3://b/.quilt/named_packages/f/a/latest")?,
                b"abcdef".to_vec(),
            )
            .await?;

        let file_path = temp_dir.into_path().join("bar");
        tokio::fs::copy(local_uri_parquet_checksummed(), &file_path).await?;

        let manifest = mocks::manifest::with_rows(vec![Row4 {
            name: PathBuf::from("bar"),
            place: format!("file://{}", file_path.display()),
            ..Row4::default()
        }]);

        let lineage = push_package(
            lineage,
            &manifest,
            &paths::DomainPaths::default(),
            &storage,
            &remote,
            Namespace::default(),
        )
        .await?;
        let result_remote_manifest: RemoteManifest = S3PackageUri::try_from("quilt+s3://b#package=f/a@0f85671863dadacf3a0e62212f1b9151a11f72228e4c82ed86ff27d46ec31d87")?.into();
        assert_eq!(
            lineage,
            PackageLineage {
                remote: result_remote_manifest,
                base_hash: "".to_string(), // Huh?
                latest_hash: "abcdef".to_string(),
                ..PackageLineage::default()
            }
        );
        Ok(())
    }

    #[tokio::test]
    #[ignore]
    async fn test_multichunk_push() -> Result<(), Error> {
        // TODO
        Ok(())
    }
}
