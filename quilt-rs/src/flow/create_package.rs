use std::collections::BTreeMap;
use std::path::Path;
use std::path::PathBuf;

use tracing::debug;
use tracing::info;

use url::Url;

use crate::checksum::calculate_hash;
use crate::io::manifest::build_manifest_from_rows_stream;
use crate::io::remote::HostConfig;
use crate::io::storage::Storage;
use crate::lineage::CommitState;
use crate::lineage::DomainLineage;
use crate::lineage::PackageLineage;
use crate::lineage::PathState;
use crate::manifest::ManifestHeader;
use crate::manifest::ManifestRow;
use crate::paths::DomainPaths;
use crate::quiltignore;
use crate::uri::Namespace;
use crate::InstallPackageError;
use crate::Error;
use crate::Res;

/// Walk a source directory recursively, collecting `(relative_path, absolute_path)` pairs.
/// Respects `.quiltignore` if present in the source directory.
async fn walk_source_dir(
    storage: &(impl Storage + Sync),
    source: &Path,
) -> Res<Vec<(PathBuf, PathBuf)>> {
    let quiltignore = quiltignore::load(source)?;

    let mut queue = std::collections::VecDeque::new();
    queue.push_back(source.to_path_buf());

    let mut files = Vec::new();

    while let Some(dir) = queue.pop_front() {
        let mut dir_entries = storage.read_dir(&dir).await?;

        while let Some(dir_entry) = dir_entries.next_entry().await? {
            let file_path = dir_entry.path();
            let file_type = dir_entry.file_type().await?;

            if file_type.is_dir() {
                if let Some(ref gi) = quiltignore {
                    let rel = file_path.strip_prefix(source)?;
                    if quiltignore::is_ignored(gi, rel, true) {
                        continue;
                    }
                }
                queue.push_back(file_path);
            } else if file_type.is_file() {
                let relative = file_path.strip_prefix(source)?.to_path_buf();
                if let Some(ref gi) = quiltignore {
                    if quiltignore::is_ignored(gi, &relative, false) {
                        continue;
                    }
                }
                files.push((relative, file_path));
            }
        }
    }

    files.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(files)
}

/// Create a new local-only package.
///
/// If `source` is provided, walks the source directory (respecting `.quiltignore`),
/// hashes each file, copies it to the objects store and the package home,
/// and creates an initial manifest with all the rows.
///
/// An initial commit is created so that `status` shows a clean state
/// (like `git init` + initial commit).
pub async fn create_package(
    lineage: DomainLineage,
    paths: &DomainPaths,
    storage: &(impl Storage + Sync),
    namespace: Namespace,
    source: Option<PathBuf>,
    message: Option<String>,
) -> Res<DomainLineage> {
    if lineage.packages.contains_key(&namespace) {
        return Err(Error::InstallPackage(InstallPackageError::AlreadyInstalled(namespace)));
    }

    info!("⏳ Creating package: {}", namespace);

    paths
        .scaffold_for_installing(storage, &lineage.home, &namespace)
        .await?;

    let package_home = crate::paths::package_home(&lineage.home, &namespace);
    let objects_dir = paths.objects_dir();
    let host_config = HostConfig::default();

    let mut rows: Vec<Res<ManifestRow>> = Vec::new();
    let mut lineage_paths: BTreeMap<PathBuf, PathState> = BTreeMap::new();

    if let Some(ref source_dir) = source {
        debug!("⏳ Walking source directory: {}", source_dir.display());
        let source_files = walk_source_dir(storage, source_dir).await?;
        debug!("✔️ Found {} files in source", source_files.len());

        for (relative_path, absolute_path) in source_files {
            // Hash the file
            let row = calculate_hash(storage, &absolute_path, &relative_path, &host_config).await?;

            // Copy to objects dir
            let object_dest = objects_dir.join(hex::encode(row.hash.digest()));
            if !storage.exists(&object_dest).await {
                storage.copy(&absolute_path, &object_dest).await?;
            }

            // Copy to package home (working copy)
            let work_dest = package_home.join(&relative_path);
            if let Some(parent) = work_dest.parent() {
                storage.create_dir_all(parent).await?;
            }
            storage.copy(&absolute_path, &work_dest).await?;

            // Build manifest row with physical_key pointing to objects
            let physical_key = Url::from_file_path(&object_dest)
                .map_err(|_| {
                    Error::Commit(format!("Failed to create URL from {:?}", &object_dest))
                })?
                .to_string();

            let manifest_row = ManifestRow {
                physical_key,
                ..row.clone()
            };

            // Track in lineage paths
            lineage_paths.insert(
                relative_path,
                PathState {
                    timestamp: storage.modified_timestamp(&work_dest).await?,
                    hash: row.hash.into(),
                },
            );

            rows.push(Ok(manifest_row));
        }
    }

    // Build initial manifest
    let header = ManifestHeader {
        message: Some(message.unwrap_or_else(|| "Created package".to_string())),
        ..ManifestHeader::default()
    };

    let stream = tokio_stream::iter(vec![Ok(rows)]);
    let dest_dir = paths.installed_manifests_dir(&namespace);
    let (_manifest_path, top_hash) =
        build_manifest_from_rows_stream(storage, dest_dir, header, stream).await?;

    info!("✔️ Initial manifest built with hash: {}", top_hash);

    // Create initial commit
    let commit = CommitState {
        hash: top_hash,
        timestamp: chrono::Utc::now(),
        prev_hashes: Vec::new(),
    };

    let package_lineage = PackageLineage {
        commit: Some(commit),
        remote_uri: None,
        base_hash: String::new(),
        latest_hash: String::new(),
        paths: lineage_paths,
    };

    let mut lineage = lineage;
    lineage.packages.insert(namespace.clone(), package_lineage);

    info!("✔️ Successfully created package: {}", namespace);
    Ok(lineage)
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_log::test;

    use aws_sdk_s3::primitives::ByteStream;

    use crate::io::storage::mocks::MockStorage;
    use crate::lineage::DomainLineage;
    use crate::manifest::Manifest;

    #[test(tokio::test)]
    async fn test_create_empty_package() -> Res {
        let (lineage, _temp_dir) = DomainLineage::from_temp_dir()?;
        let (paths, _temp_dir2) = DomainPaths::from_temp_dir()?;
        let storage = MockStorage::default();
        let namespace: Namespace = ("test", "pkg").into();

        let lineage =
            create_package(lineage, &paths, &storage, namespace.clone(), None, None).await?;

        assert!(lineage.packages.contains_key(&namespace));
        let pkg = lineage.packages.get(&namespace).unwrap();
        assert!(pkg.commit.is_some());
        assert!(pkg.remote_uri.is_none());
        assert!(pkg.paths.is_empty());
        assert!(pkg.base_hash.is_empty());
        assert!(pkg.latest_hash.is_empty());
        Ok(())
    }

    #[test(tokio::test)]
    async fn test_create_duplicate_namespace() -> Res {
        let (lineage, _temp_dir) = DomainLineage::from_temp_dir()?;
        let (paths, _temp_dir2) = DomainPaths::from_temp_dir()?;
        let storage = MockStorage::default();
        let namespace: Namespace = ("test", "pkg").into();

        let lineage =
            create_package(lineage, &paths, &storage, namespace.clone(), None, None).await?;

        let result = create_package(lineage, &paths, &storage, namespace.clone(), None, None).await;
        assert!(matches!(
            result.unwrap_err(),
            Error::InstallPackage(InstallPackageError::AlreadyInstalled(ns)) if ns == namespace
        ));
        Ok(())
    }

    #[test(tokio::test)]
    async fn test_create_with_source() -> Res {
        let (lineage, _temp_dir) = DomainLineage::from_temp_dir()?;
        let (paths, _temp_dir2) = DomainPaths::from_temp_dir()?;
        let storage = MockStorage::default();
        let namespace: Namespace = ("test", "src").into();

        // Create source directory with files
        let source_dir = storage.temp_dir.as_ref().join("source");
        storage.create_dir_all(&source_dir).await?;

        storage
            .write_byte_stream(
                source_dir.join("file1.txt"),
                ByteStream::from_static(b"hello world"),
            )
            .await?;
        storage
            .write_byte_stream(
                source_dir.join("file2.txt"),
                ByteStream::from_static(b"goodbye"),
            )
            .await?;

        let lineage = create_package(
            lineage,
            &paths,
            &storage,
            namespace.clone(),
            Some(source_dir),
            Some("Import from source".to_string()),
        )
        .await?;

        let pkg = lineage.packages.get(&namespace).unwrap();
        assert!(pkg.commit.is_some());
        assert_eq!(pkg.paths.len(), 2);
        assert!(pkg.paths.contains_key(&PathBuf::from("file1.txt")));
        assert!(pkg.paths.contains_key(&PathBuf::from("file2.txt")));
        Ok(())
    }

    #[test(tokio::test)]
    async fn test_create_with_quiltignore() -> Res {
        let (lineage, _temp_dir) = DomainLineage::from_temp_dir()?;
        let (paths, _temp_dir2) = DomainPaths::from_temp_dir()?;
        let storage = MockStorage::default();
        let namespace: Namespace = ("test", "ignore").into();

        // Create source directory with files and .quiltignore
        let source_dir = storage.temp_dir.as_ref().join("source");
        storage.create_dir_all(&source_dir).await?;

        storage
            .write_byte_stream(
                source_dir.join("data.csv"),
                ByteStream::from_static(b"a,b,c"),
            )
            .await?;
        storage
            .write_byte_stream(
                source_dir.join("notes.log"),
                ByteStream::from_static(b"some log"),
            )
            .await?;
        storage
            .write_byte_stream(
                source_dir.join(".quiltignore"),
                ByteStream::from_static(b"*.log"),
            )
            .await?;

        let lineage = create_package(
            lineage,
            &paths,
            &storage,
            namespace.clone(),
            Some(source_dir),
            None,
        )
        .await?;

        let pkg = lineage.packages.get(&namespace).unwrap();
        assert!(
            pkg.paths.contains_key(&PathBuf::from("data.csv")),
            "data.csv should be included"
        );
        assert!(
            !pkg.paths.contains_key(&PathBuf::from("notes.log")),
            "notes.log should be ignored"
        );
        // .quiltignore is not matched by *.log, so it's included
        assert!(
            pkg.paths.contains_key(&PathBuf::from(".quiltignore")),
            ".quiltignore should be included (not matched by *.log)"
        );
        Ok(())
    }

    #[test(tokio::test)]
    async fn test_create_with_custom_message() -> Res {
        let (lineage, _temp_dir) = DomainLineage::from_temp_dir()?;
        let (paths, _temp_dir2) = DomainPaths::from_temp_dir()?;
        let storage = MockStorage::default();
        let namespace: Namespace = ("test", "msg").into();

        let lineage = create_package(
            lineage,
            &paths,
            &storage,
            namespace.clone(),
            None,
            Some("My custom message".to_string()),
        )
        .await?;

        let pkg = lineage.packages.get(&namespace).unwrap();
        let commit = pkg.commit.as_ref().unwrap();
        let manifest_path = paths.installed_manifest(&namespace, &commit.hash);
        let manifest = Manifest::from_path(&storage, &manifest_path).await?;
        assert_eq!(
            manifest.header.message,
            Some("My custom message".to_string())
        );
        Ok(())
    }
}
