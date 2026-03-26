use std::collections::HashMap;
use std::collections::VecDeque;
use std::path::Path;
use std::path::PathBuf;

use ignore::gitignore::Gitignore;
use tracing::debug;
use tracing::info;
use tracing::warn;

use crate::checksum::calculate_hash;
use crate::checksum::verify_hash;
use crate::io::manifest::resolve_tag;
use crate::io::remote::HostConfig;
use crate::io::remote::Remote;
use crate::io::storage::Storage;
use crate::lineage::Change;
use crate::lineage::ChangeSet;
use crate::lineage::InstalledPackageStatus;
use crate::lineage::PackageLineage;
use crate::manifest::Manifest;
use crate::manifest::ManifestRow;
use crate::uri::Tag;
use crate::Error;
use crate::Res;

/// Refreshes the tracked `latest_hash` property in lineage.json
pub async fn refresh_latest_hash(
    mut lineage: PackageLineage,
    remote: &impl Remote,
) -> Res<PackageLineage> {
    let latest = resolve_tag(
        remote,
        &lineage.remote.origin,
        &lineage.remote.clone().into(),
        Tag::Latest,
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
    Tracked(PathBuf, ManifestRow),
    NotTracked(PathBuf, ManifestRow),
    New(PathBuf),
    Removed(ManifestRow),
    UnSupported,
}

async fn locate_files_in_package_home(
    storage: &(impl Storage + Sync),
    manifest: &Manifest,
    package_home: impl AsRef<Path>,
    mut tracked_paths: HashMap<PathBuf, ManifestRow>,
    quiltignore: Option<&Gitignore>,
) -> Res<Vec<(PathBuf, WorkdirFile)>> {
    let mut queue = VecDeque::new();
    queue.push_back(package_home.as_ref().to_path_buf());

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
                    if let Some(gi) = quiltignore {
                        let rel = file_path.strip_prefix(&package_home)?;
                        if crate::quiltignore::is_ignored(gi, rel, true) {
                            continue;
                        }
                    }
                    queue.push_back(file_path);
                } else {
                    // TODO: handle symlinks
                    files.push((file_path, WorkdirFile::UnSupported));
                }
                continue;
            }

            let logical_key = file_path.strip_prefix(&package_home)?.to_path_buf();
            if let Some(gi) = quiltignore {
                if crate::quiltignore::is_ignored(gi, &logical_key, false) {
                    continue;
                }
            }
            if let Some(row) = tracked_paths.remove(&logical_key) {
                files.push((logical_key, WorkdirFile::Tracked(file_path, row)));
            } else if let Some(row) = manifest.get_record(&logical_key) {
                files.push((logical_key, WorkdirFile::NotTracked(file_path, row.clone())));
            } else {
                files.push((logical_key, WorkdirFile::New(file_path)));
            }
        }
    }

    for (logical_key, row) in tracked_paths {
        files.push((logical_key, WorkdirFile::Removed(row)));
    }

    Ok(files)
}

async fn detect_change(
    storage: &(impl Storage + Sync),
    logical_key: &Path,
    location: WorkdirFile,
    host_config: &HostConfig,
) -> Res<Option<Change>> {
    match location {
        WorkdirFile::Tracked(path, row) => verify_hash(storage, &path, row, host_config)
            .await
            .map(|opt_row| opt_row.map(Change::Modified)),
        WorkdirFile::NotTracked(path, row) => verify_hash(storage, &path, row, host_config)
            .await
            .map(|opt_row| opt_row.map(Change::Modified)),
        WorkdirFile::New(path) => calculate_hash(storage, &path, logical_key, host_config)
            .await
            .map(|row| Some(Change::Added(row))),
        WorkdirFile::Removed(row) => Ok(Some(Change::Removed(row))),
        WorkdirFile::UnSupported => {
            // TODO: handle symlinks
            // TODO: changes.insert(path, Change::Broken)
            warn!("❌ Unexpected file type: {}", logical_key.display());
            Ok(None)
        }
    }
}

async fn fingerprint_files(
    storage: &(impl Storage + Sync),
    files: Vec<(PathBuf, WorkdirFile)>,
    host_config: HostConfig,
) -> Res<ChangeSet> {
    let mut changes = ChangeSet::new();
    for (logical_key, location) in files {
        if let Some(change) = detect_change(storage, &logical_key, location, &host_config).await? {
            changes.insert(logical_key, change);
        }
    }
    Ok(changes)
}

/// Creates the status of local modifications
/// It is used for `flow::commit` and for showing the status in UI.
pub async fn create_status(
    lineage: PackageLineage,
    storage: &(impl Storage + Sync),
    manifest: &Manifest,
    package_home: impl AsRef<Path>,
    host_config: HostConfig,
) -> Res<(PackageLineage, InstalledPackageStatus)> {
    info!(
        "⏳ Creating status for working directory: {}",
        package_home.as_ref().display()
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
            .ok_or(Error::ManifestPath(format!(
                "path {} not found in installed manifest",
                path.display()
            )))?;
        orig_paths.insert(path.clone(), row.clone());
    }
    debug!("✔️ Found {} paths in lineage", orig_paths.len());

    let quiltignore = crate::quiltignore::load(package_home.as_ref())?;
    let files =
        locate_files_in_package_home(storage, manifest, package_home, orig_paths, quiltignore.as_ref()).await?;
    debug!("✔️ Located files in working directory {:?}", files);
    let changes = fingerprint_files(storage, files, host_config).await?;
    debug!("✔️ Computed file fingerprints {:?}", changes);

    debug!("⏳ Creating package status");
    let status = InstalledPackageStatus::new(lineage.clone().into(), changes);
    info!("✔️ Status created with {} changes", status.changes.len());
    Ok((lineage, status))
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_log::test;

    use std::collections::BTreeMap;

    use aws_sdk_s3::primitives::ByteStream;

    use crate::checksum::Crc64Hash;
    use crate::checksum::Sha256ChunkedHash;
    use crate::fixtures;
    use crate::io::remote::HostChecksums;
    use crate::io::storage::mocks::MockStorage;
    use crate::lineage::CommitState;
    use crate::lineage::PathState;
    use crate::lineage::UpstreamState;

    #[test(tokio::test)]
    async fn test_default_status() -> Res {
        let storage = MockStorage::default();
        let (_lineage, status) = create_status(
            PackageLineage::default(),
            &storage,
            &Manifest::default(),
            PathBuf::default(),
            HostConfig::default(),
        )
        .await?;
        assert_eq!(status.upstream_state, UpstreamState::default());
        assert!(status.changes.is_empty());
        Ok(())
    }

    #[test(tokio::test)]
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
            &Manifest::default(),
            PathBuf::default(),
            HostConfig::default(),
        )
        .await?;
        assert_eq!(status.upstream_state, UpstreamState::Behind);
        Ok(())
    }

    #[test(tokio::test)]
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
            &Manifest::default(),
            PathBuf::default(),
            HostConfig::default(),
        )
        .await?;
        assert_eq!(status.upstream_state, UpstreamState::Ahead);
        Ok(())
    }

    #[test(tokio::test)]
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
            &Manifest::default(),
            PathBuf::default(),
            HostConfig::default(),
        )
        .await?;
        assert_eq!(status.upstream_state, UpstreamState::Diverged);
        Ok(())
    }

    #[test(tokio::test)]
    async fn test_removed_files() -> Res {
        let manifest = fixtures::manifest_with_objects_all_sizes::manifest().await?;
        let logical_key = PathBuf::from("less-then-8mb.txt");
        let manifest_record = manifest.get_record(&logical_key).unwrap();
        let storage = MockStorage::default();
        let lineage = PackageLineage {
            paths: BTreeMap::from([(
                logical_key.clone(),
                PathState {
                    hash: manifest_record.hash.clone().into(),
                    ..PathState::default()
                },
            )]),
            ..PackageLineage::default()
        };
        let working_dir = storage.temp_dir.as_ref().join(PathBuf::from("foo/bar"));
        storage
            .write_byte_stream(
                working_dir.join(&logical_key),
                ByteStream::from_static(fixtures::objects::less_than_8mb()),
            )
            .await?;

        // First, we create a status and see the file is not changed
        let (_, status) = create_status(
            lineage.clone(),
            &storage,
            &manifest,
            &working_dir,
            HostConfig::default(),
        )
        .await?;
        let file_not_removed_yet = status.changes.get(&logical_key);
        assert!(file_not_removed_yet.is_none());

        // Then we remove the file and create a status again
        storage.remove_file(working_dir.join(&logical_key)).await?;
        let (_, status) = create_status(
            lineage,
            &storage,
            &manifest,
            working_dir,
            HostConfig::default(),
        )
        .await?;
        // It's "removed", because it's present in lineage and manifest,
        // but absent from file system
        let removed_file = status.changes.get(&logical_key).unwrap();
        assert!(matches!(removed_file, Change::Removed(_)));
        assert!(!storage.exists(&logical_key).await);
        Ok(())
    }

    #[test(tokio::test)]
    async fn test_added_files() -> Res {
        let lineage = PackageLineage::default();
        let manifest = Manifest::default();

        let storage = MockStorage::default();
        let working_dir = storage.temp_dir.as_ref().join(PathBuf::from("foo/bar"));
        let logical_key = PathBuf::from("inside/package/file.pq");
        let physical_key = working_dir.join(&logical_key);
        storage
            .write_byte_stream(
                &physical_key,
                ByteStream::from_static(fixtures::objects::less_than_8mb()),
            )
            .await?;

        let (_, status) = create_status(
            lineage,
            &storage,
            &manifest,
            working_dir.clone(),
            HostConfig::default(),
        )
        .await?;

        let added_file = status.changes.get(&logical_key).unwrap();
        if let Change::Added(added_row) = added_file {
            let reference_row = ManifestRow {
                logical_key,
                size: 16,
                hash: Sha256ChunkedHash::try_from(fixtures::objects::LESS_THAN_8MB_HASH_B64)?
                    .into(),
                meta: None,
                physical_key: format!("file://{}", physical_key.display()),
            };
            assert_eq!(added_row, &reference_row);
            Ok(())
        } else {
            panic!("Expected Change::Added, got {:?}", added_file)
        }
    }

    #[test(tokio::test)]
    async fn test_added_files_crc64() -> Res {
        let lineage = PackageLineage::default();
        let manifest = Manifest::default();

        let storage = MockStorage::default();
        let working_dir = storage.temp_dir.as_ref();
        let file_path = PathBuf::from("some.pq");
        storage
            .write_byte_stream(
                working_dir.join(&file_path),
                ByteStream::from_static(fixtures::objects::less_than_8mb()),
            )
            .await?;

        // Use CRC64 host configuration
        let host_config = HostConfig {
            checksums: HostChecksums::Crc64,
            host: None,
        };

        let (_, status) =
            create_status(lineage, &storage, &manifest, working_dir, host_config).await?;

        let added_file = status.changes.get(&file_path).unwrap();
        if let Change::Added(added_row) = added_file {
            let reference_row = ManifestRow {
                logical_key: PathBuf::from("some.pq"),
                size: 16,
                hash: Crc64Hash::try_from("CRSFynAYcw4=")?.into(),
                ..ManifestRow::default()
            };
            assert_eq!(added_row, &reference_row);
            Ok(())
        } else {
            panic!("Expected Change::Added, got {:?}", added_file)
        }
    }

    // TODO: add tests for every type of chunksum

    #[test(tokio::test)]
    async fn test_quiltignore_basic_exclusion() -> Res {
        let storage = MockStorage::default();
        let working_dir = storage.temp_dir.as_ref().join("pkg");

        // Create files
        storage
            .write_byte_stream(
                working_dir.join("data.csv"),
                ByteStream::from_static(b"csv data"),
            )
            .await?;
        storage
            .write_byte_stream(
                working_dir.join("script.py"),
                ByteStream::from_static(b"python code"),
            )
            .await?;

        // Create .quiltignore
        std::fs::write(working_dir.join(".quiltignore"), "*.py\n").unwrap();

        let (_, status) = create_status(
            PackageLineage::default(),
            &storage,
            &Manifest::default(),
            &working_dir,
            HostConfig::default(),
        )
        .await?;

        assert!(status.changes.contains_key(&PathBuf::from("data.csv")));
        assert!(!status.changes.contains_key(&PathBuf::from("script.py")));
        Ok(())
    }

    #[test(tokio::test)]
    async fn test_quiltignore_directory_exclusion() -> Res {
        let storage = MockStorage::default();
        let working_dir = storage.temp_dir.as_ref().join("pkg");

        storage
            .write_byte_stream(
                working_dir.join("cache/file.txt"),
                ByteStream::from_static(b"cached"),
            )
            .await?;
        storage
            .write_byte_stream(
                working_dir.join("keep.txt"),
                ByteStream::from_static(b"keep"),
            )
            .await?;

        std::fs::write(working_dir.join(".quiltignore"), "cache/\n").unwrap();

        let (_, status) = create_status(
            PackageLineage::default(),
            &storage,
            &Manifest::default(),
            &working_dir,
            HostConfig::default(),
        )
        .await?;

        assert!(status.changes.contains_key(&PathBuf::from("keep.txt")));
        assert!(!status.changes.contains_key(&PathBuf::from("cache/file.txt")));
        Ok(())
    }

    #[test(tokio::test)]
    async fn test_quiltignore_negation() -> Res {
        let storage = MockStorage::default();
        let working_dir = storage.temp_dir.as_ref().join("pkg");

        storage
            .write_byte_stream(
                working_dir.join("debug.log"),
                ByteStream::from_static(b"debug"),
            )
            .await?;
        storage
            .write_byte_stream(
                working_dir.join("important.log"),
                ByteStream::from_static(b"important"),
            )
            .await?;

        std::fs::write(working_dir.join(".quiltignore"), "*.log\n!important.log\n").unwrap();

        let (_, status) = create_status(
            PackageLineage::default(),
            &storage,
            &Manifest::default(),
            &working_dir,
            HostConfig::default(),
        )
        .await?;

        assert!(!status.changes.contains_key(&PathBuf::from("debug.log")));
        assert!(status.changes.contains_key(&PathBuf::from("important.log")));
        Ok(())
    }

    #[test(tokio::test)]
    async fn test_quiltignore_comments_and_blank_lines() -> Res {
        let storage = MockStorage::default();
        let working_dir = storage.temp_dir.as_ref().join("pkg");

        storage
            .write_byte_stream(
                working_dir.join("file.tmp"),
                ByteStream::from_static(b"tmp"),
            )
            .await?;
        storage
            .write_byte_stream(
                working_dir.join("file.txt"),
                ByteStream::from_static(b"txt"),
            )
            .await?;

        std::fs::write(
            working_dir.join(".quiltignore"),
            "# this is a comment\n\n*.tmp\n",
        )
        .unwrap();

        let (_, status) = create_status(
            PackageLineage::default(),
            &storage,
            &Manifest::default(),
            &working_dir,
            HostConfig::default(),
        )
        .await?;

        assert!(!status.changes.contains_key(&PathBuf::from("file.tmp")));
        assert!(status.changes.contains_key(&PathBuf::from("file.txt")));
        Ok(())
    }

    #[test(tokio::test)]
    async fn test_quiltignore_self_exclusion() -> Res {
        let storage = MockStorage::default();
        let working_dir = storage.temp_dir.as_ref().join("pkg");

        storage
            .write_byte_stream(
                working_dir.join("data.csv"),
                ByteStream::from_static(b"data"),
            )
            .await?;

        std::fs::write(working_dir.join(".quiltignore"), ".quiltignore\n").unwrap();

        let (_, status) = create_status(
            PackageLineage::default(),
            &storage,
            &Manifest::default(),
            &working_dir,
            HostConfig::default(),
        )
        .await?;

        assert!(status.changes.contains_key(&PathBuf::from("data.csv")));
        assert!(!status.changes.contains_key(&PathBuf::from(".quiltignore")));
        Ok(())
    }

    #[test(tokio::test)]
    async fn test_quiltignore_tracked_file_becomes_removed() -> Res {
        let manifest = fixtures::manifest_with_objects_all_sizes::manifest().await?;
        let logical_key = PathBuf::from("less-then-8mb.txt");
        let manifest_record = manifest.get_record(&logical_key).unwrap();
        let storage = MockStorage::default();
        let working_dir = storage.temp_dir.as_ref().join("pkg");

        let lineage = PackageLineage {
            paths: BTreeMap::from([(
                logical_key.clone(),
                PathState {
                    hash: manifest_record.hash.clone().into(),
                    ..PathState::default()
                },
            )]),
            ..PackageLineage::default()
        };

        // Write the tracked file to disk
        storage
            .write_byte_stream(
                working_dir.join(&logical_key),
                ByteStream::from_static(fixtures::objects::less_than_8mb()),
            )
            .await?;

        // Add a .quiltignore that excludes the tracked file
        std::fs::write(working_dir.join(".quiltignore"), "*.txt\n").unwrap();

        let (_, status) = create_status(
            lineage,
            &storage,
            &manifest,
            &working_dir,
            HostConfig::default(),
        )
        .await?;

        // The file is ignored by .quiltignore, so it won't be found in the walk.
        // Since it's in lineage.paths but not found, it appears as Removed.
        let change = status.changes.get(&logical_key).unwrap();
        assert!(matches!(change, Change::Removed(_)));
        Ok(())
    }
}
