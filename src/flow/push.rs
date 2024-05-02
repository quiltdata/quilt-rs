use std::path::PathBuf;

use multihash::Multihash;
use tracing::log;
use url::Url;

use crate::checksum::MULTIHASH_SHA256_CHUNKED;
use crate::flow::browse::browse_remote_manifest;
use crate::flow::certify_latest::certify_latest;
use crate::io::manifest::resolve_latest;
use crate::io::manifest::tag_timestamp;
use crate::io::manifest::upload_manifest;
use crate::io::remote::Remote;
use crate::io::storage::Storage;
use crate::lineage::PackageLineage;
use crate::manifest::Table;
use crate::paths;
use crate::uri::Namespace;
use crate::uri::S3Uri;
use crate::Error;

pub async fn push_package(
    mut lineage: PackageLineage,
    local_manifest: &mut Table,
    paths: &paths::DomainPaths,
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    namespace: Namespace,
) -> Result<PackageLineage, Error> {
    let commit = match lineage.commit {
        None => return Ok(lineage), // nothing to commit
        Some(commit) => commit,
    };

    let manifest_uri = lineage.remote;

    let remote_manifest =
        browse_remote_manifest(paths, storage, remote, &manifest_uri.clone().into()).await?;

    // ## copy data
    // Copy each of the _modified_ paths from their local_key to remote_key,
    // keeping track of the resulting versionIds
    //
    // TODO: FAIL if the remote bucket does NOT support versioning (as it would be destructive)

    // ignore removed items, upload changed and new items
    for row in local_manifest.records.values_mut() {
        if let Some(remote_row) = remote_manifest.records.get(&row.name) {
            if remote_row == row {
                row.place = remote_row.place.to_owned();
                continue;
            }
        }

        let local_url = Url::parse(&row.place)?;
        let file_path: PathBuf = local_url.to_file_path().unwrap();

        let s3_key = format!("{}/{}", namespace, row.name.display());
        let s3_uri = S3Uri {
            bucket: manifest_uri.bucket.to_string(),
            key: format!("{}/{}", namespace, row.name.display()),
            version: None,
        };
        log::debug!("Uploading to S3: {}", s3_uri);

        let (version_id, checksum) = remote
            .put_object_and_checksum(&file_path, &s3_uri, row.size)
            .await?;

        // Update the manifest with the sha2-256-chunked checksum.
        row.hash = Multihash::wrap(MULTIHASH_SHA256_CHUNKED, checksum.as_ref())?;

        let remote_url = S3Uri {
            bucket: manifest_uri.bucket.clone(),
            key: s3_key,
            version: version_id,
        };
        log::debug!("got remote url: {}", remote_url);

        // "Relax" the manifest by using those new remote keys
        row.place = remote_url.to_string();
    }

    let new_manifest_uri = upload_manifest(
        storage,
        remote,
        paths,
        manifest_uri.bucket.clone(),
        manifest_uri.namespace.clone(),
        local_manifest.clone(),
    )
    .await?;

    tag_timestamp(remote, &new_manifest_uri, commit.timestamp).await?;

    // Check the hash of remote's latest manifest
    lineage.latest_hash = resolve_latest(remote, manifest_uri.into()).await?;
    lineage.remote = new_manifest_uri.clone();

    // Reset the commit state.
    lineage.commit = None;

    // Try certifying latest if tracking
    if lineage.base_hash == lineage.latest_hash {
        // remote latest has not been updated, certifying the new latest
        return certify_latest(lineage, remote, new_manifest_uri).await;
    }

    Ok(lineage)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::lineage::CommitState;
    use crate::lineage::PackageLineage;
    use crate::manifest::Row;
    use crate::mocks;
    use crate::uri::ManifestUri;

    #[tokio::test]
    async fn test_no_push_if_no_commit() -> Result<(), Error> {
        let storage = mocks::storage::MockStorage::default();
        let remote = mocks::remote::MockRemote::default();
        let lineage = push_package(
            PackageLineage::default(),
            &mut Table::default(),
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
        let manifest_uri = ManifestUri {
            bucket: "b".to_string(),
            namespace: ("a", "c").into(),
            hash: "__FOO__".to_string(),
        };
        let lineage = PackageLineage {
            commit: Some(CommitState::default()),
            remote: manifest_uri,
            ..PackageLineage::default()
        };
        let jsonl = std::fs::read(mocks::manifest::parquet_checksummed())?;
        let manifest_key =
            ".quilt/packages/b/770459d4230273fd44b272c552d1204458175e7d7cb26fcd601c662cf5f72d05";
        let storage = mocks::storage::MockStorage::default();
        storage
            .write_file(PathBuf::from(manifest_key), &jsonl)
            .await?;

        let remote = mocks::remote::MockRemote::default();
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
            &mut Table::default(),
            &paths::DomainPaths::default(),
            &storage,
            &remote,
            Namespace::default(),
        )
        .await?;
        let manifest_uri = ManifestUri {
            bucket: "b".to_string(),
            namespace: ("a", "c").into(),
            hash: "770459d4230273fd44b272c552d1204458175e7d7cb26fcd601c662cf5f72d05".to_string(),
        };
        assert_eq!(
            lineage,
            PackageLineage {
                remote: manifest_uri,
                base_hash: "".to_string(), // Huh?
                latest_hash: "abcdef".to_string(),
                ..PackageLineage::default()
            }
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_single_chunk_push() -> Result<(), Error> {
        let manifest_uri = ManifestUri {
            bucket: "b".to_string(),
            namespace: ("f", "a").into(),
            hash: "__FOO__".to_string(),
        };
        let lineage = PackageLineage {
            commit: Some(CommitState::default()),
            remote: manifest_uri,
            ..PackageLineage::default()
        };
        let jsonl = std::fs::read(mocks::manifest::parquet_checksummed())?;
        let manifest_key =
            ".quilt/packages/b/0f85671863dadacf3a0e62212f1b9151a11f72228e4c82ed86ff27d46ec31d87";
        let storage = mocks::storage::MockStorage::default();
        storage
            .write_file(PathBuf::from(manifest_key), &jsonl)
            .await?;
        let remote = mocks::remote::MockRemote::default();
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

        let file_path = PathBuf::from("/b/a/r");
        let manifest_file = std::fs::read(mocks::manifest::parquet_checksummed())?;
        remote.storage.write_file(&file_path, &manifest_file).await?;
        let mut manifest = mocks::manifest::with_rows(vec![Row {
            name: PathBuf::from("bar"),
            place: format!("file://{}", file_path.display()),
            ..Row::default()
        }]);

        let lineage = push_package(
            lineage,
            &mut manifest,
            &paths::DomainPaths::default(),
            &storage,
            &remote,
            Namespace::default(),
        )
        .await?;
        let manifest_uri = ManifestUri {
            bucket: "b".to_string(),
            namespace: ("f", "a").into(),
            hash: "0f85671863dadacf3a0e62212f1b9151a11f72228e4c82ed86ff27d46ec31d87".to_string(),
        };
        assert_eq!(
            lineage,
            PackageLineage {
                remote: manifest_uri,
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
