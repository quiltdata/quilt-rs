use std::collections::BTreeMap;
use std::collections::HashSet;
use std::path::Path;
use std::path::PathBuf;

use serde_json::json;
use tokio_stream::StreamExt;
use tracing::warn;
use tracing::{debug, info};
use url::Url;

use crate::io::manifest::build_manifest_from_rows_stream;
use crate::io::manifest::RowsStream;
use crate::io::manifest::StreamRowsChunk;
use crate::io::storage::Storage;
use crate::lineage::Change;
use crate::lineage::CommitState;
use crate::lineage::InstalledPackageStatus;
use crate::lineage::PackageFileFingerprint;
use crate::lineage::PackageLineage;
use crate::lineage::PathState;
use crate::manifest::Header;
use crate::manifest::JsonObject;
use crate::manifest::Row;
use crate::manifest::Table;
use crate::manifest::Workflow;
use crate::paths::scaffold_paths;
use crate::paths::DomainPaths;
use crate::uri::Namespace;
use crate::Error;
use crate::Res;

async fn stream_local_with_changes(
    local_manifest: &Table,
    removed: HashSet<PathBuf>,
    modified: BTreeMap<PathBuf, Row>,
    new_files: StreamRowsChunk,
) -> impl RowsStream {
    let changes_stream = local_manifest.records_stream().await.map(move |rows| {
        rows.map(|rows| {
            rows.iter()
                .filter_map(|row_res| match row_res {
                    Ok(row) => {
                        if removed.contains(&row.name) {
                            return None;
                        }
                        if let Some(modified_row) = modified.get(&row.name) {
                            return Some(Ok(modified_row.clone()));
                        }
                        Some(Ok(row.clone()))
                    }
                    Err(err) => Some(Err(Error::Table(err.to_string()))),
                })
                .collect()
        })
    });
    tokio_stream::iter(vec![Ok(new_files)]).chain(changes_stream)
}

async fn create_immutable_object_copy(
    storage: &impl Storage,
    paths: &DomainPaths,
    working_dir: &Path,
    lineage: &mut PackageLineage,
    logical_key: &PathBuf,
    current: PackageFileFingerprint,
) -> Res<Row> {
    debug!(
        "⏳ Creating immutable object copy for: {}",
        logical_key.display()
    );
    let objects_dir = paths.objects_dir();
    let object_dest = objects_dir.join(hex::encode(current.hash.digest()));
    let new_physical_key = Url::from_file_path(&object_dest)
        .map_err(|_| Error::Commit(format!("Failed to create URL from {:?}", &object_dest)))?
        .into();

    let row = Row {
        name: logical_key.clone(),
        place: new_physical_key,
        size: current.size,
        hash: current.hash,
        info: serde_json::Value::default(),
        meta: serde_json::Value::default(),
    };

    let work_dest = working_dir.join(logical_key);

    if !storage.exists(&object_dest).await {
        debug!(
            "⏳ Copying file to objects directory: {}",
            object_dest.display()
        );
        storage.copy(&work_dest, object_dest).await?;
        debug!("✔️ File copied successfully");
    } else {
        debug!(
            "✔️ Object already exists in storage: {}",
            object_dest.display()
        );
    }
    lineage.paths.insert(
        logical_key.clone(),
        PathState {
            timestamp: storage.modified_timestamp(&work_dest).await?,
            hash: current.hash,
        },
    );
    Ok(row)
}

// TODO
// pub struct Commit {
//[     message: Option<String>,
//     user_meta: Option<JsonObject>,
//     workflow: Option<Workflow>,
// }

/// Commit new commit with new `message`, `user_meta` and all changes got from calling `flow::status`
// TODO: move `working_dir` to `paths`, and `paths` to `storage`
#[allow(clippy::too_many_arguments)]
pub async fn commit_package(
    mut lineage: PackageLineage,
    manifest: &mut Table,
    paths: &DomainPaths,
    storage: &(impl Storage + Sync),
    working_dir: PathBuf,
    status: InstalledPackageStatus,
    namespace: Namespace,
    message: String,
    user_meta: Option<JsonObject>,
    workflow: Option<Workflow>,
) -> Res<PackageLineage> {
    info!(
        r#"⏳ Starting commit with message "{}" and user_meta `{:?}`"#,
        message, user_meta
    );

    let required_paths = paths.required_for_installing(&namespace);
    scaffold_paths(storage, required_paths).await?;

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
                lineage.paths.remove(&row.name);
                removed_keys.insert(row.name);
            }
            Change::Added(current) => {
                let added = create_immutable_object_copy(
                    storage,
                    paths,
                    &working_dir,
                    &mut lineage,
                    &logical_key,
                    current,
                )
                .await?;
                new_files.push(Ok(added))
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

    let header = Header {
        info: json!({
            "message": message,
            "version": "v0",
            "workflow": workflow,
        }),
        meta: if let Some(mut u) = user_meta {
            u.sort_keys();
            u.into()
        } else {
            serde_json::Value::Null
        },
    };

    debug!(
        "⏳ Building new manifest with {} removed, {} modified, {} new files",
        removed_keys.len(),
        modified_keys.len(),
        new_files.len()
    );
    let stream = stream_local_with_changes(manifest, removed_keys, modified_keys, new_files).await;
    let dest_dir = paths.installed_manifests(&namespace);
    let (manifest_path, new_top_hash) =
        build_manifest_from_rows_stream(storage, dest_dir, header, stream).await?;
    info!(
        "✔️New manifest with {} was built in {}",
        manifest_path.display(),
        new_top_hash
    );
    let mut prev_hashes = Vec::new();
    if let Some(commit) = lineage.commit {
        prev_hashes.push(commit.hash.to_owned());
        prev_hashes.extend(commit.prev_hashes.to_owned());
    }
    let commit = CommitState {
        hash: new_top_hash,
        timestamp: chrono::Utc::now(),
        prev_hashes,
    };
    lineage.commit = Some(commit);

    if let Some(ref commit) = lineage.commit {
        info!(
            "✔️ Successfully committed changes with hash: {}",
            commit.hash
        );
    } else {
        warn!("❌ Failed writing the commit to the lineage",);
    }
    Ok(lineage)
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::BTreeMap;

    use crate::fixtures::sample_file_1;
    use crate::io::storage::mocks::MockStorage;
    use crate::lineage::Change;

    // NOTE: Tests use "/" path for working directory, because it then parsed with Url and have to be absolute path

    #[tokio::test]
    async fn test_commit() -> Res {
        let storage = MockStorage::default();

        let commit_message = "Lorem ipsum".to_string();
        let mut user_meta = serde_json::Map::new();
        user_meta.insert(
            "lorem".to_string(),
            serde_json::Value::String("ipsum".to_string()),
        );

        let lineage = PackageLineage::default();
        assert!(lineage.commit.is_none());
        let lineage = commit_package(
            lineage,
            &mut Table::default(),
            &DomainPaths::default(),
            &storage,
            PathBuf::default(),
            InstalledPackageStatus::default(),
            ("foo", "bar").into(),
            commit_message,
            Some(user_meta),
            None,
        )
        .await?;
        let hash = "56c329d2390c9c6efedb698f47b75f096112c89a7751d55a426507ec6c432897";
        assert!(
            storage
                .exists(&PathBuf::from(format!(".quilt/installed/foo/bar/{}", hash)))
                .await
        );
        assert_eq!(lineage.commit.unwrap().hash, hash.to_string());
        Ok(())
    }

    #[tokio::test]
    async fn test_removing_and_commit() -> Res {
        let storage = MockStorage::default();

        let commit_message = "Lorem ipsum".to_string();
        let mut user_meta = serde_json::Map::new();
        user_meta.insert(
            "lorem".to_string(),
            serde_json::Value::String("ipsum".to_string()),
        );
        let status = InstalledPackageStatus {
            changes: BTreeMap::from([(
                PathBuf::from("foo"),
                Change::Removed(Row {
                    name: PathBuf::from("foo"),
                    ..Row::default()
                }),
            )]),
            ..InstalledPackageStatus::default()
        };

        let lineage = PackageLineage {
            paths: BTreeMap::from([(PathBuf::from("foo"), sample_file_1::path_state()?)]),
            ..PackageLineage::default()
        };
        let mut manifest = Table::default();
        manifest
            .insert_record(sample_file_1::row(PathBuf::from("foo"))?)
            .await?;

        assert!(
            lineage.commit.is_none(),
            "Initial lineage has commit already"
        );
        assert!(
            lineage.paths.contains_key(&PathBuf::from("foo")),
            "Initial lineage doesn't have testing path"
        );

        let lineage = commit_package(
            lineage,
            &mut manifest,
            &DomainPaths::default(),
            &storage,
            PathBuf::default(),
            status,
            ("foo", "bar").into(),
            commit_message,
            Some(user_meta),
            None,
        )
        .await?;

        let hash = "56c329d2390c9c6efedb698f47b75f096112c89a7751d55a426507ec6c432897";
        assert!(
            !lineage.paths.contains_key(&PathBuf::from("foo")),
            "Commited lineage still has a path, that should be clear after commit"
        );
        assert!(
            storage
                .exists(&PathBuf::from(format!(".quilt/installed/foo/bar/{}", hash)))
                .await,
            "Registry doesn't have installed package with a new hash"
        );
        assert_eq!(lineage.commit.unwrap().hash, hash.to_string());

        Ok(())
    }

    #[tokio::test]
    async fn test_adding_and_commit() -> Res {
        let storage = MockStorage::default();
        storage
            .write_file(PathBuf::from("/working-dir/bar"), &Vec::new())
            .await?;

        let status = InstalledPackageStatus {
            changes: BTreeMap::from([(
                PathBuf::from("bar"),
                Change::Added(sample_file_1::fingerprint()?),
            )]),
            ..InstalledPackageStatus::default()
        };

        let lineage = PackageLineage::default();
        let mut manifest = Table::default();
        manifest
            .insert_record(sample_file_1::row(PathBuf::from("foo"))?)
            .await?;

        assert!(
            lineage.commit.is_none(),
            "Initial lineage has commit already"
        );
        assert!(
            !lineage.paths.contains_key(&PathBuf::from("bar")),
            "Initial lineage has path, but shouldn't because we test _new_ file"
        );

        let lineage = commit_package(
            lineage,
            &mut manifest,
            &DomainPaths::new(PathBuf::from("/")),
            &storage,
            PathBuf::from("/working-dir"),
            status,
            ("foo", "bar").into(),
            "Lorem ipsum".to_string(),
            None,
            None,
        )
        .await?;

        let hash = "7065646573747269616e";
        assert!(
            lineage.paths.contains_key(&PathBuf::from("bar")),
            "Commited lineage doesn't have path, but should have. We added new file and it should be there."
        );
        assert!(
            storage
                .exists(&PathBuf::from(format!("/.quilt/objects/{}", hash)))
                .await,
            "Registry doesn't have installed path"
        );
        assert_eq!(
            lineage.commit.unwrap().hash,
            // NOTE: I copied this hash from the test result itself.
            //       I don't know what is the right hash
            "5819856fad67101036f115a273d7444059f403e37d51a9e3e4afa92d7d12786f"
        );

        Ok(())
    }

    // It is no longer reproducible in tests
    // and I doubt it ever could be reproducible at all
    // TODO: anyway it makes sense to add some sanity checks
    //       even for imposible states
    #[ignore]
    #[tokio::test]
    async fn test_adding_manifest_already_has_it() -> Res {
        let storage = MockStorage::default();
        storage
            .write_file(PathBuf::from("foo"), &Vec::new())
            .await?;

        let status = InstalledPackageStatus {
            changes: BTreeMap::from([(
                PathBuf::from("foo"),
                Change::Added(PackageFileFingerprint::default()),
            )]),
            ..InstalledPackageStatus::default()
        };

        let lineage = PackageLineage {
            paths: BTreeMap::from([(PathBuf::from("foo"), sample_file_1::path_state()?)]),
            ..PackageLineage::default()
        };
        let mut manifest = Table::default();
        manifest
            .insert_record(sample_file_1::row(PathBuf::from("foo"))?)
            .await?;

        let result = commit_package(
            lineage,
            &mut manifest,
            &DomainPaths::new(PathBuf::from("/")),
            &storage,
            PathBuf::default(),
            status,
            ("foo", "bar").into(),
            "Lorem ipsum".to_string(),
            None,
            None,
        )
        .await;

        assert_eq!(
            result.unwrap_err().to_string(),
            r#"Commit error: cannot overwrite "foo""#
        );

        Ok(())
    }

    #[tokio::test]
    async fn test_modifying_and_commit() -> Res {
        let storage = MockStorage::default();
        storage
            .write_file(PathBuf::from("/working-dir/bar"), &Vec::new())
            .await?;

        let status = InstalledPackageStatus {
            changes: BTreeMap::from([(
                PathBuf::from("bar"),
                Change::Modified(PackageFileFingerprint {
                    size: 0,
                    hash: multihash::Multihash::wrap(0xb510, b"walker")?,
                }),
            )]),
            ..InstalledPackageStatus::default()
        };

        let lineage = PackageLineage {
            paths: BTreeMap::from([(PathBuf::from("bar"), sample_file_1::path_state()?)]),
            ..PackageLineage::default()
        };
        let mut manifest = Table::default();
        manifest
            .insert_record(sample_file_1::row(PathBuf::from("bar"))?)
            .await?;

        assert!(
            lineage.commit.is_none(),
            "Initial lineage has commit already"
        );
        assert!(
            lineage.paths.contains_key(&PathBuf::from("bar")),
            "Initial lineage doesn't have path, but should because we test installed and modified file"
        );

        let lineage = commit_package(
            lineage,
            &mut manifest,
            &DomainPaths::new(PathBuf::from("/")),
            &storage,
            PathBuf::from("/working-dir"),
            status,
            ("foo", "bar").into(),
            "Lorem ipsum".to_string(),
            None,
            None,
        )
        .await?;

        let hash = "77616c6b6572";
        assert!(
            lineage.paths.contains_key(&PathBuf::from("bar")),
            "Commited lineage doesn't have path, but should have. We added new file and it should be there."
        );
        assert!(
            storage
                .exists(&PathBuf::from(format!("/.quilt/objects/{}", hash)))
                .await,
            "Registry doesn't have installed path"
        );
        assert_eq!(
            lineage.commit.unwrap().hash,
            // NOTE: I copied this hash from the test result itself.
            //       I don't know what is the right hash
            "48e56751fda714b87fd3e5cb0a496cd0daa6d76ac45f0a89c5dc4c3fbbfe522e"
        );

        Ok(())
    }
}
