use std::path::Path;
use std::path::PathBuf;

use serde_json::json;
use tracing::log;
use url::Url;

use crate::paths;
use crate::quilt::Storage;
use crate::Error;
use crate::Row4;

use crate::flow::status::Change;
use crate::flow::status::InstalledPackageStatus;
use crate::flow::status::PackageFileFingerprint;
use crate::lineage::CommitState;
use crate::lineage::PackageLineage;
use crate::lineage::PathState;
use crate::quilt::manifest::JsonObject;
use crate::quilt::manifest_handle::ReadableManifest;
use crate::quilt::uri::Namespace;
use crate::quilt4::table::Table;

fn remove_entry(
    table: &mut Table,
    lineage: &mut PackageLineage,
    logical_key: &PathBuf,
    previous: PackageFileFingerprint,
) -> Result<(), Error> {
    let removed = table.remove_record(logical_key)?;
    if removed.size != previous.size || removed.hash != previous.hash {
        return Err(Error::Commit(format!(
            "unexpected size or hash for removed {:?}",
            logical_key
        )));
    }
    lineage.paths.remove(logical_key);
    Ok(())
}

async fn modify_entry(
    storage: &impl Storage,
    paths: &paths::DomainPaths,
    working_dir: &Path,
    table: &mut Table,
    lineage: &mut PackageLineage,
    logical_key: &PathBuf,
    current: PackageFileFingerprint,
) -> Result<(), Error> {
    let objects_dir = paths.objects_dir();
    // FIXME: This should really be done when the domain is created.
    storage.create_dir_all(&objects_dir).await?;
    let object_dest = objects_dir.join(hex::encode(current.hash.digest()));
    let new_physical_key = Url::from_file_path(&object_dest)
        .map_err(|_| Error::Commit(format!("Failed to create URL from {:?}", &object_dest)))?
        .into();

    if table
        .records
        .insert(
            logical_key.clone(),
            Row4 {
                name: logical_key.clone(),
                place: new_physical_key,
                size: current.size,
                hash: current.hash,
                info: serde_json::Value::default(),
                meta: serde_json::Value::default(),
            },
        )
        .is_some()
    {
        return Err(Error::Commit(format!("cannot overwrite {:?}", logical_key)));
    }

    let work_dest = working_dir.join(logical_key);

    if !storage.exists(&object_dest).await {
        storage.copy(&work_dest, object_dest).await?;
    }
    lineage.paths.insert(
        logical_key.clone(),
        PathState {
            timestamp: storage.modified_timestamp(&work_dest).await?,
            hash: current.hash,
        },
    );
    Ok(())
}

// TODO: move `working_dir` to `paths`, and `paths` to `storage`
#[allow(clippy::too_many_arguments)]
pub async fn commit_package(
    mut lineage: PackageLineage,
    manifest: &(impl ReadableManifest + Sync),
    paths: &paths::DomainPaths,
    storage: &(impl Storage + Sync),
    working_dir: PathBuf,
    status: InstalledPackageStatus,
    namespace: Namespace,
    message: String,
    user_meta: Option<JsonObject>,
) -> Result<PackageLineage, Error> {
    log::debug!("commit: {message:?}, {user_meta:?}");
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

    let mut table = manifest.read(storage).await?;

    for (
        logical_key,
        Change {
            current, previous, ..
        },
    ) in status.changes
    {
        if let Some(previous) = previous {
            remove_entry(&mut table, &mut lineage, &logical_key, previous)?;
        }
        if let Some(current) = current {
            modify_entry(
                storage,
                paths,
                &working_dir,
                &mut table,
                &mut lineage,
                &logical_key,
                current,
            )
            .await?;
        }
    }

    table.header.info = json!({
        "message": message,
        "version": "v0",
    });
    if let Some(user_meta) = user_meta {
        table.header.meta = user_meta.into();
    }

    let new_top_hash = table.top_hash();

    let new_manifest_path = paths.installed_manifest(&namespace, &new_top_hash);
    table.write_to_path(storage, &new_manifest_path).await?;

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

    Ok(lineage)
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::BTreeMap;

    use crate::flow::status::Change;
    use crate::flow::status::DiscreteChange;
    use crate::quilt::mocks;
    use crate::quilt::storage::mock_storage::MockStorage;

    // NOTE: Tests use "/" path for working directory, because it then parsed with Url and have to be absolute path

    #[tokio::test]
    async fn test_commit() -> Result<(), Error> {
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
            &mocks::manifest::default(),
            &paths::DomainPaths::default(),
            &storage,
            PathBuf::default(),
            InstalledPackageStatus::default(),
            ("foo", "bar").into(),
            commit_message,
            Some(user_meta),
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
    async fn test_removing_and_commit() -> Result<(), Error> {
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
                Change {
                    previous: Some(mocks::status::package_file_fingerprint()),
                    current: None,
                    state: DiscreteChange::Removed,
                },
            )]),
            ..InstalledPackageStatus::default()
        };

        let lineage = mocks::lineage::with_paths(vec![PathBuf::from("foo")]);
        let manifest = mocks::manifest::with_record_keys(vec![PathBuf::from("foo")]);

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
            &manifest,
            &paths::DomainPaths::default(),
            &storage,
            PathBuf::default(),
            status,
            ("foo", "bar").into(),
            commit_message,
            Some(user_meta),
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
    async fn test_adding_and_commit() -> Result<(), Error> {
        let storage = MockStorage::default();
        storage
            .write_file(PathBuf::from("/working-dir/bar"), &Vec::new())
            .await?;

        let status = InstalledPackageStatus {
            changes: BTreeMap::from([(
                PathBuf::from("bar"),
                Change {
                    current: Some(mocks::status::package_file_fingerprint()),
                    previous: None,
                    state: DiscreteChange::Added,
                },
            )]),
            ..InstalledPackageStatus::default()
        };

        let lineage = PackageLineage::default();
        let manifest = mocks::manifest::with_record_keys(vec![PathBuf::from("foo")]);

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
            &manifest,
            &paths::DomainPaths::new(PathBuf::from("/")),
            &storage,
            PathBuf::from("/working-dir"),
            status,
            ("foo", "bar").into(),
            "Lorem ipsum".to_string(),
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
            "cab702f67a810907dde744a637f4686c3b57f36852c438e15c2075d865b29738"
        );

        Ok(())
    }

    #[tokio::test]
    async fn test_adding_manifest_already_has_it() -> Result<(), Error> {
        let storage = MockStorage::default();

        let status = InstalledPackageStatus {
            changes: BTreeMap::from([(
                PathBuf::from("foo"),
                Change {
                    current: Some(mocks::status::package_file_fingerprint()),
                    previous: None,
                    state: DiscreteChange::Added,
                },
            )]),
            ..InstalledPackageStatus::default()
        };

        let lineage = mocks::lineage::with_paths(vec![PathBuf::from("foo")]);
        let manifest = mocks::manifest::with_record_keys(vec![PathBuf::from("foo")]);

        let result = commit_package(
            lineage,
            &manifest,
            &paths::DomainPaths::new(PathBuf::from("/")),
            &storage,
            PathBuf::default(),
            status,
            ("foo", "bar").into(),
            "Lorem ipsum".to_string(),
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
    async fn test_modifying_and_commit() -> Result<(), Error> {
        let storage = MockStorage::default();
        storage
            .write_file(PathBuf::from("/working-dir/bar"), &Vec::new())
            .await?;

        let status = InstalledPackageStatus {
            changes: BTreeMap::from([(
                PathBuf::from("bar"),
                Change {
                    previous: Some(PackageFileFingerprint {
                        size: 0,
                        hash: mocks::row_hash_sample1(),
                    }),
                    current: Some(PackageFileFingerprint {
                        size: 0,
                        hash: multihash::Multihash::wrap(0xb510, b"walker")?,
                    }),
                    state: DiscreteChange::Modified,
                },
            )]),
            ..InstalledPackageStatus::default()
        };

        let lineage = mocks::lineage::with_paths(vec![PathBuf::from("bar")]);
        let manifest = mocks::manifest::with_record_keys(vec![PathBuf::from("bar")]);

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
            &manifest,
            &paths::DomainPaths::new(PathBuf::from("/")),
            &storage,
            PathBuf::from("/working-dir"),
            status,
            ("foo", "bar").into(),
            "Lorem ipsum".to_string(),
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
