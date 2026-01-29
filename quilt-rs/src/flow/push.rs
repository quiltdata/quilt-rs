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
use crate::manifest::Row;
use crate::manifest::Table;
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
    remote_manifest: &Table,
    rows: StreamItem,
) -> StreamItem {
    let mut output = Vec::new();
    for row in rows? {
        let row = row?;
        debug!("⏳ Processing row: {}", row.name.display());
        if let Ok(Some(remote_row)) = remote_manifest.get_record(&row.name).await {
            if remote_row == row {
                debug!("✔️ Using existing remote row for: {}", row.name.display());
                output.push(Ok(Row {
                    place: remote_row.place.to_owned(),
                    ..row.clone()
                }));
            } else {
                debug!("⏳ Uploading modified row for: {}", row.name.display());
                output.push(upload_row(remote, host_config, package_handle.clone(), row).await)
            }
        } else {
            debug!("⏳ Uploading new row for: {}", row.name.display());
            output.push(upload_row(remote, host_config, package_handle.clone(), row).await)
        }
    }
    Ok(output)
}

async fn stream_uploaded_local_rows<'a>(
    remote: &'a impl Remote,
    host_config: &'a HostConfig,
    local_manifest: &'a Table,
    remote_manifest: &'a Table,
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
    local_manifest: Table,
    paths: &paths::DomainPaths,
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    namespace: Option<Namespace>,
    host_config: HostConfig,
) -> Res<PackageLineage> {
    let commit = match lineage.commit {
        None => {
            info!("No changes to push");
            return Ok(lineage); // nothing to commit
        }
        Some(commit) => commit,
    };

    debug!("⏳ Fetching remote manifest");
    let remote_manifest = flow::browse(paths, storage, remote, &lineage.remote).await?;
    debug!("✔️ Remote manifest fetched");

    // ## copy data
    // Copy each of the _modified_ paths from their local_key to remote_key,
    // keeping track of the resulting versionIds
    //
    // TODO: FAIL if the remote bucket does NOT support versioning (as it would be destructive)

    let namespace = namespace.unwrap_or(lineage.remote.namespace.clone());

    debug!("⏳ Creating manifest URI");
    let manifest_uri = ManifestUri {
        namespace,
        ..lineage.remote.clone()
    };
    debug!("✔️ Created manifest URI: {}", manifest_uri.display());

    debug!("⏳ Building and uploading manifest");
    let header = local_manifest.get_header().await?;
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
    let dest_dir = paths.manifest_cache_dir(&manifest_uri.bucket);
    let (cache_path, top_hash) =
        build_manifest_from_rows_stream(storage, dest_dir, header, stream).await?;
    debug!(
        "✔️ Built manifest with hash {} at {}",
        top_hash,
        cache_path.display()
    );

    let new_manifest_uri = ManifestUri {
        hash: top_hash,
        ..lineage.remote.clone()
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

    lineage.remote = new_manifest_uri.clone();
    lineage.commit = None;

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

    use crate::fixtures;
    use crate::io::remote::mocks::MockRemote;
    use crate::io::storage::mocks::MockStorage;
    use crate::lineage::CommitState;
    use crate::lineage::PackageLineage;
    use crate::manifest::Row;
    use crate::uri::S3Uri;

    #[test(tokio::test)]
    async fn test_no_push_if_no_commit() -> Res {
        let storage = MockStorage::default();
        let remote = MockRemote::default();
        let lineage = push_package(
            PackageLineage::default(),
            Table::default(),
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
                hash: fixtures::manifest_empty::EMPTY_NULL_TOP_HASH.to_string(),
                prev_hashes: Vec::new(),
            }),
            remote: manifest_uri,
            ..PackageLineage::default()
        };
        let jsonl = std::fs::read(fixtures::manifest::parquet_checksummed()?)?;
        let manifest_key = format!(
            ".quilt/packages/b/{}",
            fixtures::manifest_empty::EMPTY_NULL_TOP_HASH
        );
        let storage = MockStorage::default();
        storage
            .write_file(PathBuf::from(manifest_key), &jsonl)
            .await?;

        let remote = MockRemote::default();
        remote
            .put_object(
                &None,
                &S3Uri::try_from("s3://b/.quilt/packages/1220__FOO__.parquet")?,
                jsonl,
            )
            .await?;
        remote
            .put_object(
                &None,
                &S3Uri::try_from("s3://b/.quilt/named_packages/a/c/latest")?,
                b"abcdef".to_vec(),
            )
            .await?;
        let table = fixtures::manifest_empty::empty_null();
        let lineage = push_package(
            lineage,
            table,
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
            hash: fixtures::manifest_empty::EMPTY_NULL_TOP_HASH.to_string(),
            origin: None,
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

    #[test(tokio::test)]
    async fn test_single_chunk_push() -> Res {
        let manifest_uri = ManifestUri {
            bucket: "b".to_string(),
            namespace: ("f", "a").into(),
            hash: "__FOO__".to_string(),
            origin: None,
        };
        let lineage = PackageLineage {
            commit: Some(CommitState {
                timestamp: chrono::Utc::now(),
                hash: fixtures::manifest::PARQUEST_CHECKSUMMED_HASH.to_string(),
                prev_hashes: Vec::new(),
            }),
            remote: manifest_uri,
            ..PackageLineage::default()
        };
        let jsonl = std::fs::read(fixtures::manifest::parquet_checksummed()?)?;
        let manifest_key = format!(
            ".quilt/packages/b/{}",
            fixtures::manifest::PARQUEST_CHECKSUMMED_HASH
        );
        let storage = MockStorage::default();
        storage
            .write_file(PathBuf::from(manifest_key), &jsonl)
            .await?;
        let remote = MockRemote::default();
        remote
            .put_object(
                &None,
                &S3Uri::try_from("s3://b/.quilt/packages/1220__FOO__.parquet")?,
                jsonl,
            )
            .await?;
        remote
            .put_object(
                &None,
                &S3Uri::try_from("s3://b/.quilt/named_packages/f/a/latest")?,
                b"abcdef".to_vec(),
            )
            .await?;

        let file_path = PathBuf::from("/b/a/r");
        let manifest_file = std::fs::read(fixtures::manifest::parquet_checksummed()?)?;
        remote
            .storage
            .write_file(&file_path, &manifest_file)
            .await?;

        let mut manifest = Table::default();
        manifest.header.meta = Some(serde_json::Value::Null);
        manifest
            .insert_record(Row {
                name: PathBuf::from("bar"),
                place: format!("file://{}", file_path.display()),
                ..Row::default()
            })
            .await?;

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
            hash: fixtures::manifest::PARQUEST_CHECKSUMMED_HASH.to_string(),
            origin: None,
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

    #[test(tokio::test)]
    #[ignore]
    async fn test_multichunk_push() -> Res {
        // TODO
        Ok(())
    }
}
