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
use crate::lineage::PackageLineage;
use crate::lineage::PathState;
use crate::manifest::Header;
use crate::manifest::Row;
use crate::manifest::Table;
use crate::manifest::Workflow;
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
    // Collect all rows from the local manifest stream
    let mut all_rows: Vec<Res<Row>> = Vec::new();
    
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
                        if removed.contains(&row.name) {
                            continue;
                        }
                        
                        // Use modified version if available, otherwise use original
                        if let Some(modified_row) = modified.get(&row.name) {
                            all_rows.push(Ok(modified_row.clone()));
                        } else {
                            all_rows.push(Ok(row.clone()));
                        }
                    }
                    Err(err) => all_rows.push(Err(Error::Table(err.to_string()))),
                }
            }
        }
    }
    
    // Sort all rows by name
    all_rows.sort_by(|a, b| {
        match (a, b) {
            (Ok(row_a), Ok(row_b)) => row_a.name.cmp(&row_b.name),
            (Ok(_), Err(_)) => std::cmp::Ordering::Less,
            (Err(_), Ok(_)) => std::cmp::Ordering::Greater,
            (Err(_), Err(_)) => std::cmp::Ordering::Equal,
        }
    });
    
    // Convert back to a stream
    tokio_stream::iter(vec![Ok(all_rows)])
}

async fn create_immutable_object_copy(
    storage: &impl Storage,
    paths: &DomainPaths,
    working_dir: &Path,
    lineage: &mut PackageLineage,
    logical_key: &PathBuf,
    current: Row,
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
        ..current
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
//     user_meta: Option<serde_json::Value>,
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
    user_meta: Option<serde_json::Value>,
    workflow: Option<Workflow>,
) -> Res<PackageLineage> {
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
                lineage.paths.remove(&row.name);
                removed_keys.insert(row.name);
            }
            Change::Added(current) => {
                if manifest.contains_record(&current.name).await {
                    return Err(Error::Commit(format!(
                        "Trying to add a file that is already in the manifest: \"{}\"",
                        current.name.display()
                    )));
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
        meta: if let Some(u) = user_meta {
            match u {
                serde_json::Value::Object(mut m) => {
                    m.sort_keys();
                    Some(m.into())
                }
                _ => u.into(),
            }
        } else {
            None
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

    use crate::fixtures;
    use crate::io::storage::mocks::MockStorage;
    use crate::lineage::Change;

    // NOTE: Tests use "/" path for working directory, because it then parsed with Url and have to be absolute path

    #[tokio::test]
    async fn test_commit_empty() -> Res {
        let storage = MockStorage::default();
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
            String::default(),
            None,
            None,
        )
        .await?;
        let hash = fixtures::manifest_empty::EMPTY_NONE_TOP_HASH;
        assert!(
            storage
                .exists(&PathBuf::from(format!(".quilt/installed/foo/bar/{}", hash)))
                .await
        );
        assert_eq!(lineage.commit.unwrap().hash, hash.to_string());
        Ok(())
    }

    #[tokio::test]
    async fn test_commit_meta() -> Res {
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
            Some(serde_json::Value::Object(user_meta)),
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

        let status = InstalledPackageStatus {
            changes: BTreeMap::from([(
                PathBuf::from("one/two two/three three three/READ ME.md"),
                Change::Removed(Row {
                    name: PathBuf::from("one/two two/three three three/READ ME.md"),
                    ..Row::default()
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

        let lineage = commit_package(
            lineage,
            &mut manifest,
            &DomainPaths::default(),
            &storage,
            PathBuf::default(),
            status,
            ("foo", "bar").into(),
            String::from("Initial"),
            Some(serde_json::json!({"A": "b", "z": "Y", "a": "B", "Z": "y"})),
            None,
        )
        .await?;

        let hash = "22590f2254e00b12f0c141117969172e925d6b8e9af26a04fa35658f1ad4e04c";
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
        let added_file = Row {
            name: PathBuf::from("foo"),
            ..fixtures::manifest_with_objects_all_sizes::manifest()
                .await?
                .get_record(&PathBuf::from("0mb.bin"))
                .await?
                .unwrap()
        };

        let storage = MockStorage::default();
        storage
            .write_file(PathBuf::from("/working-dir/foo"), &Vec::new())
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

        let lineage = commit_package(
            lineage,
            &mut manifest,
            &DomainPaths::new(PathBuf::from("/")),
            &storage,
            PathBuf::from("/working-dir"),
            status,
            ("foo", "bar").into(),
            String::from("Initial"),
            Some(serde_json::json!({"A": "b", "z": "Y", "a": "B", "Z": "y"})),
            None,
        )
        .await?;

        let hash = fixtures::objects::ZERO_HASH_HEX;
        assert!(
            lineage.paths.contains_key(&PathBuf::from("foo")),
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
            "e8fc7ccb96e87acd4ca02123e0c658ad92cdb2cc2822103d4f5bac79254cca08"
        );

        Ok(())
    }

    // It is no longer reproducible in tests
    // and I doubt it ever could be reproducible at all
    // TODO: anyway it makes sense to add some sanity checks
    //       even for imposible states
    #[tokio::test]
    async fn test_adding_manifest_already_has_it() -> Res {
        let added_file = Row {
            name: PathBuf::from("one/two two/three three three/READ ME.md"),
            ..fixtures::manifest_with_objects_all_sizes::manifest()
                .await?
                .get_record(&PathBuf::from("one/two two/three three three/READ ME.md"))
                .await?
                .unwrap()
        };
        let hash = added_file.hash.clone();

        let storage = MockStorage::default();
        storage
            .write_file(
                PathBuf::from("one/two two/three three three/READ ME.md"),
                "This is the README.".as_bytes(),
            )
            .await?;
        storage
            .write_file(
                PathBuf::from(format!(".quilt/objects/{}", hex::encode(hash.digest()))),
                "This is the README.".as_bytes(),
            )
            .await?;

        let status = InstalledPackageStatus {
            changes: BTreeMap::from([(
                PathBuf::from("one/two two/three three three/READ ME.md"),
                Change::Added(Row {
                    name: PathBuf::from("one/two two/three three three/READ ME.md"),
                    ..added_file.clone()
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

        let result = commit_package(
            lineage,
            &mut manifest,
            &DomainPaths::new(PathBuf::from("/")),
            &storage,
            PathBuf::default(),
            status,
            ("foo", "bar").into(),
            String::from("Initial"),
            Some(serde_json::json!({"A": "b", "z": "Y", "a": "B", "Z": "y"})),
            None,
        )
        .await;

        assert_eq!(
            result.unwrap_err().to_string(),
            "Commit error: Trying to add a file that is already in the manifest: \"one/two two/three three three/READ ME.md\""
        );

        Ok(())
    }

    #[tokio::test]
    async fn test_modifying_and_commit() -> Res {
        let storage = MockStorage::default();
        storage
            .write_file(
                PathBuf::from("/working-dir/one/two two/three three three/READ ME.md"),
                &fixtures::objects::less_than_8mb(),
            )
            .await?;

        let modified_file = Row {
            name: PathBuf::from("one/two two/three three three/READ ME.md"),
            ..fixtures::manifest_with_objects_all_sizes::manifest()
                .await?
                .get_record(&PathBuf::from("less-then-8mb.txt"))
                .await?
                .unwrap()
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
            lineage.paths.contains_key(&PathBuf::from("one/two two/three three three/READ ME.md")),
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
            String::from("Initial"),
            Some(serde_json::json!({"A": "b", "z": "Y", "a": "B", "Z": "y"})),
            None,
        )
        .await?;

        assert!(
            lineage.paths.contains_key(&PathBuf::from("one/two two/three three three/READ ME.md")),
            "Commited lineage doesn't have path, but should have. We added new file and it should be there."
        );
        assert!(
            storage
                .exists(&PathBuf::from(format!(
                    "/.quilt/objects/{}",
                    fixtures::objects::LESS_THAN_8MB_HASH_HEX
                )))
                .await,
            "Registry doesn't have installed path"
        );
        assert_eq!(
            lineage.commit.unwrap().hash,
            "39bbc9a95f787cd938fb5830abe5e25408f0aac4000528b8717130be5f7bc2b3"
        );

        Ok(())
    }
}
