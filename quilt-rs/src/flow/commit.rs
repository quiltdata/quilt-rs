use std::collections::BTreeMap;
use std::collections::HashSet;
use std::path::Path;
use std::path::PathBuf;

use tokio_stream::StreamExt;
use tracing::debug;
use tracing::info;
use url::Url;

use crate::Error;
use crate::Res;
use crate::error::ManifestError;
use crate::error::PackageOpError;
use crate::io::manifest::StreamRowsChunk;
use crate::io::manifest::build_manifest_from_rows_stream;
use crate::io::remote::Remote;
use crate::io::remote::entry_view;
use crate::io::remote::validate_workflow;
use crate::io::storage::Storage;
use crate::lineage::Change;
use crate::lineage::CommitState;
use crate::lineage::InstalledPackageStatus;
use crate::lineage::PackageLineage;
use crate::lineage::PathState;
use crate::manifest::Manifest;
use crate::manifest::ManifestHeader;
use crate::manifest::ManifestRow;
use crate::manifest::Workflow;
use crate::paths::DomainPaths;
use crate::workflow::EntryView;
use quilt_uri::Host;
use quilt_uri::Namespace;

/// Merge the stored manifest with this revision's changes into the sorted
/// row set the new manifest is built from.
///
/// Returns the rows materialized (the caller runs the workflow gate over them
/// before writing), then wraps them back into a single-chunk stream for
/// [`build_manifest_from_rows_stream`].
async fn collect_local_with_changes(
    local_manifest: &Manifest,
    removed: HashSet<PathBuf>,
    modified: BTreeMap<PathBuf, ManifestRow>,
    new_files: StreamRowsChunk,
) -> StreamRowsChunk {
    // Collect all rows from the local manifest stream
    let mut all_rows: Vec<Res<ManifestRow>> = Vec::new();

    // Add new files to the collection
    all_rows.extend(new_files);

    // Process and add existing rows from the manifest
    let mut stream = local_manifest.records_stream().await;
    while let Some(chunk_result) = stream.next().await {
        if let Ok(chunk) = chunk_result {
            for row_res in chunk {
                match row_res {
                    Ok(row) => {
                        // Skip removed rows
                        if removed.contains(&row.logical_key) {
                            continue;
                        }

                        // Use modified version if available, otherwise use original
                        if let Some(modified_row) = modified.get(&row.logical_key) {
                            all_rows.push(Ok(modified_row.clone()));
                        } else {
                            all_rows.push(Ok(row));
                        }
                    }
                    Err(err) => {
                        all_rows.push(Err(Error::Manifest(ManifestError::Table(err.to_string()))));
                    }
                }
            }
        }
    }

    // Sort all rows by name
    all_rows.sort_by(|a, b| match (a, b) {
        (Ok(row_a), Ok(row_b)) => row_a.logical_key.cmp(&row_b.logical_key),
        (Ok(_), Err(_)) => std::cmp::Ordering::Less,
        (Err(_), Ok(_)) => std::cmp::Ordering::Greater,
        (Err(_), Err(_)) => std::cmp::Ordering::Equal,
    });

    all_rows
}

async fn create_immutable_object_copy(
    storage: &impl Storage,
    paths: &DomainPaths,
    working_dir: &Path,
    lineage: &mut PackageLineage,
    logical_key: &PathBuf,
    current: ManifestRow,
) -> Res<ManifestRow> {
    debug!(
        "⏳ Creating immutable object copy for: {}",
        logical_key.display()
    );
    let objects_dir = paths.objects_dir();
    let object_dest = objects_dir.join(hex::encode(current.hash.digest()));
    let new_physical_key = Url::from_file_path(&object_dest)
        .map_err(|()| {
            Error::PackageOp(PackageOpError::Commit(format!(
                "Failed to create URL from {}",
                object_dest.display()
            )))
        })?
        .to_string();

    let current_hash = current.hash.clone();
    let row = ManifestRow {
        logical_key: logical_key.clone(),
        physical_key: new_physical_key,
        ..current
    };

    let work_dest = working_dir.join(logical_key);

    if storage.exists(&object_dest).await {
        debug!(
            "✔️ Object already exists in storage: {}",
            object_dest.display()
        );
    } else {
        debug!(
            "⏳ Copying file to objects directory: {}",
            object_dest.display()
        );
        storage.copy(&work_dest, object_dest).await?;
        debug!("✔️ File copied successfully");
    }
    lineage.paths.insert(
        logical_key.clone(),
        PathState {
            timestamp: storage.modified_timestamp(&work_dest).await?,
            hash: current_hash.into(),
        },
    );
    Ok(row)
}

// TODO
// pub struct Commit {
//[     message: Option<String>,
//     user_meta: Option<serde_json::Value>,
//     workflow: Option<Workflow>,
// }

/// What to do with the package-level metadata of the revision being committed.
///
/// The manifest header stores metadata as an optional JSON value; this enum
/// is the caller-facing contract for changing it. `Keep` carries the previous
/// revision's metadata forward verbatim. `Clear` produces a header without a
/// `user_meta` field. `Set` replaces it (objects are stored with sorted keys).
#[derive(Debug, Clone, PartialEq)]
pub enum UserMeta {
    Keep,
    Clear,
    Set(serde_json::Value),
}

/// Commit new commit with new `message`, `user_meta` and all changes got from calling `flow::status`
///
/// `user_meta` selects the new revision's package-level metadata — see
/// [`UserMeta`]: `Keep` inherits the previous revision's, `Clear` removes it,
/// `Set` replaces it.
///
/// On `Ok`, the returned `CommitState` is also stored in `lineage.commit` —
/// callers that need the new top hash should read it from the tuple rather
/// than unwrapping `lineage.commit`.
// TODO: move `working_dir` to `paths`, and `paths` to `storage`
#[allow(clippy::too_many_arguments)]
#[allow(
    clippy::too_many_lines,
    reason = "cohesive commit orchestration; clearer as a linear sequence than extracted helpers"
)]
pub async fn commit_package(
    mut lineage: PackageLineage,
    manifest: &mut Manifest,
    paths: &DomainPaths,
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    host: &Option<Host>,
    working_dir: PathBuf,
    status: InstalledPackageStatus,
    namespace: Namespace,
    message: String,
    user_meta: UserMeta,
    workflow: Option<Workflow>,
) -> Res<(PackageLineage, CommitState)> {
    info!(
        r#"⏳ Starting commit with message "{}" and user_meta `{:?}`"#,
        message, user_meta
    );

    // create a new manifest based on the stored version

    // for each modified file:
    //   - compute the new hash
    //   - store in the identity cache at $LOCAL/.quilt/objects/<hash>
    //   - update the modified entries in the manifest with the new physical keys
    //     pointing to the new objects in the identity cache
    //   - ? set entry.meta.pulled_hashes to previous object hash?
    //   - ? set entry.meta.remote_key to the remote's physical key?

    // compute the new top hash
    // store the new manifest under the new top hash at $LOCAL/.quilt/packages/<hash>
    // XXX: prefix with the namespace?
    // XXX: what to do on collisions?
    //      e.g. when a file was changed, committed, and then reverted

    // store revision pointers to the newly created manifest
    //   - in the local registry??
    //   - in the lineage
    //     - commit:
    //       - timestamp
    //       - user ?
    //       - multihash: new_top_hash
    //       - pulled_hashes: [old_top_hash] ?
    //       - paths:
    //         - [modified file's path]:
    //           - multihash
    //           # XXX: do we actually need this? can be inferred from namespace + logical key
    //           - remote_key: "s3://..." # no version id
    //           - local_key: $LOCAL/.quilt/objects/<hash>
    //           - pulled_hashes: [old_hash] ?
    // NOTE: each commit MUST include all paths from prior commits
    //       (since the last pull, until reset by a sync)

    let mut modified_keys = BTreeMap::new();
    let mut removed_keys = HashSet::new();
    let mut new_files = Vec::new();
    for (logical_key, state) in status.changes {
        debug!(
            "Processing change type {:?} for: {}",
            state,
            logical_key.display()
        );
        match state {
            Change::Removed(row) => {
                lineage.paths.remove(&row.logical_key);
                removed_keys.insert(row.logical_key);
            }
            Change::Added(current) => {
                if manifest.contains_record(&current.logical_key) {
                    return Err(Error::PackageOp(PackageOpError::Commit(format!(
                        "Trying to add a file that is already in the manifest: \"{}\"",
                        current.logical_key.display()
                    ))));
                }
                let added = create_immutable_object_copy(
                    storage,
                    paths,
                    &working_dir,
                    &mut lineage,
                    &logical_key,
                    current,
                )
                .await?;
                new_files.push(Ok(added));
            }
            Change::Modified(current) => {
                let modified = create_immutable_object_copy(
                    storage,
                    paths,
                    &working_dir,
                    &mut lineage,
                    &logical_key,
                    current,
                )
                .await?;
                modified_keys.insert(logical_key.clone(), modified);
            }
        }
    }

    let user_meta = match user_meta {
        UserMeta::Keep => manifest.header.user_meta.clone(),
        UserMeta::Clear => None,
        UserMeta::Set(serde_json::Value::Object(mut m)) => {
            m.sort_keys();
            Some(m.into())
        }
        UserMeta::Set(other) => Some(other),
    };

    let header = ManifestHeader {
        message: Some(message.clone()),
        workflow,
        user_meta,
        ..ManifestHeader::default()
    };

    debug!(
        "⏳ Building new manifest with {} removed, {} modified, {} new files",
        removed_keys.len(),
        modified_keys.len(),
        new_files.len()
    );
    let all_rows =
        collect_local_with_changes(manifest, removed_keys, modified_keys, new_files).await;

    // The workflow quality gate: reject before any manifest is written, so a
    // rejected revision is never committed. Vacuously passes for an ungoverned
    // bucket (the header carries no workflow).
    let entries: Vec<EntryView> = all_rows
        .iter()
        .filter_map(|row| row.as_ref().ok())
        .map(entry_view)
        .collect();
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

    let stream = tokio_stream::iter(vec![Ok(all_rows)]);
    let dest_dir = paths.installed_manifests_dir(&namespace);
    let (manifest_path, new_top_hash) =
        build_manifest_from_rows_stream(storage, dest_dir, header, stream).await?;
    info!(
        "✔️New manifest with {} was built in {}",
        manifest_path.display(),
        new_top_hash
    );
    let mut prev_hashes = Vec::new();
    if let Some(commit) = lineage.commit {
        prev_hashes.push(commit.hash.clone());
        prev_hashes.extend(commit.prev_hashes.clone());
    }
    let commit = CommitState {
        hash: new_top_hash,
        timestamp: chrono::Utc::now(),
        prev_hashes,
    };
    lineage.commit = Some(commit.clone());

    info!(
        "✔️ Successfully committed changes with hash: {}",
        commit.hash
    );
    Ok((lineage, commit))
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_log::test;

    use std::collections::BTreeMap;

    use aws_sdk_s3::primitives::ByteStream;

    use crate::fixtures;
    use crate::io::remote::mocks::MockRemote;
    use crate::io::storage::mocks::MockStorage;
    use crate::lineage::Change;
    use crate::manifest::WorkflowId;
    use crate::workflow::WorkflowValidationError;
    use quilt_uri::S3Uri;

    // NOTE: Tests use "/" path for working directory, because it then parsed with Url and have to be absolute path

    /// A manifest whose header carries the given package-level metadata and no
    /// rows — a minimal stand-in for a previously committed revision.
    fn manifest_with_meta(user_meta: Option<serde_json::Value>) -> Manifest {
        Manifest {
            header: ManifestHeader {
                user_meta,
                ..ManifestHeader::default()
            },
            ..Manifest::default()
        }
    }

    /// The commit-side workflow gate: a governed bucket whose workflow requires
    /// an `owner` in the package metadata must reject a commit that lacks it,
    /// and must reject *before* any manifest is written — nothing is committed.
    #[test(tokio::test)]
    async fn test_commit_rejected_by_workflow_writes_nothing() -> Res {
        let paths = DomainPaths::new(PathBuf::from("/foo"));
        let namespace: Namespace = ("foo", "bar").into();
        let config_uri: S3Uri = "s3://b/.quilt/workflows/config.yml".parse()?;
        let workflow = Workflow {
            config: config_uri.clone(),
            id: Some(WorkflowId {
                id: "gate".to_string(),
                schemas: BTreeMap::new(),
            }),
        };

        // The top hash this revision *would* carry: the header records only the
        // workflow's config URI + id, not the config's contents, so a commit
        // selecting the same workflow against an unconstrained config produces
        // byte-identical manifest bytes — and thus the exact hash that must be
        // absent from storage after a rejection.
        let ok_remote = MockRemote::default();
        ok_remote
            .put_object(
                &None,
                &config_uri,
                b"version: \"1\"\nworkflows:\n  gate:\n    name: Gate\n".to_vec(),
            )
            .await?;
        let ok_storage = MockStorage::default();
        let (_lineage, ok_commit) = commit_package(
            PackageLineage::default(),
            &mut Manifest::default(),
            &paths,
            &ok_storage,
            &ok_remote,
            &None,
            PathBuf::default(),
            InstalledPackageStatus::default(),
            namespace.clone(),
            String::from("msg"),
            UserMeta::Clear,
            Some(workflow.clone()),
        )
        .await?;
        let would_be_hash = ok_commit.hash;

        // The governed config: the `gate` workflow's metadata_schema requires
        // an `owner`, which the committed metadata does not provide.
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
        let err = commit_package(
            PackageLineage::default(),
            &mut Manifest::default(),
            &paths,
            &storage,
            &remote,
            &None,
            PathBuf::default(),
            InstalledPackageStatus::default(),
            namespace.clone(),
            String::from("msg"),
            UserMeta::Clear,
            Some(workflow),
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
            "a rejected commit must not write a manifest"
        );
        Ok(())
    }

    #[test(tokio::test)]
    async fn test_commit_empty() -> Res {
        let storage = MockStorage::default();
        let paths = DomainPaths::new(PathBuf::from("/foo"));
        let lineage = PackageLineage::default();
        assert!(lineage.commit.is_none());
        let (_lineage, commit) = commit_package(
            lineage,
            &mut Manifest::default(),
            &paths,
            &storage,
            &MockRemote::default(),
            &None,
            PathBuf::default(),
            InstalledPackageStatus::default(),
            ("foo", "bar").into(),
            String::default(),
            UserMeta::Clear,
            None,
        )
        .await?;
        let hash = fixtures::top_hash::EMPTY_NONE_TOP_HASH;
        assert!(
            storage
                .exists(&paths.installed_manifest(&("foo", "bar").into(), hash))
                .await
        );
        assert_eq!(commit.hash, hash);
        Ok(())
    }

    /// Commit a package whose previous revision carried `initial_meta`,
    /// passing `passed_meta` as the new metadata; returns the new top hash.
    async fn commit_hash(
        initial_meta: Option<serde_json::Value>,
        passed_meta: UserMeta,
    ) -> Res<String> {
        let storage = MockStorage::default();
        let paths = DomainPaths::new(PathBuf::from("/foo"));
        let (_lineage, commit) = commit_package(
            PackageLineage::default(),
            &mut manifest_with_meta(initial_meta),
            &paths,
            &storage,
            &MockRemote::default(),
            &None,
            PathBuf::default(),
            InstalledPackageStatus::default(),
            ("foo", "bar").into(),
            String::default(),
            passed_meta,
            None,
        )
        .await?;
        Ok(commit.hash)
    }

    /// `Keep` carries the previous revision's package-level metadata forward:
    /// it must produce the same top hash as passing that metadata explicitly.
    #[test(tokio::test)]
    async fn test_commit_keep_preserves_existing_meta() -> Res {
        let meta = serde_json::json!({"kept": "value"});
        assert_eq!(
            commit_hash(Some(meta.clone()), UserMeta::Keep).await?,
            commit_hash(Some(meta.clone()), UserMeta::Set(meta)).await?,
            "Keep must inherit the previous header's user_meta"
        );
        Ok(())
    }

    /// `Clear` removes the metadata: the result differs from `Keep` and is
    /// identical to committing a package that never had metadata.
    #[test(tokio::test)]
    async fn test_commit_clear_removes_meta() -> Res {
        let meta = serde_json::json!({"kept": "value"});
        assert_ne!(
            commit_hash(Some(meta.clone()), UserMeta::Keep).await?,
            commit_hash(Some(meta.clone()), UserMeta::Clear).await?,
            "Clear must remove metadata, not inherit it"
        );
        assert_eq!(
            commit_hash(Some(meta), UserMeta::Clear).await?,
            commit_hash(None, UserMeta::Clear).await?,
            "a cleared package must hash like one that never had metadata"
        );
        Ok(())
    }

    #[test(tokio::test)]
    async fn test_commit_meta() -> Res {
        let storage = MockStorage::default();

        let commit_message = "Lorem ipsum".to_string();
        let mut user_meta = serde_json::Map::new();
        user_meta.insert(
            "lorem".to_string(),
            serde_json::Value::String("ipsum".to_string()),
        );

        let paths = DomainPaths::new(PathBuf::from("/foo"));
        let lineage = PackageLineage::default();
        assert!(lineage.commit.is_none());
        let (_lineage, commit) = commit_package(
            lineage,
            &mut Manifest::default(),
            &paths,
            &storage,
            &MockRemote::default(),
            &None,
            PathBuf::default(),
            InstalledPackageStatus::default(),
            ("foo", "bar").into(),
            commit_message,
            UserMeta::Set(serde_json::Value::Object(user_meta)),
            None,
        )
        .await?;
        let hash = "56c329d2390c9c6efedb698f47b75f096112c89a7751d55a426507ec6c432897";
        assert!(
            storage
                .exists(&paths.installed_manifest(&("foo", "bar").into(), hash))
                .await
        );
        assert_eq!(commit.hash, hash);
        Ok(())
    }

    #[test(tokio::test)]
    async fn test_removing_and_commit() -> Res {
        let storage = MockStorage::default();

        let status = InstalledPackageStatus {
            changes: BTreeMap::from([(
                PathBuf::from("one/two two/three three three/READ ME.md"),
                Change::Removed(ManifestRow {
                    logical_key: PathBuf::from("one/two two/three three three/READ ME.md"),
                    ..ManifestRow::default()
                }),
            )]),
            ..InstalledPackageStatus::default()
        };

        let lineage = PackageLineage {
            paths: BTreeMap::from([(
                PathBuf::from("one/two two/three three three/READ ME.md"),
                PathState::default(),
            )]),
            ..PackageLineage::default()
        };
        let mut manifest = crate::fixtures::manifest_with_objects_all_sizes::manifest().await?;

        assert!(
            lineage.commit.is_none(),
            "Initial lineage has commit already"
        );
        assert!(
            lineage
                .paths
                .contains_key(&PathBuf::from("one/two two/three three three/READ ME.md")),
            "Initial lineage doesn't have testing path"
        );

        let paths = DomainPaths::new(PathBuf::from("/foo"));
        let (lineage, commit) = commit_package(
            lineage,
            &mut manifest,
            &paths,
            &storage,
            &MockRemote::default(),
            &None,
            PathBuf::default(),
            status,
            ("foo", "bar").into(),
            String::from("Initial"),
            UserMeta::Set(serde_json::json!({"A": "b", "z": "Y", "a": "B", "Z": "y"})),
            None,
        )
        .await?;

        let hash = "22590f2254e00b12f0c141117969172e925d6b8e9af26a04fa35658f1ad4e04c";
        assert!(
            !lineage
                .paths
                .contains_key(&PathBuf::from("one/two two/three three three/READ ME.md")),
            "Commited lineage still has a path, that should be clear after commit"
        );
        assert!(
            storage
                .exists(&paths.installed_manifest(&("foo", "bar").into(), hash))
                .await,
            "Registry doesn't have installed package with a new hash"
        );
        assert_eq!(commit.hash, hash);

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_adding_and_commit() -> Res {
        let manifest = fixtures::manifest_with_objects_all_sizes::manifest().await?;
        let base_record = manifest.get_record(&PathBuf::from("0mb.bin")).unwrap();
        let added_file = ManifestRow {
            logical_key: PathBuf::from("foo"),
            hash: base_record.hash.clone(),
            size: base_record.size,
            physical_key: base_record.physical_key.clone(),
            ..ManifestRow::default()
        };

        let storage = MockStorage::default();
        storage
            .write_byte_stream(PathBuf::from("/working-dir/foo"), ByteStream::default())
            .await?;

        let status = InstalledPackageStatus {
            changes: BTreeMap::from([(PathBuf::from("foo"), Change::Added(added_file.clone()))]),
            ..InstalledPackageStatus::default()
        };

        let lineage = PackageLineage::default();
        let mut manifest = crate::fixtures::manifest_with_objects_all_sizes::manifest().await?;

        assert!(
            lineage.commit.is_none(),
            "Initial lineage has commit already"
        );
        assert!(
            !lineage.paths.contains_key(&PathBuf::from("foo")),
            "Initial lineage has path, but shouldn't because we test _new_ file"
        );

        let paths = DomainPaths::new(PathBuf::from("/foo"));
        let (lineage, commit) = commit_package(
            lineage,
            &mut manifest,
            &paths,
            &storage,
            &MockRemote::default(),
            &None,
            PathBuf::from("/working-dir"),
            status,
            ("foo", "bar").into(),
            String::from("Initial"),
            UserMeta::Set(serde_json::json!({"A": "b", "z": "Y", "a": "B", "Z": "y"})),
            None,
        )
        .await?;

        let hash = fixtures::objects::ZERO_HASH_HEX;
        assert!(
            lineage.paths.contains_key(&PathBuf::from("foo")),
            "Commited lineage doesn't have path, but should have. We added new file and it should be there."
        );
        assert!(
            storage.exists(&paths.objects_dir().join(hash)).await,
            "Registry doesn't have installed path"
        );
        assert_eq!(
            commit.hash,
            "e8fc7ccb96e87acd4ca02123e0c658ad92cdb2cc2822103d4f5bac79254cca08"
        );

        Ok(())
    }

    // It is no longer reproducible in tests
    // and I doubt it ever could be reproducible at all
    // TODO: anyway it makes sense to add some sanity checks
    //       even for imposible states
    #[test(tokio::test)]
    async fn test_adding_manifest_already_has_it() -> Res {
        let manifest = fixtures::manifest_with_objects_all_sizes::manifest().await?;
        let base_record = manifest
            .get_record(&PathBuf::from("one/two two/three three three/READ ME.md"))
            .unwrap();
        let added_file = ManifestRow {
            logical_key: PathBuf::from("one/two two/three three three/READ ME.md"),
            hash: base_record.hash.clone(),
            size: base_record.size,
            physical_key: base_record.physical_key.clone(),
            ..ManifestRow::default()
        };
        let hash = added_file.hash.clone();

        let storage = MockStorage::default();
        let paths = DomainPaths::new(PathBuf::from("/foo"));
        storage
            .write_byte_stream(
                PathBuf::from("one/two two/three three three/READ ME.md"),
                ByteStream::from_static(b"This is the README."),
            )
            .await?;
        storage
            .write_byte_stream(
                paths.object(hash.digest()),
                ByteStream::from_static(b"This is the README."),
            )
            .await?;

        let status = InstalledPackageStatus {
            changes: BTreeMap::from([(
                PathBuf::from("one/two two/three three three/READ ME.md"),
                Change::Added(added_file.clone()),
            )]),
            ..InstalledPackageStatus::default()
        };

        let lineage = PackageLineage {
            paths: BTreeMap::from([(
                PathBuf::from("one/two two/three three three/READ ME.md"),
                PathState::default(),
            )]),
            ..PackageLineage::default()
        };
        let mut manifest = crate::fixtures::manifest_with_objects_all_sizes::manifest().await?;

        let result = commit_package(
            lineage,
            &mut manifest,
            &paths,
            &storage,
            &MockRemote::default(),
            &None,
            PathBuf::default(),
            status,
            ("foo", "bar").into(),
            String::from("Initial"),
            UserMeta::Set(serde_json::json!({"A": "b", "z": "Y", "a": "B", "Z": "y"})),
            None,
        )
        .await;

        assert_eq!(
            result.unwrap_err().to_string(),
            "Commit error: Trying to add a file that is already in the manifest: \"one/two two/three three three/READ ME.md\""
        );

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_modifying_and_commit() -> Res {
        let storage = MockStorage::default();
        let paths = DomainPaths::new(PathBuf::from("/foo"));
        storage
            .write_byte_stream(
                PathBuf::from("/working-dir/one/two two/three three three/READ ME.md"),
                ByteStream::from_static(fixtures::objects::less_than_8mb()),
            )
            .await?;

        let manifest = fixtures::manifest_with_objects_all_sizes::manifest().await?;
        let base_record = manifest
            .get_record(&PathBuf::from("less-then-8mb.txt"))
            .unwrap();
        let modified_file = ManifestRow {
            logical_key: PathBuf::from("one/two two/three three three/READ ME.md"),
            hash: base_record.hash.clone(),
            size: base_record.size,
            physical_key: base_record.physical_key.clone(),
            ..ManifestRow::default()
        };
        let status = InstalledPackageStatus {
            changes: BTreeMap::from([(
                PathBuf::from("one/two two/three three three/READ ME.md"),
                Change::Modified(modified_file),
            )]),
            ..InstalledPackageStatus::default()
        };

        let lineage = PackageLineage {
            paths: BTreeMap::from([(
                PathBuf::from("one/two two/three three three/READ ME.md"),
                PathState::default(),
            )]),
            ..PackageLineage::default()
        };
        let mut manifest = crate::fixtures::manifest_with_objects_all_sizes::manifest().await?;

        assert!(
            lineage.commit.is_none(),
            "Initial lineage has commit already"
        );
        assert!(
            lineage
                .paths
                .contains_key(&PathBuf::from("one/two two/three three three/READ ME.md")),
            "Initial lineage doesn't have path, but should because we test installed and modified file"
        );

        let (lineage, commit) = commit_package(
            lineage,
            &mut manifest,
            &paths,
            &storage,
            &MockRemote::default(),
            &None,
            PathBuf::from("/working-dir"),
            status,
            ("foo", "bar").into(),
            String::from("Initial"),
            UserMeta::Set(serde_json::json!({"A": "b", "z": "Y", "a": "B", "Z": "y"})),
            None,
        )
        .await?;

        assert!(
            lineage
                .paths
                .contains_key(&PathBuf::from("one/two two/three three three/READ ME.md")),
            "Commited lineage doesn't have path, but should have. We added new file and it should be there."
        );
        assert!(
            storage
                .exists(
                    &paths
                        .objects_dir()
                        .join(fixtures::objects::LESS_THAN_8MB_HASH_HEX)
                )
                .await,
            "Registry doesn't have installed path"
        );
        assert_eq!(
            commit.hash,
            "39bbc9a95f787cd938fb5830abe5e25408f0aac4000528b8717130be5f7bc2b3"
        );

        Ok(())
    }
}
