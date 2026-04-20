use tokio_stream::StreamExt;
use tracing::debug;
use tracing::info;
use url::Url;

use crate::checksum::calculate_hash;
use crate::error::PackageOpError;
use crate::io::manifest::build_manifest_from_rows_stream;
use crate::io::manifest::RowsStream;
use crate::io::remote::HostConfig;
use crate::io::storage::Storage;
use crate::lineage::CommitState;
use crate::lineage::PackageLineage;
use crate::manifest::Manifest;
use crate::manifest::ManifestHeader;
use crate::manifest::ManifestRow;
use crate::manifest::Workflow;
use crate::paths::DomainPaths;
use crate::uri::Namespace;
use crate::Error;
use crate::Res;

/// Re-hash all rows from the manifest stream, converting any rows whose
/// hash algorithm doesn't match `host_config` to the correct algorithm.
/// Rows that already use the correct algorithm are passed through unchanged.
async fn rehash_rows<'a>(
    storage: &'a (impl Storage + Sync),
    manifest: &'a Manifest,
    host_config: &'a HostConfig,
) -> impl RowsStream + 'a {
    let target_algorithm = host_config.checksums.algorithm_code();
    let stream = manifest.records_stream().await;

    stream.then(move |chunk_result| async move {
        let chunk = chunk_result?;
        let mut output = Vec::new();
        for row_result in chunk {
            let row = row_result?;
            if row.hash.algorithm() == target_algorithm {
                debug!(
                    "✔️ Row already uses correct algorithm: {}",
                    row.logical_key.display()
                );
                output.push(Ok(row));
            } else {
                debug!(
                    "⏳ Re-hashing row with remote algorithm: {}",
                    row.logical_key.display()
                );
                let local_url = Url::parse(&row.physical_key).map_err(|e| {
                    Error::PackageOp(PackageOpError::Commit(format!(
                        "Invalid physical_key URL: {e}"
                    )))
                })?;
                let file_path = local_url.to_file_path().map_err(|_| {
                    Error::PackageOp(PackageOpError::Commit(format!(
                        "Cannot convert to file path: {local_url}"
                    )))
                })?;
                let rehashed =
                    calculate_hash(storage, &file_path, &row.logical_key, host_config).await?;
                output.push(Ok(ManifestRow {
                    physical_key: row.physical_key,
                    meta: row.meta,
                    ..rehashed
                }));
            }
        }
        Ok(output)
    })
}

/// Re-commit the current local manifest using the remote's `HostConfig`
/// and workflow. This ensures the commit's top hash matches what push
/// will produce after uploading rows.
///
/// Called automatically by `set_remote` so the user can push immediately
/// without a manual re-commit.
pub async fn recommit_for_remote(
    mut lineage: PackageLineage,
    manifest: &Manifest,
    paths: &DomainPaths,
    storage: &(impl Storage + Sync),
    namespace: Namespace,
    host_config: HostConfig,
    workflow: Option<Workflow>,
) -> Res<PackageLineage> {
    let old_commit = match lineage.commit.take() {
        Some(commit) => commit,
        None => return Ok(lineage),
    };

    info!("⏳ Re-committing package for remote (rehashing with remote config)");

    let header = ManifestHeader {
        message: manifest.header.message.clone(),
        user_meta: manifest.header.user_meta.clone(),
        workflow,
        ..ManifestHeader::default()
    };

    let stream = Box::pin(rehash_rows(storage, manifest, &host_config).await);
    let dest_dir = paths.installed_manifests_dir(&namespace);
    let (_manifest_path, new_top_hash) =
        build_manifest_from_rows_stream(storage, dest_dir, header, stream).await?;

    info!(
        "✔️ Re-committed with new hash: {} (was: {})",
        new_top_hash, old_commit.hash
    );

    let mut prev_hashes = vec![old_commit.hash];
    prev_hashes.extend(old_commit.prev_hashes);

    lineage.commit = Some(CommitState {
        hash: new_top_hash,
        timestamp: chrono::Utc::now(),
        prev_hashes,
    });

    Ok(lineage)
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_log::test;

    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use aws_sdk_s3::primitives::ByteStream;

    use crate::io::storage::mocks::MockStorage;
    use crate::lineage::PathState;

    /// Helper: create a local package with a single file using the given host_config,
    /// returning (lineage, manifest, storage, paths, namespace).
    async fn create_test_package(
        host_config: &HostConfig,
    ) -> Res<(
        PackageLineage,
        Manifest,
        MockStorage,
        DomainPaths,
        Namespace,
    )> {
        let storage = MockStorage::default();
        let (paths, _temp) = DomainPaths::from_temp_dir()?;
        let namespace: Namespace = ("test", "pkg").into();

        // Create directories
        let installed_dir = paths.installed_manifests_dir(&namespace);
        storage.create_dir_all(&installed_dir).await?;
        let objects_dir = paths.objects_dir();
        storage.create_dir_all(&objects_dir).await?;

        // Create a test file in objects
        let file_content = b"hello world";
        let logical_key = PathBuf::from("data.txt");

        // Write to a temp location for hashing
        let temp_file = objects_dir.join("temp_data");
        storage
            .write_byte_stream(&temp_file, ByteStream::from_static(file_content))
            .await?;

        let row = calculate_hash(&storage, &temp_file, &logical_key, host_config).await?;
        let object_dest = objects_dir.join(hex::encode(row.hash.digest()));
        storage.copy(&temp_file, &object_dest).await?;

        let physical_key = url::Url::from_file_path(&object_dest).unwrap().to_string();
        let manifest_row = ManifestRow {
            physical_key,
            ..row.clone()
        };

        // Build manifest
        let header = ManifestHeader {
            message: Some("Initial commit".to_string()),
            user_meta: Some(serde_json::json!({"key": "value"})),
            ..ManifestHeader::default()
        };
        let stream = tokio_stream::iter(vec![Ok(vec![Ok(manifest_row)])]);
        let (_path, top_hash) =
            build_manifest_from_rows_stream(&storage, installed_dir, header, stream).await?;

        let commit = CommitState {
            hash: top_hash,
            timestamp: chrono::Utc::now(),
            prev_hashes: Vec::new(),
        };

        let lineage = PackageLineage {
            commit: Some(commit),
            remote_uri: None,
            base_hash: String::new(),
            latest_hash: String::new(),
            paths: BTreeMap::from([(
                logical_key,
                PathState {
                    timestamp: chrono::Utc::now(),
                    hash: row.hash.into(),
                },
            )]),
        };

        let manifest_path =
            paths.installed_manifest(&namespace, &lineage.commit.as_ref().unwrap().hash);
        let manifest = Manifest::from_path(&storage, &manifest_path).await?;

        Ok((lineage, manifest, storage, paths, namespace))
    }

    #[test(tokio::test)]
    async fn test_recommit_changes_algorithm() -> Res {
        let sha256_config = HostConfig::default_sha256_chunked();
        let crc64_config = HostConfig::default_crc64();

        let (lineage, manifest, storage, paths, namespace) =
            create_test_package(&sha256_config).await?;

        let old_hash = lineage.commit.as_ref().unwrap().hash.clone();

        let lineage = recommit_for_remote(
            lineage,
            &manifest,
            &paths,
            &storage,
            namespace,
            crc64_config,
            None,
        )
        .await?;

        let commit = lineage.commit.as_ref().unwrap();
        assert_ne!(commit.hash, old_hash, "Top hash should change");
        assert_eq!(
            commit.prev_hashes,
            vec![old_hash],
            "Old hash should be in prev_hashes"
        );

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_recommit_no_commit() -> Res {
        let storage = MockStorage::default();
        let (paths, _temp) = DomainPaths::from_temp_dir()?;
        let namespace: Namespace = ("test", "pkg").into();

        let lineage = PackageLineage::default();
        assert!(lineage.commit.is_none());

        let result = recommit_for_remote(
            lineage,
            &Manifest::default(),
            &paths,
            &storage,
            namespace,
            HostConfig::default(),
            None,
        )
        .await?;

        assert!(result.commit.is_none(), "Should return lineage as-is");
        Ok(())
    }

    #[test(tokio::test)]
    async fn test_recommit_same_algorithm() -> Res {
        let sha256_config = HostConfig::default_sha256_chunked();

        let (lineage, manifest, storage, paths, namespace) =
            create_test_package(&sha256_config).await?;

        let old_hash = lineage.commit.as_ref().unwrap().hash.clone();

        let lineage = recommit_for_remote(
            lineage,
            &manifest,
            &paths,
            &storage,
            namespace,
            sha256_config,
            None,
        )
        .await?;

        let commit = lineage.commit.as_ref().unwrap();
        // When same algorithm and no workflow change, hash should still differ
        // because the header is rebuilt (workflow field changes from None to None,
        // but the manifest is rebuilt from scratch which may produce same hash)
        // Actually with identical inputs, the top hash should be the same
        assert_eq!(
            commit.prev_hashes,
            vec![old_hash.clone()],
            "Old hash should be in prev_hashes even if hash didn't change"
        );

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_recommit_preserves_message_and_meta() -> Res {
        let sha256_config = HostConfig::default_sha256_chunked();
        let crc64_config = HostConfig::default_crc64();

        let (lineage, manifest, storage, paths, namespace) =
            create_test_package(&sha256_config).await?;

        let lineage = recommit_for_remote(
            lineage,
            &manifest,
            &paths,
            &storage,
            namespace.clone(),
            crc64_config,
            None,
        )
        .await?;

        // Read the new manifest and verify header
        let commit = lineage.commit.as_ref().unwrap();
        let manifest_path = paths.installed_manifest(&namespace, &commit.hash);
        let new_manifest = Manifest::from_path(&storage, &manifest_path).await?;

        assert_eq!(
            new_manifest.header.message,
            Some("Initial commit".to_string()),
            "Message should be preserved"
        );
        assert_eq!(
            new_manifest.header.user_meta,
            Some(serde_json::json!({"key": "value"})),
            "User meta should be preserved"
        );

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_recommit_adds_workflow() -> Res {
        let host_config = HostConfig::default_sha256_chunked();

        let (lineage, manifest, storage, paths, namespace) =
            create_test_package(&host_config).await?;

        let workflow = Workflow {
            config: "s3://bucket/.quilt/workflows/config.yml".parse()?,
            id: Some(crate::manifest::WorkflowId {
                id: "test-workflow".to_string(),
                metadata: None,
            }),
        };

        let lineage = recommit_for_remote(
            lineage,
            &manifest,
            &paths,
            &storage,
            namespace.clone(),
            host_config,
            Some(workflow),
        )
        .await?;

        let commit = lineage.commit.as_ref().unwrap();
        let manifest_path = paths.installed_manifest(&namespace, &commit.hash);
        let new_manifest = Manifest::from_path(&storage, &manifest_path).await?;

        assert!(
            new_manifest.header.workflow.is_some(),
            "Workflow should be set"
        );
        assert_eq!(
            new_manifest
                .header
                .workflow
                .as_ref()
                .unwrap()
                .id
                .as_ref()
                .unwrap()
                .id,
            "test-workflow"
        );

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_recommit_empty_package() -> Res {
        let crc64_config = HostConfig::default_crc64();

        let storage = MockStorage::default();
        let (paths, _temp) = DomainPaths::from_temp_dir()?;
        let namespace: Namespace = ("test", "empty").into();

        let installed_dir = paths.installed_manifests_dir(&namespace);
        storage.create_dir_all(&installed_dir).await?;

        // Build empty manifest
        let header = ManifestHeader {
            message: Some("Empty package".to_string()),
            ..ManifestHeader::default()
        };
        let stream = tokio_stream::iter(vec![Ok(Vec::<Res<ManifestRow>>::new())]);
        let (_path, top_hash) =
            build_manifest_from_rows_stream(&storage, installed_dir, header, stream).await?;

        let lineage = PackageLineage {
            commit: Some(CommitState {
                hash: top_hash.clone(),
                timestamp: chrono::Utc::now(),
                prev_hashes: Vec::new(),
            }),
            ..PackageLineage::default()
        };

        let manifest_path = paths.installed_manifest(&namespace, &top_hash);
        let manifest = Manifest::from_path(&storage, &manifest_path).await?;

        let result = recommit_for_remote(
            lineage,
            &manifest,
            &paths,
            &storage,
            namespace,
            crc64_config,
            None,
        )
        .await?;

        assert!(result.commit.is_some());
        assert_eq!(result.commit.as_ref().unwrap().prev_hashes, vec![top_hash]);

        Ok(())
    }
}
