use tokio_stream::StreamExt;
use tracing::debug;
use tracing::info;
use tracing::warn;

use crate::flow;
use crate::io::manifest::build_manifest_from_rows_stream;
use crate::io::manifest::resolve_tag;
use crate::io::manifest::tag_timestamp;
use crate::io::manifest::upload_manifest;
use crate::io::manifest::upload_row;
use crate::io::manifest::RowsStream;
use crate::io::manifest::StreamItem;
use crate::io::remote::HostConfig;
use crate::io::remote::Remote;
use crate::io::storage::Storage;
use crate::lineage::PackageLineage;
use crate::manifest::Manifest;
use crate::manifest::ManifestRow;
use crate::paths;
use crate::uri::ManifestUri;
use crate::uri::Namespace;
use crate::uri::S3PackageHandle;
use crate::uri::Tag;
use crate::Error;
use crate::Res;

async fn use_existing_row_or_upload(
    remote: &impl Remote,
    host_config: &HostConfig,
    package_handle: &S3PackageHandle,
    remote_manifest: &Manifest,
    rows: StreamItem,
) -> StreamItem {
    let mut output = Vec::new();
    for row in rows? {
        let row = row?;
        debug!("⏳ Processing row: {}", row.logical_key.display());
        if let Some(remote_row) = remote_manifest.get_record(&row.logical_key) {
            if remote_row == &row {
                debug!(
                    "✔️ Using existing remote row for: {}",
                    row.logical_key.display()
                );
                let updated_manifest_row = ManifestRow {
                    physical_key: remote_row.physical_key.to_owned(),
                    ..row.clone()
                };
                output.push(Ok(updated_manifest_row));
            } else {
                debug!(
                    "⏳ Uploading modified row for: {}",
                    row.logical_key.display()
                );
                let uploaded_row =
                    upload_row(remote, host_config, package_handle.clone(), row).await?;
                output.push(Ok(uploaded_row));
            }
        } else {
            debug!("⏳ Uploading new row for: {}", row.logical_key.display());
            let uploaded_row = upload_row(remote, host_config, package_handle.clone(), row).await?;
            output.push(Ok(uploaded_row));
        }
    }
    Ok(output)
}

async fn stream_uploaded_local_rows<'a>(
    remote: &'a impl Remote,
    host_config: &'a HostConfig,
    local_manifest: &'a Manifest,
    remote_manifest: &'a Manifest,
    package_handle: &'a S3PackageHandle,
) -> impl RowsStream + 'a {
    let stream = local_manifest.records_stream().await;
    stream.then(move |rows| {
        use_existing_row_or_upload(remote, host_config, package_handle, remote_manifest, rows)
    })
}

/// Push the new package revision to the remote and tags it as "latest".
pub async fn push_package(
    mut lineage: PackageLineage,
    local_manifest: Manifest,
    paths: &paths::DomainPaths,
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    namespace: Option<Namespace>,
    host_config: HostConfig,
) -> Res<PackageLineage> {
    let commit = match lineage.commit.take() {
        None => {
            info!("No changes to push");
            return Ok(lineage); // nothing to commit
        }
        Some(commit) => commit,
    };

    let remote_uri = lineage.remote()?.clone();

    debug!("⏳ Fetching remote manifest");
    let remote_manifest = flow::browse(paths, storage, remote, &remote_uri).await?;
    debug!("✔️ Remote manifest fetched");

    // ## copy data
    // Copy each of the _modified_ paths from their local_key to remote_key,
    // keeping track of the resulting versionIds
    //
    // TODO: FAIL if the remote bucket does NOT support versioning (as it would be destructive)

    let namespace = namespace.unwrap_or(remote_uri.namespace.clone());

    debug!("⏳ Creating manifest URI");
    let manifest_uri = ManifestUri {
        namespace,
        ..remote_uri.clone()
    };
    debug!("✔️ Created manifest URI: {}", manifest_uri.display());

    debug!("⏳ Building and uploading manifest");
    let package_handle = S3PackageHandle::from(&manifest_uri);
    let stream = Box::pin(
        stream_uploaded_local_rows(
            remote,
            &host_config,
            &local_manifest,
            &remote_manifest,
            &package_handle,
        )
        .await,
    );
    let dest_dir = paths.cached_manifests_dir(&manifest_uri.bucket);
    let (cache_path, top_hash) =
        build_manifest_from_rows_stream(storage, dest_dir, local_manifest.header.clone(), stream)
            .await?;
    debug!(
        "✔️ Built manifest with hash {} at {}",
        top_hash,
        cache_path.display()
    );

    let new_manifest_uri = ManifestUri {
        hash: top_hash,
        ..remote_uri.clone()
    };

    debug!(
        "⏳ Uploading manifest to remote {}",
        new_manifest_uri.display()
    );
    upload_manifest(storage, remote, &new_manifest_uri, &cache_path).await?;
    debug!("✔️ Manifest uploaded");

    debug!("⏳ Adding timestamp tag {}", commit.timestamp);
    tag_timestamp(remote, &new_manifest_uri, commit.timestamp).await?;
    debug!("✔️ Timestamp tag added");

    debug!("⏳ Checking remote's latest manifest hash");
    lineage.latest_hash = resolve_tag(
        remote,
        &new_manifest_uri.origin,
        &manifest_uri.into(),
        Tag::Latest,
    )
    .await?
    .hash;
    debug!("✔️ Latest hash is: {}", lineage.latest_hash);

    lineage.remote_uri = Some(new_manifest_uri.clone());

    if new_manifest_uri.hash != commit.hash {
        debug!("❌ Hash mismatch, copying cached to installed");
        // Otherwise, lineage will be pointing to the wrong/inexisting hash
        paths::copy_cached_to_installed(paths, storage, &new_manifest_uri).await?;
        Err(Error::Push(
            "Latest local hash is not equal to pushed manifest commit".to_string(),
        ))?
    }

    // Try certifying latest if tracking
    if lineage.base_hash == lineage.latest_hash {
        debug!("⏳ Remote latest not updated, certifying new latest");
        return flow::certify_latest(lineage, remote, new_manifest_uri).await;
    } else {
        warn!(r#"⏳ We do not "track" the latest hash, so we will not certify it"#);
    }

    info!("✔️ Successfully pushed package");
    Ok(lineage)
}

#[cfg(test)]
mod tests {
    use super::*;

    use test_log::test;

    use std::path::PathBuf;

    use aws_sdk_s3::primitives::ByteStream;

    use crate::fixtures;
    use crate::io::remote::mocks::MockRemote;
    use crate::io::storage::mocks::MockStorage;
    use crate::lineage::CommitState;
    use crate::lineage::PackageLineage;
    use crate::uri::S3Uri;

    #[test(tokio::test)]
    async fn test_no_push_if_no_commit() -> Res {
        let storage = MockStorage::default();
        let remote = MockRemote::default();
        let lineage = push_package(
            PackageLineage::default(),
            Manifest::default(),
            &paths::DomainPaths::default(),
            &storage,
            &remote,
            None,
            HostConfig::default(),
        )
        .await?;
        assert_eq!(lineage, PackageLineage::default());
        Ok(())
    }

    #[test(tokio::test)]
    async fn test_no_entries_push() -> Res {
        let manifest_uri = ManifestUri {
            bucket: "b".to_string(),
            namespace: ("a", "c").into(),
            hash: "__FOO__".to_string(),
            origin: None,
        };
        let lineage = PackageLineage {
            commit: Some(CommitState {
                timestamp: chrono::Utc::now(),
                hash: fixtures::top_hash::EMPTY_NULL_TOP_HASH.to_string(),
                prev_hashes: Vec::new(),
            }),
            remote_uri: Some(manifest_uri),
            ..PackageLineage::default()
        };
        let manifest_key = format!(
            ".quilt/packages/b/{}",
            fixtures::top_hash::EMPTY_NULL_TOP_HASH
        );
        let storage = MockStorage::default();
        storage
            .write_byte_stream(PathBuf::from(manifest_key), ByteStream::from_static(b"foo"))
            .await?;

        let remote = MockRemote::default();
        let dummy_manifest = r#"{"version": "v0"}"#;
        remote
            .put_object(
                &None,
                &S3Uri::try_from("s3://b/.quilt/packages/__FOO__")?,
                dummy_manifest.as_bytes().to_vec(),
            )
            .await?;
        remote
            .put_object(
                &None,
                &S3Uri::try_from("s3://b/.quilt/named_packages/a/c/latest")?,
                b"abcdef".to_vec(),
            )
            .await?;
        let mut manifest = Manifest::default();
        manifest.header.user_meta = Some(serde_json::Value::Null);
        let lineage = push_package(
            lineage,
            manifest,
            &paths::DomainPaths::default(),
            &storage,
            &remote,
            None,
            HostConfig::default(),
        )
        .await?;
        let manifest_uri = ManifestUri {
            bucket: "b".to_string(),
            namespace: ("a", "c").into(),
            hash: fixtures::top_hash::EMPTY_NULL_TOP_HASH.to_string(),
            origin: None,
        };
        assert_eq!(
            lineage,
            PackageLineage {
                remote_uri: Some(manifest_uri),
                base_hash: "".to_string(), // Huh?
                latest_hash: "abcdef".to_string(),
                ..PackageLineage::default()
            }
        );
        Ok(())
    }

    #[test(tokio::test)]
    async fn test_push_virtual_manifest() -> Res {
        let manifest_uri = ManifestUri {
            bucket: "b".to_string(),
            namespace: ("f", "a").into(),
            hash: "hash-we-later-rewrite-with-push".to_string(),
            origin: None,
        };
        let lineage = PackageLineage {
            commit: Some(CommitState {
                timestamp: chrono::Utc::now(),
                hash: fixtures::manifest::TOP_HASH.to_string(),
                prev_hashes: Vec::new(),
            }),
            remote_uri: Some(manifest_uri),
            ..PackageLineage::default()
        };
        let storage = MockStorage::default();
        let remote = MockRemote::default();
        let dummy_manifest = r#"{"version": "v0"}"#;
        remote
            .put_object(
                &None,
                &S3Uri::try_from("s3://b/.quilt/packages/hash-we-later-rewrite-with-push")?,
                dummy_manifest.as_bytes().to_vec(),
            )
            .await?;
        remote
            .put_object(
                &None,
                &S3Uri::try_from("s3://b/.quilt/named_packages/f/a/latest")?,
                b"latest-hash-abcdef".to_vec(),
            )
            .await?;

        let mut manifest = Manifest::default();
        manifest.header.message = Some("Initial".to_string());
        manifest.header.user_meta = None;

        let file_content = b"Thu Feb 29 19:07:56 PST 2024\n";

        for i in 0..10 {
            let file_path = PathBuf::from(format!("/b/a/r{}", i));
            remote
                .storage
                .write_byte_stream(&file_path, ByteStream::from_static(file_content))
                .await?;

            manifest
                .insert_record(ManifestRow {
                    logical_key: PathBuf::from(format!("e0-{}.txt", i)),
                    physical_key: format!("file://{}", file_path.display()),
                    hash: crate::checksum::Sha256ChunkedHash::try_from(
                        "/UMjH1bsbrMLBKdd9cqGGvtjhWzawhz1BfrxgngUhVI=",
                    )?
                    .into(),
                    size: file_content.len() as u64,
                    meta: Some(serde_json::Value::Null),
                })
                .await?;
        }

        let lineage = push_package(
            lineage,
            manifest,
            &paths::DomainPaths::default(),
            &storage,
            &remote,
            None,
            HostConfig::default(),
        )
        .await?;
        let manifest_uri = ManifestUri {
            bucket: "b".to_string(),
            namespace: ("f", "a").into(),
            hash: fixtures::manifest::TOP_HASH.to_string(),
            origin: None,
        };
        assert_eq!(
            lineage,
            PackageLineage {
                remote_uri: Some(manifest_uri),
                base_hash: "".to_string(), // Huh?
                latest_hash: "latest-hash-abcdef".to_string(),
                ..PackageLineage::default()
            }
        );
        Ok(())
    }
}
