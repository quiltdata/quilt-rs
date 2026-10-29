use std::collections::HashMap;
use std::collections::HashSet;
use std::collections::VecDeque;
use std::path::Path;
use std::path::PathBuf;
use std::time::SystemTime;

use tracing::trace;
use tracing::warn;

use crate::Error;
use crate::Res;
use crate::checksum::calculate_hash;
use crate::checksum::hash_file;
use crate::checksum::verify_hash;
use crate::file_stamp::EntryKind;
use crate::file_stamp::FileStamp;
use crate::file_stamp::read_dir_stamps;
use crate::io::manifest::resolve_tag;
use crate::io::remote::HostConfig;
use crate::io::remote::Remote;
use crate::io::storage::Storage;
use crate::junk;
use crate::lineage::Change;
use crate::lineage::ChangeSet;
use crate::lineage::InstalledPackageStatus;
use crate::lineage::PackageLineage;
use crate::manifest::Manifest;
use crate::manifest::ManifestRow;
use crate::object_hash::ObjectHash;
use crate::paths::DomainPaths;
use crate::quiltignore;
use crate::stat_cache::CachedHash;
use crate::stat_cache::StatCache;
use quilt_uri::Tag;
use quilt_uri::UriError;

/// Refreshes the tracked `latest_hash` property in lineage.json
pub async fn refresh_latest_hash(
    mut lineage: PackageLineage,
    remote: &impl Remote,
) -> Res<PackageLineage> {
    let remote_uri = lineage.remote()?.clone();
    let origin = remote_uri.origin.clone();
    let latest = resolve_tag(remote, origin.as_ref(), remote_uri, Tag::Latest).await?;
    if lineage.latest_hash == latest.hash {
        return Ok(lineage);
    }
    lineage.latest_hash = latest.hash;
    Ok(lineage)
}

#[derive(Debug)]
enum WorkdirFile {
    Tracked(PathBuf, FileStamp, ManifestRow),
    NotTracked(PathBuf, FileStamp, ManifestRow),
    New(PathBuf, FileStamp),
    Removed(ManifestRow),
    UnSupported,
}

/// What the walk of the package home found, before it is matched with the
/// manifest.
#[derive(Default)]
pub(crate) struct Walked {
    /// (`logical_key`, `absolute_path`, kind, stamp) for everything not ignored
    /// that is not a folder.
    pub(crate) entries: Vec<(PathBuf, PathBuf, EntryKind, FileStamp)>,
    /// Files matched by an ignore rule: (`logical_key`, rule, size)
    ignored_files: Vec<(PathBuf, quiltignore::IgnoredBy, u64)>,
    /// Folders matched by an ignore rule: (path relative to the package home,
    /// rule). The walk does not enter them.
    ignored_dirs: Vec<(PathBuf, quiltignore::IgnoredBy)>,
}

/// What the package already has, so ignore rules other than the package's
/// spare it: its files, and every folder holding one.
#[derive(Default)]
pub(crate) struct Tracked {
    pub(crate) files: HashSet<PathBuf>,
    pub(crate) dirs: HashSet<PathBuf>,
}

impl Tracked {
    pub(crate) fn new<'a>(keys: impl Iterator<Item = &'a PathBuf>) -> Self {
        let mut tracked = Tracked::default();
        for key in keys {
            for dir in key.ancestors().skip(1) {
                if dir.as_os_str().is_empty() || !tracked.dirs.insert(dir.to_path_buf()) {
                    break;
                }
            }
            tracked.files.insert(key.clone());
        }
        tracked
    }
}

/// The whole walk, one folder call at a time. Blocking.
pub(crate) fn walk_package_home(
    package_home: &Path,
    rules: &quiltignore::Rules,
    tracked: &Tracked,
) -> Res<Walked> {
    let mut walked = Walked::default();
    let mut queue = VecDeque::new();
    queue.push_back(package_home.to_path_buf());

    while let Some(dir) = queue.pop_front() {
        let entries = match read_dir_stamps(&dir) {
            Ok(entries) => entries,
            // A folder that lists but can't be stamped holds files the walk
            // can't see; skipping it would report them removed.
            Err(err) if std::fs::read_dir(&dir).is_ok() => return Err(err.into()),
            Err(err) => {
                warn!("❌ Failed to read directory {}: {}", dir.display(), err);
                continue;
            }
        };
        for (name, kind, stamp) in entries {
            let file_path = dir.join(name);
            let logical_key = crate::paths::logical_key(file_path.strip_prefix(package_home)?);
            match kind {
                EntryKind::Dir => {
                    if let Some(ignored_by) =
                        rules.dir_ignored_by(&logical_key, tracked.dirs.contains(&logical_key))
                    {
                        walked.ignored_dirs.push((logical_key, ignored_by));
                        continue;
                    }
                    queue.push_back(file_path);
                }
                EntryKind::File => {
                    if let Some(ignored_by) =
                        rules.file_ignored_by(&logical_key, tracked.files.contains(&logical_key))
                    {
                        walked
                            .ignored_files
                            .push((logical_key, ignored_by, stamp.size));
                        continue;
                    }
                    walked.entries.push((logical_key, file_path, kind, stamp));
                }
                EntryKind::Other => walked.entries.push((logical_key, file_path, kind, stamp)),
            }
        }
    }
    Ok(walked)
}

fn locate_files(
    walked: Vec<(PathBuf, PathBuf, EntryKind, FileStamp)>,
    manifest: &Manifest,
    mut tracked_paths: HashMap<PathBuf, ManifestRow>,
) -> Vec<(PathBuf, WorkdirFile)> {
    // Every file the walk met is looked up here, so the rows are indexed once.
    let mut manifest_rows: HashMap<&Path, &ManifestRow> = HashMap::new();
    for row in &manifest.rows {
        manifest_rows
            .entry(row.logical_key.as_path())
            .or_insert(row);
    }
    let mut files = Vec::with_capacity(walked.len());
    for (logical_key, file_path, kind, stamp) in walked {
        if kind != EntryKind::File {
            // TODO: handle symlinks
            files.push((file_path, WorkdirFile::UnSupported));
            continue;
        }
        if let Some(row) = tracked_paths.remove(&logical_key) {
            files.push((logical_key, WorkdirFile::Tracked(file_path, stamp, row)));
        } else if let Some(row) = manifest_rows.get(logical_key.as_path()).copied() {
            files.push((
                logical_key,
                WorkdirFile::NotTracked(file_path, stamp, row.clone()),
            ));
        } else {
            files.push((logical_key, WorkdirFile::New(file_path, stamp)));
        }
    }
    for (logical_key, row) in tracked_paths {
        files.push((logical_key, WorkdirFile::Removed(row)));
    }
    files
}

/// Hashes files, through the stat cache when there is one.
struct Hasher<'a, S> {
    storage: &'a S,
    cache: Option<&'a mut StatCache>,
    /// When this walk read the stamps; what the cache saves them as.
    walked_at: SystemTime,
}

impl<S: Storage + Sync> Hasher<'_, S> {
    /// The hash of the file at `path` with `algorithm`, and its size.
    async fn hash(
        &mut self,
        logical_key: &Path,
        path: &Path,
        stamp: &FileStamp,
        algorithm: u64,
    ) -> Res<(ObjectHash, u64)> {
        if let Some(cache) = self.cache.as_deref()
            && let Some(hash) = cache.lookup(logical_key, stamp, algorithm)
        {
            return Ok((hash.clone(), stamp.size));
        }
        let (hash, size) = hash_file(self.storage, path, algorithm).await?;
        if let Some(cache) = self.cache.as_deref_mut() {
            cache.record(
                logical_key,
                CachedHash {
                    stamp: *stamp,
                    hash: hash.clone(),
                    saved_at: self.walked_at,
                },
            );
        }
        Ok((hash, size))
    }

    /// The same rows `checksum::verify_hash` gives: `None` when the file
    /// matches `row`, otherwise the row hashed with the host's algorithm.
    async fn verify(
        &mut self,
        logical_key: &Path,
        path: &Path,
        stamp: &FileStamp,
        row: ManifestRow,
        host_config: &HostConfig,
    ) -> Res<Option<ManifestRow>> {
        let (hash, size) = self
            .hash(logical_key, path, stamp, row.hash.algorithm())
            .await?;
        if hash == row.hash {
            return Ok(None);
        }
        if hash.algorithm() == host_config.checksums.algorithm_code() {
            return Ok(Some(ManifestRow { hash, size, ..row }));
        }
        self.calculate(&row.logical_key, path, stamp, host_config)
            .await
            .map(Some)
    }

    /// The same row `checksum::calculate_hash` gives.
    async fn calculate(
        &mut self,
        logical_key: &Path,
        path: &Path,
        stamp: &FileStamp,
        host_config: &HostConfig,
    ) -> Res<ManifestRow> {
        let (hash, size) = self
            .hash(
                logical_key,
                path,
                stamp,
                host_config.checksums.algorithm_code(),
            )
            .await?;
        Ok(ManifestRow {
            logical_key: logical_key.to_path_buf(),
            physical_key: format!("file://{}", path.display()),
            size,
            hash,
            ..ManifestRow::default()
        })
    }

    async fn detect_change(
        &mut self,
        logical_key: &Path,
        location: WorkdirFile,
        host_config: &HostConfig,
    ) -> Res<Option<Change>> {
        if let Some(cache) = self.cache.as_deref_mut()
            && !matches!(location, WorkdirFile::Removed(_) | WorkdirFile::UnSupported)
        {
            cache.see(logical_key);
        }
        match location {
            WorkdirFile::Tracked(path, stamp, row) | WorkdirFile::NotTracked(path, stamp, row) => {
                self.verify(logical_key, &path, &stamp, row, host_config)
                    .await
                    .map(|opt_row| opt_row.map(Change::Modified))
            }
            WorkdirFile::New(path, stamp) => self
                .calculate(logical_key, &path, &stamp, host_config)
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
}

async fn fingerprint_files(
    storage: &(impl Storage + Sync),
    files: Vec<(PathBuf, WorkdirFile)>,
    host_config: HostConfig,
    cache: Option<&mut StatCache>,
    walked_at: SystemTime,
) -> Res<ChangeSet> {
    let mut hasher = Hasher {
        storage,
        cache,
        walked_at,
    };
    let mut changes = ChangeSet::new();
    for (logical_key, location) in files {
        if let Some(change) = hasher
            .detect_change(&logical_key, location, &host_config)
            .await?
        {
            changes.insert(logical_key, change);
        }
    }
    Ok(changes)
}

/// Hash again the files `status` says were added or modified, without a stat
/// cache, so a commit names its objects by what they hold now.
pub async fn rehash_changes(
    storage: &(impl Storage + Sync),
    manifest: &Manifest,
    package_home: impl AsRef<Path>,
    mut status: InstalledPackageStatus,
    host_config: &HostConfig,
) -> Res<InstalledPackageStatus> {
    let package_home = package_home.as_ref();
    let mut changes = ChangeSet::new();
    for (logical_key, change) in std::mem::take(&mut status.changes) {
        let path = package_home.join(&logical_key);
        let change = match change {
            Change::Removed(row) => Some(Change::Removed(row)),
            Change::Added(_) => Some(Change::Added(
                calculate_hash(storage, &path, &logical_key, host_config).await?,
            )),
            Change::Modified(current) => match manifest.get_record(&logical_key) {
                Some(base) => verify_hash(storage, &path, base.clone(), host_config)
                    .await?
                    .map(Change::Modified),
                None => Some(Change::Modified(ManifestRow {
                    meta: current.meta,
                    ..calculate_hash(storage, &path, &logical_key, host_config).await?
                })),
            },
        };
        if let Some(change) = change {
            changes.insert(logical_key, change);
        }
    }
    status.changes = changes;
    Ok(status)
}

/// Creates the status of local modifications
/// It is used for `flow::commit` and for showing the status in UI.
// TODO: drop `PackageLineage` from the return type. This function does not
// mutate the input lineage; the tuple forces every caller to pattern-match
// a dead first element, and once misled someone into persisting it from
// `InstalledPackage::status()`, which raced with concurrent writers.
pub async fn create_status(
    lineage: PackageLineage,
    storage: &(impl Storage + Sync),
    paths: &DomainPaths,
    manifest: &Manifest,
    package_home: impl AsRef<Path>,
    host_config: HostConfig,
) -> Res<(PackageLineage, InstalledPackageStatus)> {
    create_status_cached(
        lineage,
        storage,
        paths,
        manifest,
        package_home,
        host_config,
        None,
    )
    .await
}

/// [`create_status`], hashing only the files whose stamp differs from what
/// `cache` saved. The changes are the same as without it, unless a file's
/// content changed with every stamp value kept, which the daily full walk
/// catches.
pub async fn create_status_cached(
    lineage: PackageLineage,
    storage: &(impl Storage + Sync),
    paths: &DomainPaths,
    manifest: &Manifest,
    package_home: impl AsRef<Path>,
    host_config: HostConfig,
    cache: Option<&mut StatCache>,
) -> Res<(PackageLineage, InstalledPackageStatus)> {
    // `trace`: this runs on a timer, several times a second across packages, and
    // the answer is almost always the same. The signal worth a debug line is "the
    // status *changed*", which the caller emits once it can tell — it holds the
    // fingerprint and the previous value, and this function holds neither.
    trace!(
        "⏳ Creating status for working directory: {}",
        package_home.as_ref().display()
    );

    // compute the status based on the following sources:
    //   - the cached manifest
    //   - paths
    //   - working directory state
    // installed entries marked as "installed" (initially as "downloading")
    // modified entries marked as "modified", etc

    trace!("⏳ Collecting paths from lineage");
    let mut orig_paths = HashMap::new();
    for path in lineage.paths.keys() {
        // `trace`, not `debug`: this fires once per *file*, and status is recomputed
        // for every installed package on every autosync tick. At debug it was 86%
        // of a real log — thirty thousand lines in ten minutes — which made the
        // file useless to read and buried everything else.
        trace!("🔍 Checking manifest for path: {}", path.display());
        match manifest.get_record(path) {
            Some(row) => {
                orig_paths.insert(path.clone(), row.clone());
            }
            None if lineage
                .remote_uri
                .as_ref()
                .is_none_or(|r| r.hash.is_empty()) =>
            {
                warn!(
                    "Lineage path {} not found in manifest, skipping (local-only package)",
                    path.display()
                );
            }
            None => {
                return Err(Error::Uri(UriError::ManifestPath(format!(
                    "path {} not found in installed manifest",
                    path.display()
                ))));
            }
        }
    }
    trace!("✔️ Found {} paths in lineage", orig_paths.len());

    let rules = quiltignore::Rules::load(package_home.as_ref(), &paths.user_quiltignore())?;
    let tracked = Tracked::new(
        manifest
            .rows
            .iter()
            .map(|row| &row.logical_key)
            .chain(orig_paths.keys()),
    );
    let home = package_home.as_ref().to_path_buf();
    let walked_at = SystemTime::now();
    let walk = tokio::task::spawn_blocking(move || walk_package_home(&home, &rules, &tracked))
        .await
        .map_err(std::io::Error::other)??;
    let files = locate_files(walk.entries, manifest, orig_paths);
    // Captured before the vector is consumed; reported once, on the outcome line.
    let walked = files.len();
    let changes = fingerprint_files(storage, files, host_config, cache, walked_at).await?;
    let ignored_files = walk.ignored_files;

    // Detect junky files among the changes
    let junky_changes: Vec<(PathBuf, String)> = changes
        .keys()
        .filter_map(|path| junk::check(path).map(|m| (path.clone(), m.pattern)))
        .collect();

    let mut status = InstalledPackageStatus::new(lineage.clone().into(), changes);
    status.ignored_files = ignored_files;
    status.ignored_dirs = walk.ignored_dirs;
    status.junky_changes = junky_changes;
    status.latest_hash.clone_from(&lineage.latest_hash);
    trace!(
        // `walked` earns its place by distinguishing two readings of "0 changes":
        // nothing changed, or we looked at nothing — a wrong or empty working
        // directory. Everything else here is the outcome.
        //
        // One line per status computation, and that is the budget. There used to be
        // a second reporting the *input*, which duplicated `ignored` and dumped the
        // whole file list; the only fact it carried alone is the count now in front.
        // Names live in the per-path `trace` above, for one package at a time.
        "✔️ Status: {} files walked, {} changes, {} ignored, {} ignored folders, {} junky",
        walked,
        status.changes.len(),
        status.ignored_files.len(),
        status.ignored_dirs.len(),
        status.junky_changes.len(),
    );
    Ok((lineage, status))
}

#[cfg(test)]
mod tests {

    use super::*;
    use test_log::test;

    use std::collections::BTreeMap;

    use aws_sdk_s3::primitives::ByteStream;

    use crate::fixtures;
    use crate::io::remote::HostChecksums;
    use crate::io::storage::mocks::MockStorage;
    use crate::lineage::CommitState;
    use crate::lineage::PathState;
    use crate::lineage::UpstreamState;
    use crate::object_hash::Crc64Hash;
    use crate::object_hash::Sha256ChunkedHash;
    use crate::quiltignore::RuleSource;
    use quilt_uri::ManifestUri;

    /// Domain paths inside the storage's temp dir, so no test reads a real user-wide rules file.
    fn domain_paths(storage: &MockStorage) -> DomainPaths {
        DomainPaths::new(storage.temp_dir.path().join("domain"))
    }

    /// Helper to create a `PackageLineage` with a dummy remote (avoids Local state).
    /// Uses a non-empty hash so the lineage isn't treated as "never pushed".
    fn lineage_with_remote(mut lineage: PackageLineage) -> PackageLineage {
        let dummy_hash = "deadbeef".to_string();
        if lineage.remote_uri.is_none() {
            lineage.remote_uri = Some(ManifestUri {
                hash: dummy_hash.clone(),
                bucket: "test-bucket".to_string(),
                origin: Some(quilt_uri::fixtures::host()),
                ..ManifestUri::default()
            });
        }
        if lineage.base_hash.is_empty() {
            lineage.base_hash = dummy_hash.clone();
        }
        if lineage.latest_hash.is_empty() {
            lineage.latest_hash = dummy_hash;
        }
        lineage
    }

    #[test(tokio::test)]
    async fn test_default_status() -> Res {
        let storage = MockStorage::default();
        let (_lineage, status) = create_status(
            lineage_with_remote(PackageLineage::default()),
            &storage,
            &domain_paths(&storage),
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
        let lineage = lineage_with_remote(PackageLineage {
            commit: Some(CommitState {
                hash: "AAA".to_string(),
                ..CommitState::default()
            }),
            base_hash: "AAA".to_string(),
            latest_hash: "BBB".to_string(),
            ..PackageLineage::default()
        });

        let storage = MockStorage::default();

        let (_lineage, status) = create_status(
            lineage,
            &storage,
            &domain_paths(&storage),
            &Manifest::default(),
            PathBuf::default(),
            HostConfig::default(),
        )
        .await?;
        assert_eq!(status.upstream_state, UpstreamState::Behind);
        assert_eq!(
            status.latest_hash, "BBB",
            "the status names the newer revision it was computed against"
        );
        Ok(())
    }

    #[test(tokio::test)]
    async fn test_ahead() -> Res {
        let lineage = lineage_with_remote(PackageLineage {
            commit: Some(CommitState {
                hash: "BBB".to_string(),
                ..CommitState::default()
            }),
            base_hash: "AAA".to_string(),
            latest_hash: "AAA".to_string(),
            ..PackageLineage::default()
        });

        let storage = MockStorage::default();

        let (_, status) = create_status(
            lineage,
            &storage,
            &domain_paths(&storage),
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
        let lineage = lineage_with_remote(PackageLineage {
            commit: Some(CommitState {
                hash: "aaa".to_string(),
                ..CommitState::default()
            }),
            base_hash: "bbb".to_string(),
            latest_hash: "ccc".to_string(),
            ..PackageLineage::default()
        });

        let storage = MockStorage::default();

        let (_, status) = create_status(
            lineage,
            &storage,
            &domain_paths(&storage),
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
            &domain_paths(&storage),
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
            &domain_paths(&storage),
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
            &domain_paths(&storage),
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
            assert!(added_row.matches_content(&reference_row));
            Ok(())
        } else {
            panic!("Expected Change::Added, got {added_file:?}")
        }
    }

    /// A logical key separates folders with `/` on every OS, though the walk
    /// finds `\\` on Windows. Compared as text: Windows paths treat both
    /// separators as one, so a `PathBuf` comparison would pass either way.
    #[test(tokio::test)]
    async fn test_added_file_in_a_folder_has_a_slash_key() -> Res {
        let storage = MockStorage::default();
        let working_dir = storage.temp_dir.as_ref().join("home");
        storage
            .write_byte_stream(
                working_dir.join("data").join("raw").join("a.csv"),
                ByteStream::from_static(b"a,b\n"),
            )
            .await?;

        let (_, status) = create_status(
            PackageLineage::default(),
            &storage,
            &domain_paths(&storage),
            &Manifest::default(),
            working_dir,
            HostConfig::default(),
        )
        .await?;

        let keys: Vec<&str> = status
            .changes
            .keys()
            .map(|key| key.to_str().expect("UTF-8"))
            .collect();
        assert_eq!(keys, ["data/raw/a.csv"]);
        let Some(Change::Added(row)) = status.changes.values().next() else {
            panic!("expected one added file, got {:?}", status.changes);
        };
        assert_eq!(row.logical_key.to_str(), Some("data/raw/a.csv"));
        Ok(())
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

        let (_, status) = create_status(
            lineage,
            &storage,
            &domain_paths(&storage),
            &manifest,
            working_dir,
            host_config,
        )
        .await?;

        let added_file = status.changes.get(&file_path).unwrap();
        if let Change::Added(added_row) = added_file {
            let reference_row = ManifestRow {
                logical_key: PathBuf::from("some.pq"),
                size: 16,
                hash: Crc64Hash::try_from("CRSFynAYcw4=")?.into(),
                ..ManifestRow::default()
            };
            assert!(added_row.matches_content(&reference_row));
            Ok(())
        } else {
            panic!("Expected Change::Added, got {added_file:?}")
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
            &domain_paths(&storage),
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
            &domain_paths(&storage),
            &Manifest::default(),
            &working_dir,
            HostConfig::default(),
        )
        .await?;

        assert!(status.changes.contains_key(&PathBuf::from("keep.txt")));
        assert!(
            !status
                .changes
                .contains_key(&PathBuf::from("cache/file.txt"))
        );
        Ok(())
    }

    #[test(tokio::test)]
    async fn test_quiltignore_directory_recorded_once() -> Res {
        let storage = MockStorage::default();
        let working_dir = storage.temp_dir.as_ref().join("pkg");

        for path in ["cache/file.txt", "cache/nested/deep.txt", "keep.txt"] {
            storage
                .write_byte_stream(working_dir.join(path), ByteStream::from_static(b"x"))
                .await?;
        }

        std::fs::write(working_dir.join(".quiltignore"), "cache/\nnested/\n").unwrap();

        let (_, status) = create_status(
            PackageLineage::default(),
            &storage,
            &domain_paths(&storage),
            &Manifest::default(),
            &working_dir,
            HostConfig::default(),
        )
        .await?;

        // `cache/nested` matches a rule too, but the walk never enters `cache`.
        assert_eq!(
            status.ignored_dirs,
            vec![(
                PathBuf::from("cache"),
                quiltignore::IgnoredBy {
                    pattern: "cache/".to_string(),
                    source: RuleSource::Package { line: 1 },
                }
            )]
        );
        assert_eq!(status.ignored_files, Vec::new());
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
            &domain_paths(&storage),
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
            &domain_paths(&storage),
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
            &domain_paths(&storage),
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
            &domain_paths(&storage),
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

    /// [`LocalStorage`] that counts the files it opens.
    #[derive(Default)]
    struct CountingStorage {
        inner: crate::io::storage::LocalStorage,
        opens: std::sync::atomic::AtomicUsize,
    }

    impl CountingStorage {
        fn opens(&self) -> usize {
            self.opens.swap(0, std::sync::atomic::Ordering::SeqCst)
        }
    }

    impl Storage for CountingStorage {
        async fn copy(
            &self,
            from: impl AsRef<Path> + Send,
            to: impl AsRef<Path> + Send,
        ) -> Res<u64> {
            self.inner.copy(from, to).await
        }
        async fn create_dir_all(&self, path: impl AsRef<Path> + Send) -> Res {
            self.inner.create_dir_all(path).await
        }
        async fn create_file(&self, path: impl AsRef<Path>) -> Res<tokio::fs::File> {
            self.inner.create_file(path).await
        }
        async fn exists(&self, path: impl AsRef<Path>) -> bool {
            self.inner.exists(path).await
        }
        async fn is_file(&self, path: impl AsRef<Path>) -> bool {
            self.inner.is_file(path).await
        }
        async fn modified_timestamp(
            &self,
            path: impl AsRef<Path>,
        ) -> Res<chrono::DateTime<chrono::Utc>> {
            self.inner.modified_timestamp(path).await
        }
        async fn open_file(&self, path: impl AsRef<Path> + Send) -> Res<tokio::fs::File> {
            self.opens.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.inner.open_file(path).await
        }
        async fn read_byte_stream(&self, path: impl AsRef<Path> + Send + Sync) -> Res<ByteStream> {
            self.inner.read_byte_stream(path).await
        }
        async fn read_dir(&self, path: impl AsRef<Path> + Send + Sync) -> Res<tokio::fs::ReadDir> {
            self.inner.read_dir(path).await
        }
        async fn remove_dir_all(&self, path: impl AsRef<Path> + Send) -> Res {
            self.inner.remove_dir_all(path).await
        }
        async fn remove_file(&self, path: impl AsRef<Path> + Send) -> Result<(), std::io::Error> {
            self.inner.remove_file(path).await
        }
        async fn rename(&self, from: impl AsRef<Path> + Send, to: impl AsRef<Path> + Send) -> Res {
            self.inner.rename(from, to).await
        }
        async fn write_byte_stream(
            &self,
            path: impl AsRef<Path> + Send + Sync,
            body: ByteStream,
        ) -> Res {
            self.inner.write_byte_stream(path, body).await
        }
        async fn create_private_dir(&self, path: impl AsRef<Path> + Send) -> Res {
            self.inner.create_private_dir(path).await
        }
        async fn write_private_byte_stream(
            &self,
            path: impl AsRef<Path> + Send + Sync,
            body: ByteStream,
        ) -> Res {
            self.inner.write_private_byte_stream(path, body).await
        }
        async fn lock_exclusive(
            &self,
            path: impl AsRef<Path> + Send,
        ) -> Res<crate::io::storage::LockGuard> {
            self.inner.lock_exclusive(path).await
        }
        async fn try_lock_exclusive(
            &self,
            path: impl AsRef<Path> + Send,
        ) -> Res<Option<crate::io::storage::LockGuard>> {
            self.inner.try_lock_exclusive(path).await
        }
    }

    /// A package home with a committed file `tracked.txt` and a new file
    /// `new.txt`, both edited an hour ago so no entry is racy.
    struct CachedPackage {
        home: tempfile::TempDir,
        storage: CountingStorage,
        store: crate::state_store::PackageStateStore,
        lineage: PackageLineage,
        manifest: Manifest,
    }

    const AN_HOUR: std::time::Duration = std::time::Duration::from_secs(3600);

    fn set_modified(path: &Path, at: SystemTime) {
        std::fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(at)
            .unwrap();
    }

    impl CachedPackage {
        async fn new() -> Result<Self, Box<dyn std::error::Error>> {
            let home = tempfile::TempDir::new()?;
            std::fs::write(home.path().join("tracked.txt"), b"one")?;
            std::fs::write(home.path().join("new.txt"), b"new")?;
            let an_hour_ago = SystemTime::now() - AN_HOUR;
            set_modified(&home.path().join("tracked.txt"), an_hour_ago);
            set_modified(&home.path().join("new.txt"), an_hour_ago);
            let storage = CountingStorage::default();
            let row = calculate_hash(
                &storage,
                &home.path().join("tracked.txt"),
                Path::new("tracked.txt"),
                &HostConfig::default(),
            )
            .await?;
            let mut manifest = Manifest::default();
            manifest.insert_record(row.clone()).await?;
            let lineage = PackageLineage {
                paths: BTreeMap::from([(
                    PathBuf::from("tracked.txt"),
                    PathState {
                        hash: row.hash.clone().into(),
                        ..PathState::default()
                    },
                )]),
                ..PackageLineage::default()
            };
            storage.opens();
            Ok(Self {
                home,
                storage,
                store: crate::state_store::PackageStateStore::in_memory(),
                lineage,
                manifest,
            })
        }

        fn path(&self, name: &str) -> PathBuf {
            self.home.path().join(name)
        }

        /// A domain root with no user-wide rules file.
        fn domain_paths(&self) -> DomainPaths {
            DomainPaths::new(self.home.path().join("no-domain"))
        }

        async fn walk_with(&self, host_config: HostConfig) -> Res<ChangeSet> {
            let namespace = ("acme", "demo").into();
            let mut cache = self.store.stat_cache(&namespace).await;
            let (_, status) = create_status_cached(
                self.lineage.clone(),
                &self.storage,
                &self.domain_paths(),
                &self.manifest,
                self.home.path(),
                host_config,
                Some(&mut cache),
            )
            .await?;
            self.store.save_stat_cache(&namespace, &cache).await;
            Ok(status.changes)
        }

        async fn walk(&self) -> Res<ChangeSet> {
            self.walk_with(HostConfig::default()).await
        }

        async fn walk_without_cache(&self) -> Res<ChangeSet> {
            let (_, status) = create_status(
                self.lineage.clone(),
                &self.storage,
                &self.domain_paths(),
                &self.manifest,
                self.home.path(),
                HostConfig::default(),
            )
            .await?;
            Ok(status.changes)
        }
    }

    #[test(tokio::test)]
    async fn an_unchanged_package_walk_reads_no_file_contents()
    -> Result<(), Box<dyn std::error::Error>> {
        let package = CachedPackage::new().await?;
        let first = package.walk().await?;
        assert_eq!(package.storage.opens(), 2, "the first walk hashes both");
        let second = package.walk().await?;
        assert_eq!(package.storage.opens(), 0);
        assert_eq!(first, second);
        assert_eq!(second, package.walk_without_cache().await?);
        Ok(())
    }

    #[test(tokio::test)]
    async fn a_missing_cache_gives_the_same_changes() -> Result<(), Box<dyn std::error::Error>> {
        let package = CachedPackage::new().await?;
        std::fs::write(package.path("tracked.txt"), b"two")?;
        let cached = package.walk().await?;
        let uncached = package.walk_without_cache().await?;
        assert_eq!(cached, uncached);
        assert!(matches!(
            cached.get(Path::new("tracked.txt")),
            Some(Change::Modified(_))
        ));
        assert!(matches!(
            cached.get(Path::new("new.txt")),
            Some(Change::Added(_))
        ));
        Ok(())
    }

    /// `touch -r` and `cp -p` over a file: new content, same size, the old
    /// edit time put back.
    #[test(tokio::test)]
    async fn new_content_with_the_edit_time_restored_is_seen()
    -> Result<(), Box<dyn std::error::Error>> {
        let package = CachedPackage::new().await?;
        assert!(!package.walk().await?.contains_key(Path::new("tracked.txt")));
        let path = package.path("tracked.txt");
        let old = std::fs::metadata(&path)?.modified()?;
        // A change time is never older than the one before it, but it can be
        // the same tick on a coarse clock.
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&path, b"two")?;
        set_modified(&path, old);
        assert_eq!(std::fs::metadata(&path)?.modified()?, old);

        let changes = package.walk().await?;
        assert!(
            matches!(
                changes.get(Path::new("tracked.txt")),
                Some(Change::Modified(_))
            ),
            "{changes:?}"
        );
        assert_eq!(changes, package.walk_without_cache().await?);
        Ok(())
    }

    /// `rsync -a`: new content written beside the file, its edit time set to
    /// the old one, then renamed over it.
    #[test(tokio::test)]
    async fn a_file_replaced_with_the_edit_time_restored_is_seen()
    -> Result<(), Box<dyn std::error::Error>> {
        let package = CachedPackage::new().await?;
        package.walk().await?;
        let path = package.path("new.txt");
        let old = std::fs::metadata(&path)?.modified()?;
        let beside = package.path(".new.txt.tmp");
        std::fs::write(&beside, b"NEW")?;
        set_modified(&beside, old);
        std::fs::rename(&beside, &path)?;

        let changes = package.walk().await?;
        assert_eq!(changes, package.walk_without_cache().await?);
        let Some(Change::Added(row)) = changes.get(Path::new("new.txt")) else {
            panic!("{changes:?}");
        };
        let (fresh, _) = hash_file(&package.storage, &path, row.hash.algorithm()).await?;
        assert_eq!(row.hash, fresh);
        Ok(())
    }

    #[test(tokio::test)]
    async fn a_file_edited_in_the_second_it_was_walked_is_hashed_again()
    -> Result<(), Box<dyn std::error::Error>> {
        let package = CachedPackage::new().await?;
        set_modified(&package.path("new.txt"), SystemTime::now() + AN_HOUR);
        package.walk().await?;
        package.storage.opens();
        package.walk().await?;
        assert_eq!(package.storage.opens(), 1, "only the racy file");
        Ok(())
    }

    #[test(tokio::test)]
    async fn another_host_algorithm_hashes_again() -> Result<(), Box<dyn std::error::Error>> {
        let package = CachedPackage::new().await?;
        package.walk().await?;
        package.storage.opens();
        let crc64 = HostConfig {
            checksums: HostChecksums::Crc64,
            host: None,
        };
        let changes = package.walk_with(crc64).await?;
        assert_eq!(package.storage.opens(), 1, "the new file, for its row");
        let Some(Change::Added(row)) = changes.get(Path::new("new.txt")) else {
            panic!("{changes:?}");
        };
        assert_eq!(
            row.hash.algorithm(),
            crate::object_hash::MULTIHASH_CRC64_NVME
        );
        Ok(())
    }

    /// A folder that lists but can't be searched: its files are there, so the
    /// walk fails rather than call them removed.
    #[cfg(unix)]
    #[test(tokio::test)]
    async fn a_folder_that_lists_but_cant_be_stamped_is_an_error()
    -> Result<(), Box<dyn std::error::Error>> {
        use std::os::unix::fs::PermissionsExt;
        let package = CachedPackage::new().await?;
        let sub = package.path("sub");
        std::fs::create_dir(&sub)?;
        std::fs::write(sub.join("inside.txt"), b"x")?;
        std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o444))?;
        let walked = package.walk_without_cache().await;
        std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o755))?;
        assert!(walked.is_err(), "{walked:?}");
        Ok(())
    }

    #[test(tokio::test)]
    async fn rehash_changes_corrects_a_stale_hash() -> Result<(), Box<dyn std::error::Error>> {
        let package = CachedPackage::new().await?;
        let (_, mut status) = create_status(
            package.lineage.clone(),
            &package.storage,
            &package.domain_paths(),
            &package.manifest,
            package.home.path(),
            HostConfig::default(),
        )
        .await?;
        std::fs::write(package.path("new.txt"), b"changed after the walk")?;
        let stale = status.changes.clone();
        status = rehash_changes(
            &package.storage,
            &package.manifest,
            package.home.path(),
            status,
            &HostConfig::default(),
        )
        .await?;
        assert_ne!(stale, status.changes);
        assert_eq!(status.changes, package.walk_without_cache().await?);
        Ok(())
    }

    #[test(tokio::test)]
    async fn test_local_status() -> Res {
        let storage = MockStorage::default();
        let (lineage, status) = create_status(
            PackageLineage::default(),
            &storage,
            &domain_paths(&storage),
            &Manifest::default(),
            PathBuf::default(),
            HostConfig::default(),
        )
        .await?;
        assert_eq!(status.upstream_state, UpstreamState::Local);
        assert!(status.changes.is_empty());
        assert!(lineage.remote_uri.is_none());
        Ok(())
    }

    async fn write_files(storage: &MockStorage, dir: &Path, names: &[&str]) -> Res {
        for name in names {
            storage
                .write_byte_stream(dir.join(name), ByteStream::from_static(b"x"))
                .await?;
        }
        Ok(())
    }

    /// A lineage and manifest that track the given files exactly as they are on disk.
    async fn track_files(
        storage: &MockStorage,
        dir: &Path,
        names: &[&str],
    ) -> Res<(PackageLineage, Manifest)> {
        let mut lineage = PackageLineage::default();
        let mut manifest = Manifest::default();
        for name in names {
            let row = calculate_hash(
                storage,
                &dir.join(name),
                Path::new(name),
                &HostConfig::default(),
            )
            .await?;
            lineage.paths.insert(
                PathBuf::from(name),
                PathState {
                    hash: row.hash.clone().into(),
                    ..PathState::default()
                },
            );
            manifest.rows.push(row);
        }
        Ok((lineage, manifest))
    }

    fn ignored_source(status: &InstalledPackageStatus, path: &str) -> Option<RuleSource> {
        status
            .ignored_files
            .iter()
            .find(|(key, _, _)| key == Path::new(path))
            .map(|(_, ignored_by, _)| ignored_by.source.clone())
    }

    #[test(tokio::test)]
    async fn test_defaults_ignore_new_clutter_but_keep_zarr_metadata() -> Res {
        let storage = MockStorage::default();
        let working_dir = storage.temp_dir.as_ref().join("pkg");
        write_files(
            &storage,
            &working_dir,
            &[
                ".DS_Store",
                ".env",
                ".git/config",
                ".venv/bin/python",
                ".zattrs",
                ".zarray",
                ".zgroup",
                "img.zarr/0/.zarray",
                "data.csv",
            ],
        )
        .await?;

        let (_, status) = create_status(
            PackageLineage::default(),
            &storage,
            &domain_paths(&storage),
            &Manifest::default(),
            &working_dir,
            HostConfig::default(),
        )
        .await?;

        let changes: Vec<&Path> = status.changes.keys().map(PathBuf::as_path).collect();
        assert_eq!(
            changes,
            [
                ".zarray",
                ".zattrs",
                ".zgroup",
                "data.csv",
                "img.zarr/0/.zarray"
            ]
            .map(Path::new)
            .to_vec()
        );
        assert_eq!(
            ignored_source(&status, ".DS_Store"),
            Some(RuleSource::Default)
        );
        assert_eq!(ignored_source(&status, ".env"), Some(RuleSource::Default));
        Ok(())
    }

    #[test(tokio::test)]
    async fn test_defaults_leave_tracked_files_alone() -> Res {
        let storage = MockStorage::default();
        let working_dir = storage.temp_dir.as_ref().join("pkg");
        write_files(&storage, &working_dir, &[".DS_Store", ".git/config"]).await?;
        let (lineage, manifest) =
            track_files(&storage, &working_dir, &[".DS_Store", ".git/config"]).await?;
        write_files(&storage, &working_dir, &[".git/HEAD"]).await?;

        let (_, status) = create_status(
            lineage,
            &storage,
            &domain_paths(&storage),
            &manifest,
            &working_dir,
            HostConfig::default(),
        )
        .await?;

        assert!(status.changes.is_empty(), "{:?}", status.changes);
        assert_eq!(ignored_source(&status, ".DS_Store"), None);
        assert_eq!(ignored_source(&status, ".git/config"), None);
        assert_eq!(
            ignored_source(&status, ".git/HEAD"),
            Some(RuleSource::Default)
        );
        Ok(())
    }

    #[test(tokio::test)]
    async fn test_user_rules_apply_to_new_files_only() -> Res {
        let storage = MockStorage::default();
        let paths = domain_paths(&storage);
        let working_dir = storage.temp_dir.as_ref().join("pkg");
        write_files(&storage, &working_dir, &["kept.tmp", "new.tmp", ".env"]).await?;
        let (lineage, manifest) = track_files(&storage, &working_dir, &["kept.tmp"]).await?;
        std::fs::create_dir_all(paths.user_quiltignore().parent().unwrap())?;
        std::fs::write(paths.user_quiltignore(), "# mine\n*.tmp\n!.env\n")?;

        let (_, status) = create_status(
            lineage,
            &storage,
            &paths,
            &manifest,
            &working_dir,
            HostConfig::default(),
        )
        .await?;

        assert_eq!(
            ignored_source(&status, "new.tmp"),
            Some(RuleSource::User {
                path: paths.user_quiltignore(),
                line: 2,
            })
        );
        assert_eq!(ignored_source(&status, "kept.tmp"), None);
        assert!(!status.changes.contains_key(Path::new("kept.tmp")));
        assert!(matches!(
            status.changes.get(Path::new(".env")),
            Some(Change::Added(_))
        ));
        Ok(())
    }

    #[test(tokio::test)]
    async fn test_package_rule_wins_over_user_rule() -> Res {
        let storage = MockStorage::default();
        let paths = domain_paths(&storage);
        let working_dir = storage.temp_dir.as_ref().join("pkg");
        write_files(&storage, &working_dir, &["a.tmp"]).await?;
        std::fs::write(working_dir.join(".quiltignore"), "*.log\na.tmp\n")?;
        std::fs::create_dir_all(paths.user_quiltignore().parent().unwrap())?;
        std::fs::write(paths.user_quiltignore(), "*.tmp\n")?;

        let (_, status) = create_status(
            PackageLineage::default(),
            &storage,
            &paths,
            &Manifest::default(),
            &working_dir,
            HostConfig::default(),
        )
        .await?;

        let (_, ignored_by, _) = &status.ignored_files[0];
        assert_eq!(ignored_by.pattern, "a.tmp");
        assert_eq!(ignored_by.source, RuleSource::Package { line: 2 });
        assert!(status.changes.contains_key(Path::new(".quiltignore")));
        Ok(())
    }

    #[test(tokio::test)]
    async fn test_folder_brought_back_keeps_its_clutter_out() -> Res {
        let storage = MockStorage::default();
        let working_dir = storage.temp_dir.as_ref().join("pkg");
        write_files(
            &storage,
            &working_dir,
            &[".snakemake/x/log.txt", ".snakemake/x/.DS_Store"],
        )
        .await?;
        std::fs::write(working_dir.join(".quiltignore"), "!/.snakemake/\n")?;

        let (_, status) = create_status(
            PackageLineage::default(),
            &storage,
            &domain_paths(&storage),
            &Manifest::default(),
            &working_dir,
            HostConfig::default(),
        )
        .await?;

        assert!(
            status
                .changes
                .contains_key(Path::new(".snakemake/x/log.txt"))
        );
        assert_eq!(
            ignored_source(&status, ".snakemake/x/.DS_Store"),
            Some(RuleSource::Default)
        );
        Ok(())
    }
}
