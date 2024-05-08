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
use crate::lineage::Change;
use crate::lineage::ChangeSet;
use crate::lineage::DiscreteChange;
use crate::lineage::InstalledPackageStatus;
use crate::lineage::PackageFileFingerprint;
use crate::lineage::PackageLineage;
use crate::manifest::Table;
use crate::Error;

// TODO: move to lineage
pub async fn refresh_latest_hash(
    mut lineage: PackageLineage,
    remote: &impl Remote,
) -> Result<PackageLineage, Error> {
    let latest_hash = resolve_latest(remote, lineage.remote.clone().into()).await?;
    if lineage.latest_hash == latest_hash {
        return Ok(lineage);
    }
    lineage.latest_hash = latest_hash;
    Ok(lineage)
}

pub async fn create_status(
    lineage: PackageLineage,
    storage: &(impl Storage + Sync),
    manifest: &Table,
    working_dir: PathBuf,
) -> Result<(PackageLineage, InstalledPackageStatus), Error> {
    // compute the status based on the following sources:
    //   - the cached manifest
    //   - paths
    //   - working directory state
    // installed entries marked as "installed" (initially as "downloading")
    // modified entries marked as "modified", etc

    let mut orig_paths = HashMap::new();
    for path in lineage.paths.keys() {
        let row = manifest.get_row(path).ok_or(Error::ManifestPath(format!(
            "path {:?} not found in installed manifest",
            path
        )))?;
        // TODO: use whole Row
        orig_paths.insert(path, (row.hash, row.size));
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
                if let Some((orig_hash, orig_size)) =
                    orig_paths.remove(&relative_path.to_path_buf())
                {
                    let file_hash = match orig_hash.code() {
                        MULTIHASH_SHA256_CHUNKED => {
                            calculate_sha256_chunked_checksum(file, file_metadata.len()).await?
                        }
                        _ => calculate_sha256_checksum(file).await?,
                    };

                    if file_hash != orig_hash {
                        changes.insert(
                            relative_path.to_path_buf(),
                            Change {
                                current: Some(PackageFileFingerprint {
                                    size: file_metadata.len(),
                                    hash: file_hash,
                                }),
                                previous: Some(PackageFileFingerprint {
                                    size: orig_size,
                                    hash: orig_hash,
                                }),
                                state: DiscreteChange::Modified,
                            },
                        );
                    }
                } else {
                    let sha256_hash =
                        calculate_sha256_chunked_checksum(file, file_metadata.len()).await?;
                    changes.insert(
                        relative_path.to_path_buf(),
                        Change {
                            current: Some(PackageFileFingerprint {
                                size: file_metadata.len(),
                                hash: sha256_hash,
                            }),
                            previous: None,
                            state: DiscreteChange::Added,
                        },
                    );
                }
            } else {
                log::warn!("Unexpected file type: {:?}", file_path);
            }
        }
    }

    for (orig_path, (orig_hash, orig_size)) in orig_paths {
        changes.insert(
            orig_path.to_path_buf(),
            Change {
                current: None,
                previous: Some(PackageFileFingerprint {
                    size: orig_size,
                    hash: orig_hash,
                }),
                state: DiscreteChange::Removed,
            },
        );
    }

    let status = InstalledPackageStatus::new(lineage.clone().into(), changes);
    Ok((lineage, status))
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::checksum::ContentHash;
    use crate::lineage::CommitState;
    use crate::lineage::UpstreamDiscreteState;
    use crate::mocks;

    #[tokio::test]
    async fn test_default_status() -> Result<(), Error> {
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
    async fn test_behind() -> Result<(), Error> {
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
        assert_eq!(status.upstream_state, UpstreamDiscreteState::Behind);
        Ok(())
    }

    #[tokio::test]
    async fn test_ahead() -> Result<(), Error> {
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
        assert_eq!(status.upstream_state, UpstreamDiscreteState::Ahead);
        Ok(())
    }

    #[tokio::test]
    async fn test_diverged() -> Result<(), Error> {
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
        assert_eq!(status.upstream_state, UpstreamDiscreteState::Diverged);
        Ok(())
    }

    #[tokio::test]
    async fn test_removed_files() -> Result<(), Error> {
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
        assert!(removed_file.current.is_none());
        assert!(removed_file.previous.is_some());
        Ok(())
    }

    #[tokio::test]
    async fn test_added_files() -> Result<(), Error> {
        let lineage = PackageLineage::default();
        let manifest = Table::default();

        let storage = mocks::storage::MockStorage::default();
        let working_dir = storage.temp_dir.as_ref().join(PathBuf::from("foo/bar"));
        let file_path = PathBuf::from("inside/package/file.pq");
        storage
            .write_file(
                working_dir.join(&file_path),
                &std::fs::read(mocks::manifest::parquet())?,
            )
            .await?;

        let (_, status) =
            create_status(lineage, &storage, &manifest, working_dir.to_path_buf()).await?;

        let added_file = status.changes.get(&file_path).unwrap();
        assert!(added_file.previous.is_none());
        if let Some(current) = &added_file.current {
            assert_eq!(current.size, 5324);
            let hash = current.hash;
            let hash_str: ContentHash = hash.try_into()?;
            assert_eq!(
                hash_str,
                ContentHash::SHA256Chunked(
                    "EfrtXWeClWPJ/IVKjQeAmMKhJV45/GcpjDm1IhvhJAY=".to_string()
                )
            );
            Ok(())
        } else {
            panic!()
        }
    }

    // TODO: add tests for every type of chunksum
}
