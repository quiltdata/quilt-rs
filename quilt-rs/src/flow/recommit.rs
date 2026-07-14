use tokio_stream::StreamExt;
use tracing::debug;
use tracing::info;
use url::Url;

use crate::Error;
use crate::Res;
use crate::checksum::calculate_hash;
use crate::error::PackageOpError;
use crate::io::manifest::RowsStream;
use crate::io::manifest::build_manifest_from_rows_stream;
use crate::io::remote::HostConfig;
use crate::io::remote::Remote;
use crate::io::remote::WorkflowsConfig;
use crate::io::remote::entry_view;
use crate::io::remote::validate_workflow;
use crate::io::remote::validate_workflow_with_config;
use crate::io::storage::Storage;
use crate::lineage::CommitState;
use crate::lineage::PackageLineage;
use crate::manifest::Manifest;
use crate::manifest::ManifestHeader;
use crate::manifest::ManifestRow;
use crate::manifest::Workflow;
use crate::paths::DomainPaths;
use crate::workflow::EntryView;
use quilt_uri::Host;
use quilt_uri::Namespace;

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
                let file_path = local_url.to_file_path().map_err(|()| {
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
#[allow(clippy::too_many_arguments)]
pub async fn recommit_for_remote(
    mut lineage: PackageLineage,
    manifest: &Manifest,
    paths: &DomainPaths,
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    host: &Option<Host>,
    namespace: Namespace,
    host_config: HostConfig,
    workflow: Option<Workflow>,
    workflows_config: Option<&WorkflowsConfig>,
) -> Res<PackageLineage> {
    let Some(old_commit) = lineage.commit.take() else {
        return Ok(lineage);
    };

    info!("⏳ Re-committing package for remote (rehashing with remote config)");

    let header = ManifestHeader {
        message: manifest.header.message.clone(),
        user_meta: manifest.header.user_meta.clone(),
        workflow,
        ..ManifestHeader::default()
    };

    // Materialize the rehashed rows so the workflow gate can inspect them
    // before anything is written. The manifest's rows are already sorted by
    // logical key, and `rehash_rows` preserves that order.
    let mut rows: Vec<Res<ManifestRow>> = Vec::new();
    let mut stream = Box::pin(rehash_rows(storage, manifest, &host_config).await);
    while let Some(chunk_result) = stream.next().await {
        rows.extend(chunk_result?);
    }

    // The workflow quality gate: reject a revision the resolved workflow would
    // refuse before the recommit writes any manifest, so `set_remote` cannot
    // stamp a workflow that would only be rejected later at push. Vacuously
    // passes for an ungoverned bucket (the header carries no workflow).
    let entries: Vec<EntryView> = rows
        .iter()
        .filter_map(|row| row.as_ref().ok())
        .map(entry_view)
        .collect();
    // `set_remote` passes the config it already fetched to resolve the workflow;
    // reuse it here so the gate does not re-download the same config. At this
    // moment the header's pinned config URI addresses exactly that object (the
    // resolution just stamped it), so reusing the parsed config is semantically
    // identical to re-fetching. Callers without a pre-fetched config pass
    // `None`, and the gate fetches via the header's pinned URI as before.
    match workflows_config {
        Some(config) => {
            validate_workflow_with_config(
                remote,
                host,
                &namespace.to_string(),
                header.message.as_deref(),
                header.user_meta.as_ref(),
                header.workflow.as_ref(),
                Some(config),
                &entries,
            )
            .await?;
        }
        None => {
            validate_workflow(
                remote,
                host,
                &namespace.to_string(),
                header.message.as_deref(),
                header.user_meta.as_ref(),
                header.workflow.as_ref(),
                &entries,
            )
            .await?;
        }
    }

    let stream = tokio_stream::iter(vec![Ok(rows)]);
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

    use crate::io::remote::mocks::MockRemote;
    use crate::io::storage::mocks::MockStorage;
    use crate::lineage::PathState;

    /// Helper: create a local package with a single file using the given `host_config`,
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
            &MockRemote::default(),
            &None,
            namespace,
            crc64_config,
            None,
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
            &MockRemote::default(),
            &None,
            namespace,
            HostConfig::default(),
            None,
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
            &MockRemote::default(),
            &None,
            namespace,
            sha256_config,
            None,
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
            &MockRemote::default(),
            &None,
            namespace.clone(),
            crc64_config,
            None,
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
                schemas: BTreeMap::new(),
            }),
        };

        let lineage = recommit_for_remote(
            lineage,
            &manifest,
            &paths,
            &storage,
            &MockRemote::default(),
            &None,
            namespace.clone(),
            host_config,
            Some(workflow),
            None,
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
            &MockRemote::default(),
            &None,
            namespace,
            crc64_config,
            None,
            None,
        )
        .await?;

        assert!(result.commit.is_some());
        assert_eq!(result.commit.as_ref().unwrap().prev_hashes, vec![top_hash]);

        Ok(())
    }

    /// Build an empty committed package (message "msg", no `user_meta`) on the
    /// given storage under `/foo`-rooted paths, returning its lineage and
    /// manifest. Empty on purpose: with no rows there are no storage-specific
    /// physical keys, so two builds on different storages produce byte-identical
    /// manifests — which lets the rejection test predict the exact top hash a
    /// successful recommit would have written.
    async fn empty_committed_package(
        storage: &MockStorage,
        paths: &DomainPaths,
        namespace: &Namespace,
    ) -> Res<(PackageLineage, Manifest)> {
        let installed_dir = paths.installed_manifests_dir(namespace);
        storage.create_dir_all(&installed_dir).await?;

        let header = ManifestHeader {
            message: Some("msg".to_string()),
            ..ManifestHeader::default()
        };
        let stream = tokio_stream::iter(vec![Ok(Vec::<Res<ManifestRow>>::new())]);
        let (_path, top_hash) =
            build_manifest_from_rows_stream(storage, installed_dir, header, stream).await?;

        let lineage = PackageLineage {
            commit: Some(CommitState {
                hash: top_hash.clone(),
                timestamp: chrono::Utc::now(),
                prev_hashes: Vec::new(),
            }),
            ..PackageLineage::default()
        };
        let manifest_path = paths.installed_manifest(namespace, &top_hash);
        let manifest = Manifest::from_path(storage, &manifest_path).await?;
        Ok((lineage, manifest))
    }

    /// The recommit-side workflow gate: a workflow whose `metadata_schema`
    /// requires an `owner` must reject a recommit of a package without it,
    /// and must reject *before* any manifest is written — nothing is
    /// recommitted (mirrors the commit gate's writes-nothing test).
    #[test(tokio::test)]
    async fn test_recommit_rejected_by_workflow_writes_nothing() -> Res {
        use crate::manifest::WorkflowId;
        use crate::workflow::WorkflowValidationError;
        use quilt_uri::S3Uri;

        let paths = DomainPaths::new(PathBuf::from("/foo"));
        let namespace: Namespace = ("test", "rejected").into();
        let config_uri: S3Uri = "s3://b/.quilt/workflows/config.yml".parse()?;
        let workflow = Workflow {
            config: config_uri.clone(),
            id: Some(WorkflowId {
                id: "gate".to_string(),
                schemas: BTreeMap::new(),
            }),
        };

        // Learn the top hash a successful recommit would write: same workflow
        // reference against an unconstrained config produces byte-identical
        // manifest bytes, so this is the exact hash that must be absent from
        // storage after a rejection.
        let ok_remote = MockRemote::default();
        ok_remote
            .put_object(
                &None,
                &config_uri,
                b"version: \"1\"\nworkflows:\n  gate:\n    name: Gate\n".to_vec(),
            )
            .await?;
        let ok_storage = MockStorage::default();
        let (lineage, manifest) = empty_committed_package(&ok_storage, &paths, &namespace).await?;
        let ok_lineage = recommit_for_remote(
            lineage,
            &manifest,
            &paths,
            &ok_storage,
            &ok_remote,
            &None,
            namespace.clone(),
            HostConfig::default(),
            Some(workflow.clone()),
            None,
        )
        .await?;
        let would_be_hash = ok_lineage.commit.as_ref().unwrap().hash.clone();

        // The governed config: `gate`'s metadata_schema requires an `owner`,
        // which the package's (absent) metadata does not provide.
        let remote = MockRemote::default();
        remote
            .put_object(
                &None,
                &config_uri,
                b"version: \"1\"\nworkflows:\n  gate:\n    name: Gate\n    metadata_schema: meta\nschemas:\n  meta:\n    url: s3://b/schemas/meta.json\n".to_vec(),
            )
            .await?;
        remote
            .put_object(
                &None,
                &"s3://b/schemas/meta.json".parse()?,
                br#"{"type": "object", "required": ["owner"]}"#.to_vec(),
            )
            .await?;

        let storage = MockStorage::default();
        let (lineage, manifest) = empty_committed_package(&storage, &paths, &namespace).await?;
        let old_hash = lineage.commit.as_ref().unwrap().hash.clone();
        let err = recommit_for_remote(
            lineage,
            &manifest,
            &paths,
            &storage,
            &remote,
            &None,
            namespace.clone(),
            HostConfig::default(),
            Some(workflow),
            None,
        )
        .await
        .unwrap_err();

        assert!(
            matches!(
                err,
                Error::WorkflowValidation(WorkflowValidationError::Rejected(_))
            ),
            "expected a workflow rejection, got: {err:?}"
        );
        assert!(
            !storage
                .exists(&paths.installed_manifest(&namespace, &would_be_hash))
                .await,
            "a rejected recommit must not write a manifest"
        );
        assert!(
            storage
                .exists(&paths.installed_manifest(&namespace, &old_hash))
                .await,
            "the previous manifest must remain intact"
        );

        Ok(())
    }
}
