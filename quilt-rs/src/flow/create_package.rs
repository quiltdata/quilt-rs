use std::collections::BTreeMap;
use std::path::Path;
use std::path::PathBuf;

use tracing::debug;
use tracing::info;
use tracing::warn;

use url::Url;

use super::UserMeta;
use crate::Error;
use crate::InstallPackageError;
use crate::Res;
use crate::checksum::calculate_hash;
use crate::error::PackageOpError;
use crate::io::manifest::build_manifest_from_rows_stream;
use crate::io::remote::HostConfig;
use crate::io::storage::Storage;
use crate::lineage::CommitState;
use crate::lineage::Home;
use crate::lineage::PackageLineage;
use crate::lineage::PathState;
use crate::manifest::HeaderMessage;
use crate::manifest::ManifestHeader;
use crate::manifest::ManifestRow;
use crate::paths::DomainPaths;
use crate::quiltignore;
use crate::quiltignore::IgnoredBy;
use quilt_uri::Namespace;

/// What the ignore rules kept out of a package created from a folder.
/// The paths are relative to the source; an ignored folder is one item, not walked into.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LeftOut {
    pub files: Vec<(PathBuf, IgnoredBy)>,
    pub dirs: Vec<(PathBuf, IgnoredBy)>,
}

impl LeftOut {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.files.is_empty() && self.dirs.is_empty()
    }
}

/// "2 files and 1 folder were left out by ignore rules", or nothing when nothing was.
impl std::fmt::Display for LeftOut {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        fn count(n: usize, noun: &str) -> Option<String> {
            match n {
                0 => None,
                1 => Some(format!("1 {noun}")),
                n => Some(format!("{n} {noun}s")),
            }
        }
        let words: Vec<String> = [
            count(self.files.len(), "file"),
            count(self.dirs.len(), "folder"),
        ]
        .into_iter()
        .flatten()
        .collect();
        if words.is_empty() {
            return Ok(());
        }
        let verb = if self.files.len() + self.dirs.len() == 1 {
            "was"
        } else {
            "were"
        };
        write!(f, "{} {verb} left out by ignore rules", words.join(" and "))
    }
}

/// Walk a source directory recursively, collecting `(relative_path, absolute_path)` pairs.
/// Respects the ignore rules: the source's `.quiltignore`, the user-wide file and
/// the built-in defaults. Every file is new to the package.
async fn walk_source_dir(
    storage: &(impl Storage + Sync),
    paths: &DomainPaths,
    source: &Path,
) -> Res<(Vec<(PathBuf, PathBuf)>, LeftOut)> {
    let rules = quiltignore::Rules::load(source, &paths.user_quiltignore())?;

    let mut queue = std::collections::VecDeque::new();
    queue.push_back(source.to_path_buf());

    let mut files = Vec::new();
    let mut left_out = LeftOut::default();

    while let Some(dir) = queue.pop_front() {
        let mut dir_entries = storage.read_dir(&dir).await?;

        while let Some(dir_entry) = dir_entries.next_entry().await? {
            let file_path = dir_entry.path();
            let file_type = dir_entry.file_type().await?;

            if file_type.is_dir() {
                let rel = crate::paths::logical_key(file_path.strip_prefix(source)?);
                if let Some(by) = rules.dir_ignored_by(&rel, false) {
                    left_out.dirs.push((rel, by));
                    continue;
                }
                queue.push_back(file_path);
            } else if file_type.is_file() {
                let relative = crate::paths::logical_key(file_path.strip_prefix(source)?);
                if let Err(err) = crate::LogicalKey::new(&relative) {
                    warn!("Left out of the package: {err}");
                    continue;
                }
                if let Some(by) = rules.file_ignored_by(&relative, false) {
                    left_out.files.push((relative, by));
                    continue;
                }
                files.push((relative, file_path));
            }
        }
    }

    files.sort_by(|a, b| a.0.cmp(&b.0));
    left_out.files.sort_by(|a, b| a.0.cmp(&b.0));
    left_out.dirs.sort_by(|a, b| a.0.cmp(&b.0));
    Ok((files, left_out))
}

/// Create a new local-only package.
///
/// If `source` is provided, walks the source directory (respecting `.quiltignore`),
/// hashes each file, copies it to the objects store and the package home,
/// and creates an initial manifest with all the rows.
///
/// An initial commit is created so that `status` shows a clean state
/// (like `git init` + initial commit). `user_meta` is applied as on a later
/// commit, except `Keep` is treated as `Clear`: there is no previous revision,
/// so the header has no `user_meta` key unless `Set` provides one.
///
/// Returns the package's lineage for the caller to store, and what the ignore
/// rules left out of `source`: those files stay there, and the new package
/// never sees them. `installed` says whether the domain has the package
/// already.
#[allow(clippy::too_many_arguments, reason = "the parts a create uses")]
pub async fn create_package(
    home: &Home,
    installed: bool,
    paths: &DomainPaths,
    storage: &(impl Storage + Sync),
    namespace: Namespace,
    source: Option<PathBuf>,
    message: Option<String>,
    user_meta: UserMeta,
) -> Res<(PackageLineage, LeftOut)> {
    if installed {
        return Err(Error::InstallPackage(
            InstallPackageError::AlreadyInstalled(namespace),
        ));
    }

    info!("⏳ Creating package: {}", namespace);

    paths
        .scaffold_for_installing(storage, home, &namespace)
        .await?;

    let package_home = crate::paths::package_home(home, &namespace);
    let objects_dir = paths.objects_dir();
    let host_config = HostConfig::default();

    let mut rows: Vec<Res<ManifestRow>> = Vec::new();
    let mut lineage_paths: BTreeMap<PathBuf, PathState> = BTreeMap::new();
    let mut left_out = LeftOut::default();

    if let Some(ref source_dir) = source {
        debug!("⏳ Walking source directory: {}", source_dir.display());
        let source_files;
        (source_files, left_out) = walk_source_dir(storage, paths, source_dir).await?;
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
            let work_dest = row.logical_key.join_to(&package_home);
            if let Some(parent) = work_dest.parent() {
                storage.create_dir_all(parent).await?;
            }
            storage.copy(&absolute_path, &work_dest).await?;

            // Build manifest row with physical_key pointing to objects
            let physical_key = Url::from_file_path(&object_dest)
                .map_err(|()| {
                    Error::PackageOp(PackageOpError::Commit(format!(
                        "Failed to create URL from {}",
                        object_dest.display()
                    )))
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
    let mut header = ManifestHeader {
        message: HeaderMessage::Text(message.unwrap_or_else(|| "Created package".to_string())),
        user_meta: None,
        user_meta_numbers: crate::manifest::RawNumbers::default(),
        ..ManifestHeader::default()
    };
    match user_meta {
        UserMeta::Keep | UserMeta::Clear => {}
        UserMeta::Set(value, numbers) => header.set_meta(value, numbers),
    }

    let stream = tokio_stream::iter(vec![Ok(rows)]);
    let dest_dir = paths.installed_manifests_dir(&namespace);
    let (_manifest_path, top_hash) =
        build_manifest_from_rows_stream(storage, dest_dir, header, Vec::new(), stream).await?;

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
        ..PackageLineage::default()
    };

    info!("✔️ Successfully created package: {}", namespace);
    Ok((package_lineage, left_out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_log::test;

    use aws_sdk_s3::primitives::ByteStream;

    use crate::io::storage::mocks::MockStorage;
    use crate::manifest::Manifest;

    #[test(tokio::test)]
    async fn test_create_empty_package() -> Res {
        let (home, _temp_dir) = Home::from_temp_dir()?;
        let (paths, _temp_dir2) = DomainPaths::from_temp_dir()?;
        let storage = MockStorage::default();
        let namespace: Namespace = ("test", "pkg").into();

        let (pkg, _) = create_package(
            &home,
            false,
            &paths,
            &storage,
            namespace.clone(),
            None,
            None,
            UserMeta::Clear,
        )
        .await?;

        assert!(pkg.commit.is_some());
        assert!(pkg.remote_uri.is_none());
        assert!(pkg.paths.is_empty());
        assert_eq!(pkg.base_hash, "");
        assert_eq!(pkg.latest_hash, "");
        Ok(())
    }

    #[test(tokio::test)]
    async fn test_create_duplicate_namespace() -> Res {
        let (home, _temp_dir) = Home::from_temp_dir()?;
        let (paths, _temp_dir2) = DomainPaths::from_temp_dir()?;
        let storage = MockStorage::default();
        let namespace: Namespace = ("test", "pkg").into();

        let result = create_package(
            &home,
            true,
            &paths,
            &storage,
            namespace.clone(),
            None,
            None,
            UserMeta::Clear,
        )
        .await;
        assert!(matches!(
            result.unwrap_err(),
            Error::InstallPackage(InstallPackageError::AlreadyInstalled(ns)) if ns == namespace
        ));
        Ok(())
    }

    #[test(tokio::test)]
    async fn test_create_with_source() -> Res {
        let (home, _temp_dir) = Home::from_temp_dir()?;
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

        let (pkg, _) = create_package(
            &home,
            false,
            &paths,
            &storage,
            namespace.clone(),
            Some(source_dir),
            Some("Import from source".to_string()),
            UserMeta::Clear,
        )
        .await?;
        assert!(pkg.commit.is_some());
        assert_eq!(pkg.paths.len(), 2);
        assert!(pkg.paths.contains_key(&PathBuf::from("file1.txt")));
        assert!(pkg.paths.contains_key(&PathBuf::from("file2.txt")));
        Ok(())
    }

    /// A logical key separates folders with `/` on every OS, though the walk
    /// finds `\\` on Windows. Compared as text: Windows paths treat both
    /// separators as one, so a `PathBuf` comparison would pass either way.
    #[test(tokio::test)]
    async fn test_create_with_a_folder_uses_slash_keys() -> Res {
        let (home, _temp_dir) = Home::from_temp_dir()?;
        let (paths, _temp_dir2) = DomainPaths::from_temp_dir()?;
        let storage = MockStorage::default();
        let namespace: Namespace = ("test", "nested").into();

        let source_dir = storage.temp_dir.as_ref().join("source");
        storage
            .write_byte_stream(
                source_dir.join("data").join("raw").join("a.csv"),
                ByteStream::from_static(b"a,b\n"),
            )
            .await?;

        let (pkg, _) = create_package(
            &home,
            false,
            &paths,
            &storage,
            namespace.clone(),
            Some(source_dir),
            None,
            UserMeta::Clear,
        )
        .await?;
        let keys: Vec<&str> = pkg
            .paths
            .keys()
            .map(|key| key.to_str().expect("UTF-8"))
            .collect();
        assert_eq!(keys, ["data/raw/a.csv"]);
        let hash = &pkg.commit.as_ref().unwrap().hash;
        let manifest =
            Manifest::from_path(&storage, &paths.installed_manifest(&namespace, hash)).await?;
        let rows: Vec<&str> = manifest
            .rows
            .iter()
            .map(|row| row.logical_key.to_str().expect("UTF-8"))
            .collect();
        assert_eq!(rows, ["data/raw/a.csv"]);
        Ok(())
    }

    #[test(tokio::test)]
    async fn test_create_with_quiltignore() -> Res {
        let (home, _temp_dir) = Home::from_temp_dir()?;
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

        let (pkg, left_out) = create_package(
            &home,
            false,
            &paths,
            &storage,
            namespace.clone(),
            Some(source_dir),
            None,
            UserMeta::Clear,
        )
        .await?;
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
        assert_eq!(
            left_out.files,
            vec![(
                PathBuf::from("notes.log"),
                IgnoredBy {
                    pattern: "*.log".to_string(),
                    source: quiltignore::RuleSource::Package { line: 1 },
                }
            )]
        );
        assert_eq!(left_out.dirs, Vec::new());
        assert_eq!(left_out.to_string(), "1 file was left out by ignore rules");
        Ok(())
    }

    #[test(tokio::test)]
    async fn test_create_skips_default_ignored_files() -> Res {
        let (home, _temp_dir) = Home::from_temp_dir()?;
        let (paths, _temp_dir2) = DomainPaths::from_temp_dir()?;
        let storage = MockStorage::default();
        let namespace: Namespace = ("test", "defaults").into();

        let source_dir = storage.temp_dir.as_ref().join("source");
        for name in [".DS_Store", ".git/config", ".zattrs", "data.csv"] {
            storage
                .write_byte_stream(source_dir.join(name), ByteStream::from_static(b"x"))
                .await?;
        }

        let (pkg, left_out) = create_package(
            &home,
            false,
            &paths,
            &storage,
            namespace.clone(),
            Some(source_dir),
            None,
            UserMeta::Clear,
        )
        .await?;
        let keys: Vec<&PathBuf> = pkg.paths.keys().collect();
        assert_eq!(
            keys,
            vec![&PathBuf::from(".zattrs"), &PathBuf::from("data.csv")]
        );
        // The ignored folder is one item; its contents are not walked.
        let default = |pattern: &str| IgnoredBy {
            pattern: pattern.to_string(),
            source: quiltignore::RuleSource::Default,
        };
        assert_eq!(
            left_out,
            LeftOut {
                files: vec![(PathBuf::from(".DS_Store"), default(".DS_Store"))],
                dirs: vec![(PathBuf::from(".git"), default(".*/"))],
            }
        );
        assert_eq!(
            left_out.to_string(),
            "1 file and 1 folder were left out by ignore rules"
        );
        Ok(())
    }

    #[test]
    fn left_out_says_nothing_when_empty() {
        assert_eq!(LeftOut::default().to_string(), "");
        let by = || IgnoredBy {
            pattern: "x".to_string(),
            source: quiltignore::RuleSource::Default,
        };
        let left_out = LeftOut {
            files: vec![("a".into(), by()), ("b".into(), by())],
            dirs: vec![("c".into(), by()), ("d".into(), by())],
        };
        assert_eq!(
            left_out.to_string(),
            "2 files and 2 folders were left out by ignore rules"
        );
        let folder = LeftOut {
            files: Vec::new(),
            dirs: vec![("c".into(), by())],
        };
        assert_eq!(folder.to_string(), "1 folder was left out by ignore rules");
    }

    #[test(tokio::test)]
    async fn test_create_with_custom_message() -> Res {
        let (home, _temp_dir) = Home::from_temp_dir()?;
        let (paths, _temp_dir2) = DomainPaths::from_temp_dir()?;
        let storage = MockStorage::default();
        let namespace: Namespace = ("test", "msg").into();

        let (pkg, _) = create_package(
            &home,
            false,
            &paths,
            &storage,
            namespace.clone(),
            None,
            Some("My custom message".to_string()),
            UserMeta::Clear,
        )
        .await?;
        let commit = pkg.commit.as_ref().unwrap();
        let manifest_path = paths.installed_manifest(&namespace, &commit.hash);
        let manifest = Manifest::from_path(&storage, &manifest_path).await?;
        assert_eq!(manifest.header.message(), Some("My custom message"));
        assert_eq!(
            manifest.header.user_meta, None,
            "no --meta writes no user_meta key"
        );
        Ok(())
    }

    /// `create` with no `--meta` and the default message matches the top hash
    /// quilt3 8.1.0 gives an empty package with message "Created package" and
    /// no `user_meta` key.
    #[test(tokio::test)]
    async fn test_create_with_no_meta_matches_quilt3_hash() -> Res {
        const QUILT3_CREATE_NO_META: &str =
            "a8f353aa77baa68e9c15e50e26ab8894b319acd0a1e1289280aea98f391d3e8d";

        let (home, _temp_dir) = Home::from_temp_dir()?;
        let (paths, _temp_dir2) = DomainPaths::from_temp_dir()?;
        let storage = MockStorage::default();
        let namespace: Namespace = ("test", "no-meta").into();

        let (pkg, _) = create_package(
            &home,
            false,
            &paths,
            &storage,
            namespace.clone(),
            None,
            None,
            UserMeta::Clear,
        )
        .await?;
        assert_eq!(
            pkg.commit.as_ref().unwrap().hash,
            QUILT3_CREATE_NO_META,
            "create with no --meta should match quilt3 8.1.0"
        );
        Ok(())
    }

    /// `--meta` on create sets the first revision's metadata, including the
    /// exact number text, and `Keep` is treated as no key, like `Clear`.
    #[test(tokio::test)]
    async fn test_create_set_meta_keeps_exact_numbers() -> Res {
        let (home, _temp_dir) = Home::from_temp_dir()?;
        let (paths, _temp_dir2) = DomainPaths::from_temp_dir()?;
        let storage = MockStorage::default();
        let namespace: Namespace = ("test", "meta").into();
        let user_meta = UserMeta::parse(r#"{"n": 123456789012345678901234567890}"#)?;

        let (pkg, _) = create_package(
            &home,
            false,
            &paths,
            &storage,
            namespace.clone(),
            None,
            Some("with meta".to_string()),
            user_meta,
        )
        .await?;
        let manifest = Manifest::from_path(
            &storage,
            &paths.installed_manifest(&namespace, &pkg.commit.as_ref().unwrap().hash),
        )
        .await?;
        assert!(!manifest.header.user_meta_numbers.is_empty());
        assert!(
            manifest
                .to_jsonlines()
                .contains("123456789012345678901234567890")
        );

        let (home, _temp_dir) = Home::from_temp_dir()?;
        let (paths, _temp_dir2) = DomainPaths::from_temp_dir()?;
        let namespace: Namespace = ("test", "keep").into();
        let (pkg, _) = create_package(
            &home,
            false,
            &paths,
            &storage,
            namespace.clone(),
            None,
            None,
            UserMeta::Keep,
        )
        .await?;
        let manifest = Manifest::from_path(
            &storage,
            &paths.installed_manifest(&namespace, &pkg.commit.as_ref().unwrap().hash),
        )
        .await?;
        assert_eq!(manifest.header.user_meta, None);
        Ok(())
    }
}
