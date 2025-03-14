use multihash::Multihash;
use std::collections::HashMap;
use std::collections::VecDeque;
use std::path::Path;
use std::path::PathBuf;
use tokio::fs::File;

use tracing::{debug, info, warn};

use crate::checksum::calculate_sha256_checksum;
use crate::checksum::calculate_sha256_chunked_checksum;
use crate::checksum::MULTIHASH_SHA256_CHUNKED;
use crate::io::manifest::resolve_latest;
use crate::io::remote::Remote;
use crate::io::storage::Storage;
use crate::lineage::Change;
use crate::lineage::ChangeSet;
use crate::lineage::InstalledPackageStatus;
use crate::lineage::PackageLineage;
use crate::manifest::Row;
use crate::manifest::Table;
use crate::Error;
use crate::Res;

async fn verify_hash(file: File, hash: Multihash<256>) -> Res<Option<(u64, Multihash<256>)>> {
    let file_metadata = file.metadata().await?;
    let size = file_metadata.len();
    let calculated_hash = if hash.code() == MULTIHASH_SHA256_CHUNKED {
        calculate_sha256_chunked_checksum(file, size).await?
    } else {
        calculate_sha256_checksum(file).await?
    };

    if calculated_hash == hash {
        Ok(None)
    } else {
        Ok(Some((size, calculated_hash)))
    }
}

/// Refreshes the tracked `latest_hash` property in lineage.json
pub async fn refresh_latest_hash(
    mut lineage: PackageLineage,
    remote: &impl Remote,
) -> Res<PackageLineage> {
    let latest = resolve_latest(
        remote,
        &lineage.remote.catalog,
        &lineage.remote.clone().into(),
    )
    .await?;
    if lineage.latest_hash == latest.hash {
        return Ok(lineage);
    }
    lineage.latest_hash = latest.hash;
    Ok(lineage)
}

#[derive(Debug)]
enum WorkdirFile {
    Tracked(File, Row),
    NotTracked(File, Row),
    New(File),
    Removed(Row),
    UnSupported,
}

async fn locate_files_in_working_dir(
    storage: &(impl Storage + Sync),
    manifest: &Table,
    working_dir: impl AsRef<Path>,
    mut tracked_paths: HashMap<PathBuf, Row>,
) -> Res<Vec<(PathBuf, WorkdirFile)>> {
    let mut queue = VecDeque::new();
    queue.push_back(working_dir.as_ref().to_path_buf());

    let mut files = Vec::new();

    while let Some(dir) = queue.pop_front() {
        let mut dir_entries = match storage.read_dir(&dir).await {
            Ok(dir_entries) => dir_entries,
            Err(err) => {
                warn!("❌ Failed to read directory {}: {}", dir.display(), err);
                continue;
            }
        };

        while let Some(dir_entry) = dir_entries.next_entry().await? {
            let file_path = dir_entry.path();

            let file_type = dir_entry.file_type().await?;
            if !file_type.is_file() {
                if file_type.is_dir() {
                    queue.push_back(file_path);
                } else {
                    // TODO: handle symlinks
                    files.push((file_path, WorkdirFile::UnSupported));
                }
                continue;
            }

            let file = storage.open_file(&file_path).await?;
            let logical_key = file_path.strip_prefix(&working_dir)?.to_path_buf();
            if let Some(row) = tracked_paths.remove(&logical_key) {
                files.push((logical_key, WorkdirFile::Tracked(file, row)));
            } else if let Some(row) = manifest.get_record(&logical_key).await? {
                files.push((logical_key, WorkdirFile::NotTracked(file, row)));
            } else {
                files.push((logical_key, WorkdirFile::New(file)));
            }
        }
    }

    for (logical_key, row) in tracked_paths {
        files.push((logical_key, WorkdirFile::Removed(row)));
    }

    Ok(files)
}

async fn fingerprint_files(files: Vec<(PathBuf, WorkdirFile)>) -> Res<ChangeSet> {
    let mut changes = ChangeSet::new();
    for (logical_key, location) in files {
        match location {
            WorkdirFile::Tracked(file, row) => {
                if let Some((size, hash)) = verify_hash(file, row.hash).await? {
                    let row = Row { hash, size, ..row };
                    changes.insert(logical_key, Change::Modified(row));
                } else {
                    // the file is tracked (in lineage "paths") and has not been modified
                }
            }
            WorkdirFile::NotTracked(file, row) => {
                if let Some((size, hash)) = verify_hash(file, row.hash).await? {
                    let row = Row { hash, size, ..row };
                    changes.insert(logical_key, Change::Modified(row));
                } else {
                    debug!(
                        "✔️ File {} matches remote manifest but is not tracked locally",
                        logical_key.display()
                    );
                }
            }
            WorkdirFile::New(file) => {
                let size = file.metadata().await?.len();
                let hash = calculate_sha256_chunked_checksum(file, size).await?;
                let row = Row {
                    name: logical_key.clone(),
                    size,
                    hash,
                    ..Row::default()
                };
                changes.insert(logical_key, Change::Added(row));
            }
            WorkdirFile::Removed(row) => {
                changes.insert(logical_key, Change::Removed(row));
            }
            WorkdirFile::UnSupported => {
                // TODO: handle symlinks
                // TODO: changes.insert(path, Change::Broken)
                warn!("❌ Unexpected file type: {}", logical_key.display());
            }
        }
    }
    Ok(changes)
}

/// Creates the status of local modifications
/// It is used for `flow::commit` and for showing the status in UI.
pub async fn create_status(
    lineage: PackageLineage,
    storage: &(impl Storage + Sync),
    manifest: &Table,
    working_dir: impl AsRef<Path>,
) -> Res<(PackageLineage, InstalledPackageStatus)> {
    info!(
        "⏳ Creating status for working directory: {}",
        working_dir.as_ref().display()
    );

    // compute the status based on the following sources:
    //   - the cached manifest
    //   - paths
    //   - working directory state
    // installed entries marked as "installed" (initially as "downloading")
    // modified entries marked as "modified", etc

    debug!("⏳ Collecting paths from lineage");
    let mut orig_paths = HashMap::new();
    for path in lineage.paths.keys() {
        debug!("🔍 Checking manifest for path: {}", path.display());
        let row = manifest
            .get_record(path)
            .await?
            .ok_or(Error::ManifestPath(format!(
                "path {} not found in installed manifest",
                path.display()
            )))?;
        orig_paths.insert(path.clone(), row);
    }
    debug!("✔️ Found {} paths in lineage", orig_paths.len());

    let files = locate_files_in_working_dir(storage, manifest, working_dir, orig_paths).await?;
    debug!("✔️ Locatd files in working directory {:?}", files);
    let changes = fingerprint_files(files).await?;
    debug!("✔️ Computed file fingerprints {:?}", changes);

    debug!("⏳ Creating package status");
    let status = InstalledPackageStatus::new(lineage.clone().into(), changes);
    info!("✔️ Status created with {} changes", status.changes.len());
    Ok((lineage, status))
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::BTreeMap;

    use crate::checksum::ContentHash;
    use crate::fixtures;
    use crate::io::storage::mocks::MockStorage;
    use crate::lineage::CommitState;
    use crate::lineage::PathState;
    use crate::lineage::UpstreamState;

    #[tokio::test]
    async fn test_default_status() -> Res {
        let storage = MockStorage::default();
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
        let lineage = PackageLineage {
            commit: Some(CommitState {
                hash: "AAA".to_string(),
                ..CommitState::default()
            }),
            base_hash: "AAA".to_string(),
            latest_hash: "BBB".to_string(),
            ..PackageLineage::default()
        };

        let (_lineage, status) = create_status(
            lineage,
            &MockStorage::default(),
            &Table::default(),
            PathBuf::default(),
        )
        .await?;
        assert_eq!(status.upstream_state, UpstreamState::Behind);
        Ok(())
    }

    #[tokio::test]
    async fn test_ahead() -> Res {
        let lineage = PackageLineage {
            commit: Some(CommitState {
                hash: "BBB".to_string(),
                ..CommitState::default()
            }),
            base_hash: "AAA".to_string(),
            latest_hash: "AAA".to_string(),
            ..PackageLineage::default()
        };

        let (_, status) = create_status(
            lineage,
            &MockStorage::default(),
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
            &MockStorage::default(),
            &Table::default(),
            PathBuf::default(),
        )
        .await?;
        assert_eq!(status.upstream_state, UpstreamState::Diverged);
        Ok(())
    }

    #[tokio::test]
    async fn test_removed_files() -> Res {
        let logical_key = PathBuf::from("a/a");
        let storage = MockStorage::default();
        let lineage = PackageLineage {
            paths: BTreeMap::from([(
                logical_key.clone(),
                PathState {
                    hash: fixtures::sample_file_1::row_hash()?,
                    ..PathState::default()
                },
            )]),
            ..PackageLineage::default()
        };
        let mut manifest = Table::default();
        manifest
            .insert_record(Row {
                name: logical_key.clone(),
                hash: fixtures::sample_file_1::row_hash()?,
                ..Row::default()
            })
            .await?;
        let working_dir = storage.temp_dir.as_ref().join(PathBuf::from("foo/bar"));
        storage
            .write_file(
                working_dir.join(&logical_key),
                &std::fs::read(fixtures::manifest::jsonl()?)?,
            )
            .await?;

        // First, we create a status and see the file is not changed
        let (_, status) = create_status(lineage.clone(), &storage, &manifest, &working_dir).await?;
        let file_not_removed_yet = status.changes.get(&logical_key);
        assert!(file_not_removed_yet.is_none());

        // Then we remove the file and create a status again
        storage.remove_file(working_dir.join(&logical_key)).await?;
        let (_, status) = create_status(lineage, &storage, &manifest, working_dir).await?;
        // It's "removed", because it's present in lineage and manifest,
        // but absent from file system
        let removed_file = status.changes.get(&logical_key).unwrap();
        assert!(matches!(removed_file, Change::Removed(_)));
        assert!(!storage.exists(&logical_key).await);
        Ok(())
    }

    #[tokio::test]
    async fn test_added_files() -> Res {
        let lineage = PackageLineage::default();
        let manifest = Table::default();

        let storage = MockStorage::default();
        let working_dir = storage.temp_dir.as_ref().join(PathBuf::from("foo/bar"));
        let file_path = PathBuf::from("inside/package/file.pq");
        storage
            .write_file(
                working_dir.join(&file_path),
                &std::fs::read(fixtures::manifest::parquet()?)?,
            )
            .await?;

        let (_, status) = create_status(lineage, &storage, &manifest, working_dir).await?;

        let added_file = status.changes.get(&file_path).unwrap();
        if let Change::Added(added_row) = added_file {
            let reference_row = Row {
                name: PathBuf::from("inside/package/file.pq"),
                size: 5324,
                hash: ContentHash::SHA256Chunked(
                    "EfrtXWeClWPJ/IVKjQeAmMKhJV45/GcpjDm1IhvhJAY=".to_string(),
                )
                .try_into()?,
                ..Row::default()
            };
            assert_eq!(added_row, &reference_row);
            Ok(())
        } else {
            panic!()
        }
    }

    // TODO: add tests for every type of chunksum
}
