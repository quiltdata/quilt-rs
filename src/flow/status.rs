use std::collections::HashMap;
use std::collections::VecDeque;
use std::path::PathBuf;

use tracing::log;

use crate::checksum::calculate_sha256_checksum;
use crate::checksum::calculate_sha256_chunked_checksum;
use crate::checksum::MULTIHASH_SHA256_CHUNKED;
use crate::io::manifest::resolve_latest;
use crate::io::remote::Remote;
use crate::io::storage::Storage;
use crate::io::Entry;
use crate::lineage::Change;
use crate::lineage::ChangeSet;
use crate::lineage::InstalledPackageStatus;
use crate::lineage::PackageLineage;
use crate::manifest::Table;
use crate::Error;
use crate::Res;

/// Refreshes the tracked `latest_hash` property in lineage.json
pub async fn refresh_latest_hash(
    mut lineage: PackageLineage,
    remote: &impl Remote,
) -> Res<PackageLineage> {
    let latest = resolve_latest(remote, lineage.remote.clone().into()).await?;
    if lineage.latest_hash == latest.hash {
        return Ok(lineage);
    }
    lineage.latest_hash = latest.hash;
    Ok(lineage)
}

/// Creates the status of local modifications
/// It is used for `flow::commit` and for showing the status in UI.
pub async fn create_status(
    lineage: PackageLineage,
    storage: &(impl Storage + Sync),
    manifest: &Table,
    working_dir: PathBuf,
) -> Res<(PackageLineage, InstalledPackageStatus)> {
    // compute the status based on the following sources:
    //   - the cached manifest
    //   - paths
    //   - working directory state
    // installed entries marked as "installed" (initially as "downloading")
    // modified entries marked as "modified", etc

    let mut orig_paths = HashMap::new();
    for path in lineage.paths.keys() {
        let row = manifest
            .get_record(path)
            .await?
            .ok_or(Error::ManifestPath(format!(
                "path {:?} not found in installed manifest",
                path
            )))?;
        orig_paths.insert(path.clone(), row);
    }

    let mut queue = VecDeque::new();
    queue.push_back(working_dir.clone());

    let mut changes = ChangeSet::new();

    while let Some(dir) = queue.pop_front() {
        let mut dir_entries = match storage.read_dir(&dir).await {
            Ok(dir_entries) => dir_entries,
            Err(err) => {
                log::error!("Failed to read directory {:?}: {}", dir, err);
                continue;
            }
        };

        while let Some(dir_entry) = dir_entries.next_entry().await? {
            let file_path = dir_entry.path();
            let file_type = dir_entry.file_type().await?;

            if file_type.is_dir() {
                queue.push_back(file_path);
            } else if file_type.is_file() {
                let file = storage.open_file(&file_path).await?;
                let file_metadata = file.metadata().await?;

                // TODO: add to error converter and use `?`
                let relative_path = file_path.strip_prefix(&working_dir).unwrap();
                if let Some(orig_row) = orig_paths.remove(&relative_path.to_path_buf()) {
                    let file_hash = match orig_row.hash.code() {
                        MULTIHASH_SHA256_CHUNKED => {
                            calculate_sha256_chunked_checksum(file, file_metadata.len()).await?
                        }
                        _ => calculate_sha256_checksum(file).await?,
                    };

                    if file_hash != orig_row.hash {
                        changes.insert(
                            relative_path.to_path_buf(),
                            Change::Modified(Entry {
                                name: relative_path.into(),
                                place: file_path.into(),
                                size: file_metadata.len(),
                                hash: file_hash,
                            }),
                        );
                    }
                } else {
                    let sha256_hash =
                        calculate_sha256_chunked_checksum(file, file_metadata.len()).await?;
                    changes.insert(
                        relative_path.to_path_buf(),
                        Change::Added(Entry {
                            name: relative_path.into(),
                            place: file_path.into(),
                            size: file_metadata.len(),
                            hash: sha256_hash,
                        }),
                    );
                }
            } else {
                log::warn!("Unexpected file type: {:?}", file_path);
            }
        }
    }

    for (orig_path, orig_row) in orig_paths {
        changes.insert(orig_path.to_path_buf(), Change::Removed(orig_row));
    }

    let status = InstalledPackageStatus::new(lineage.clone().into(), changes);
    Ok((lineage, status))
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::checksum::ContentHash;
    use crate::lineage::CommitState;
    use crate::lineage::UpstreamState;
    use crate::mocks;

    #[tokio::test]
    async fn test_default_status() -> Res {
        let storage = mocks::storage::MockStorage::default();
        let (_lineage, status) = create_status(
            PackageLineage::default(),
            &storage,
            &Table::default(),
            PathBuf::default(),
        )
        .await?;
        assert_eq!(status, InstalledPackageStatus::default());
        Ok(())
    }

    #[tokio::test]
    async fn test_behind() -> Res {
        let base_hash = "AAA";
        let latest_hash = "BBB";
        let commit_hash = "AAA";
        let lineage = mocks::lineage::with_commit_hashes(base_hash, latest_hash, commit_hash);

        let (_lineage, status) = create_status(
            lineage,
            &mocks::storage::MockStorage::default(),
            &Table::default(),
            PathBuf::default(),
        )
        .await?;
        assert_eq!(status.upstream_state, UpstreamState::Behind);
        Ok(())
    }

    #[tokio::test]
    async fn test_ahead() -> Res {
        let base_hash = "AAA";
        let latest_hash = "AAA";
        let commit_hash = "BBB";
        let lineage = mocks::lineage::with_commit_hashes(base_hash, latest_hash, commit_hash);

        let (_, status) = create_status(
            lineage,
            &mocks::storage::MockStorage::default(),
            &Table::default(),
            PathBuf::default(),
        )
        .await?;
        assert_eq!(status.upstream_state, UpstreamState::Ahead);
        Ok(())
    }

    #[tokio::test]
    async fn test_diverged() -> Res {
        let lineage = PackageLineage {
            commit: Some(CommitState {
                hash: "aaa".to_string(),
                ..CommitState::default()
            }),
            base_hash: "bbb".to_string(),
            latest_hash: "ccc".to_string(),
            ..PackageLineage::default()
        };

        let (_, status) = create_status(
            lineage,
            &mocks::storage::MockStorage::default(),
            &Table::default(),
            PathBuf::default(),
        )
        .await?;
        assert_eq!(status.upstream_state, UpstreamState::Diverged);
        Ok(())
    }

    #[tokio::test]
    async fn test_removed_files() -> Res {
        let lineage = mocks::lineage::with_paths(vec![PathBuf::from("a/a")]);
        let manifest = mocks::manifest::with_record_keys(vec![PathBuf::from("a/a")]);
        let (_, status) = create_status(
            lineage,
            &mocks::storage::MockStorage::default(),
            &manifest,
            PathBuf::default(),
        )
        .await?;

        // It's "removed", because it's present in lineage and manifest,
        // but absent from file system (FIXME)
        let removed_file = status.changes.get(&PathBuf::from("a/a")).unwrap();
        assert!(matches!(removed_file, Change::Removed(_)));
        Ok(())
    }

    #[tokio::test]
    async fn test_added_files() -> Res {
        let lineage = PackageLineage::default();
        let manifest = Table::default();

        let storage = mocks::storage::MockStorage::default();
        let working_dir = storage.temp_dir.as_ref().join(PathBuf::from("foo/bar"));
        let file_path = PathBuf::from("inside/package/file.pq");
        let working_path = working_dir.join(&file_path);
        storage
            .write_file(&working_path, &std::fs::read(mocks::manifest::parquet())?)
            .await?;

        let (_, status) =
            create_status(lineage, &storage, &manifest, working_dir.to_path_buf()).await?;

        let added_file = status.changes.get(&file_path).unwrap();
        if let Change::Added(fingerprint) = added_file {
            assert_eq!(
                *fingerprint,
                Entry {
                    name: file_path.clone(),
                    place: working_path.into(),
                    size: 5324,
                    hash: ContentHash::SHA256Chunked(
                        "EfrtXWeClWPJ/IVKjQeAmMKhJV45/GcpjDm1IhvhJAY=".to_string()
                    )
                    .try_into()?,
                }
            );
            Ok(())
        } else {
            panic!()
        }
    }

    // TODO: add tests for every type of chunksum
}
