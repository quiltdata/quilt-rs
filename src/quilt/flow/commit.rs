use std::path::PathBuf;

use serde_json::json;
use tracing::log;
use url::Url;

use crate::paths;
use crate::quilt::Storage;
use crate::Error;
use crate::Row4;

use crate::quilt::flow::status::create_status;
use crate::quilt::flow::status::Change;
use crate::quilt::lineage::CommitState;
use crate::quilt::lineage::PackageLineage;
use crate::quilt::lineage::PathState;
use crate::quilt::manifest::JsonObject;
use crate::quilt::manifest_handle::ReadableManifest;

// TODO: move `working_dir` to `paths`, and `paths` to `storage`
#[allow(clippy::too_many_arguments)]
pub async fn commit_package(
    lineage: PackageLineage,
    manifest: &(impl ReadableManifest + Sync),
    paths: &paths::DomainPaths,
    storage: &mut impl Storage,
    working_dir: PathBuf,
    namespace: String,
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

    // TODO: Maybe have the user pass this as an argument?
    let (mut lineage, status) =
        create_status(lineage, storage, manifest, working_dir.clone()).await?;

    let objects_dir = paths.objects_dir();
    // TODO: This should really be done when the domain is created.
    storage.create_dir_all(&objects_dir).await?;

    let mut table = manifest.read().await?;

    for (logical_key, Change { current, previous }) in status.changes {
        if let Some(previous) = previous {
            let removed = table
                .records
                .remove(&logical_key)
                .ok_or(Error::Commit(format!("cannot remove {}", logical_key)))?;
            if removed.size != previous.size || removed.hash != previous.hash {
                return Err(Error::Commit(format!(
                    "unexpected size or hash for removed {}",
                    logical_key
                )));
            }
            lineage.paths.remove(&logical_key);
        }
        if let Some(current) = current {
            let object_dest = objects_dir.join(hex::encode(current.hash.digest()));
            let new_physical_key = Url::from_file_path(&object_dest)
                .map_err(|_| {
                    Error::Commit(format!("Failed to create URL from {:?}", &object_dest))
                })?
                .into();

            if table
                .records
                .insert(
                    logical_key.to_owned(),
                    Row4 {
                        name: logical_key.to_owned(),
                        place: new_physical_key,
                        size: current.size,
                        hash: current.hash,
                        info: serde_json::Value::default(),
                        meta: serde_json::Value::default(),
                    },
                )
                .is_some()
            {
                return Err(Error::Commit(format!("cannot overwrite {}", logical_key)));
            }

            let work_dest = working_dir.join(&logical_key);

            if !storage.exists(&object_dest).await {
                storage.copy(&work_dest, object_dest).await?;
            }
            lineage.paths.insert(
                logical_key,
                PathState {
                    timestamp: storage.modified_timestamp(&work_dest).await?,
                    hash: current.hash,
                },
            );
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

    use temp_dir::TempDir;

    use crate::quilt::storage::mock_storage::MockStorage;
    use crate::quilt::Table;

    struct TestManifest {}

    impl ReadableManifest for TestManifest {
        fn get_path_buf(&self) -> PathBuf {
            PathBuf::new()
        }
        async fn read(&self) -> Result<Table, Error> {
            Ok(Table::default())
        }
    }

    #[tokio::test]
    async fn test_commit() -> Result<(), Error> {
        let working_dir = TempDir::new()?;
        let namespace = "foo/bar".to_string();
        let mut storage = MockStorage::default();

        let domain_paths = &paths::DomainPaths::new(working_dir.path().to_path_buf());
        // storage
        //     .create_dir_all(&domain_paths.installed_manifests(&namespace))
        //     .await?;
        // storage.create_dir_all(&domain_paths.objects_dir()).await?;
        tokio::fs::create_dir_all(&domain_paths.installed_manifests(&namespace)).await?;
        tokio::fs::create_dir_all(&domain_paths.objects_dir()).await?;

        let commit_message = "Lorem ipsum".to_string();
        let mut user_meta = serde_json::Map::new();
        user_meta.insert(
            "lorem".to_string(),
            serde_json::Value::String("ipsum".to_string()),
        );

        let lineage = PackageLineage::default();
        assert!(lineage.commit.is_none());
        let manifest = TestManifest {};
        let lineage = commit_package(
            lineage,
            &manifest,
            domain_paths,
            &mut storage,
            working_dir.path().to_path_buf(),
            namespace,
            commit_message.clone(),
            Some(user_meta),
        )
        .await?;
        assert_eq!(
            lineage.commit.unwrap().hash,
            "56c329d2390c9c6efedb698f47b75f096112c89a7751d55a426507ec6c432897".to_string()
        );
        Ok(())
    }
}
