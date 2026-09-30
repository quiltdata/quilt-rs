//! Reclaim what `.quilt/` holds for nothing: objects no installed manifest
//! uses, the remote-manifest cache, and staging dirs a killed write left.
//!
//! There is no reference count. An object is proven dead by reading every
//! installed manifest, of every package, while holding every package's lock:
//! an object none of them names is one no package can reach.

use std::collections::BTreeSet;
use std::path::Path;
use std::path::PathBuf;

use tracing::debug;
use tracing::info;

use crate::Error;
use crate::Res;
use crate::io::storage::LockGuard;
use crate::io::storage::Storage;
use crate::lineage::DomainLineage;
use crate::manifest::Manifest;
use crate::package_lock;
use crate::paths::DomainPaths;
use quilt_uri::Namespace;

/// What a [`gc`] removed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GcReport {
    /// Files deleted from `objects/`.
    pub objects: usize,
    /// Manifests deleted from the `packages/` cache.
    pub cached_manifests: usize,
    /// Entries removed from `staging/`.
    pub staging: usize,
    /// The deleted files' lengths, summed. On a filesystem that clones, an
    /// object sharing blocks with a working file frees less than this.
    pub bytes: u64,
}

impl GcReport {
    /// Whether nothing was removed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.objects == 0 && self.cached_manifests == 0 && self.staging == 0
    }
}

/// Delete every object no installed manifest uses, everything in the
/// `packages/` cache and everything in `staging/`.
///
/// Try-locks every package first, and deletes nothing if one is busy:
/// [`Error::PackageBusy`] names it. The locks are held to the end, so no
/// writer can stage a file or file an object meanwhile.
pub async fn gc(
    paths: &DomainPaths,
    storage: &(impl Storage + Sync),
    lineage: &DomainLineage,
) -> Res<GcReport> {
    info!(
        "⏳ Collecting garbage in {}",
        paths.dot_quilt_dir().display()
    );
    let _held = lock_every_package(paths, storage, lineage).await?;

    let mut report = GcReport::default();

    let objects = list_files(storage, &paths.objects_dir()).await?;
    let candidates = objects.iter().map(|(name, _, _)| name.clone()).collect();
    let in_use = objects_in_use(paths, storage, candidates).await?;
    for (name, path, len) in objects {
        if in_use.contains(&name) {
            continue;
        }
        debug!("🗑️ Removing unused object {name}");
        storage.remove_file(&path).await?;
        report.objects += 1;
        report.bytes += len;
    }

    for bucket in list_entries(storage, &paths.cached_manifests_root()).await? {
        let (files, bytes) = remove_entry(storage, bucket).await?;
        report.cached_manifests += files;
        report.bytes += bytes;
    }

    for entry in list_entries(storage, &paths.staging_dir()).await? {
        let (_, bytes) = remove_entry(storage, entry).await?;
        report.staging += 1;
        report.bytes += bytes;
    }

    info!("✔️ Collected garbage: {report:?}");
    Ok(report)
}

/// Of `candidates` — object file names — those some installed manifest
/// uses: any revision of any package under `installed/`.
///
/// A row uses an object by its hash (the name is the digest's hex) and, for
/// a `file://` physical key into an `objects/` directory, by that file's
/// name. The two usually agree; a row carried over by a recommit can keep
/// its old key under a new hash, so both count.
///
/// Stops reading once every candidate is found. Fails on a manifest it
/// cannot read, rather than treat its objects as unused.
pub(crate) async fn objects_in_use(
    paths: &DomainPaths,
    storage: &(impl Storage + Sync),
    mut candidates: BTreeSet<String>,
) -> Res<BTreeSet<String>> {
    let mut in_use = BTreeSet::new();
    for owner in list_entries(storage, &paths.installed_dir()).await? {
        for package in list_entries(storage, &owner.path).await? {
            for (_, manifest_path, _) in list_files(storage, &package.path).await? {
                if candidates.is_empty() {
                    return Ok(in_use);
                }
                let manifest = Manifest::from_path(storage, &manifest_path).await?;
                for row in &manifest.rows {
                    for name in row_objects(&row.hash, &row.physical_key) {
                        if let Some(name) = candidates.take(&name) {
                            in_use.insert(name);
                        }
                    }
                }
            }
        }
    }
    Ok(in_use)
}

/// The object names one row uses: its digest's hex, and the file name of a
/// `file://` key into an `objects/` directory.
fn row_objects(
    hash: &crate::object_hash::ObjectHash,
    physical_key: &str,
) -> impl Iterator<Item = String> {
    let by_key = url::Url::parse(physical_key)
        .ok()
        .filter(|url| url.scheme() == "file")
        .and_then(|url| url.to_file_path().ok())
        .filter(|path| {
            path.parent()
                .and_then(Path::file_name)
                .is_some_and(|dir| dir == crate::paths::OBJECTS_DIR)
        })
        .and_then(|path| Some(path.file_name()?.to_string_lossy().into_owned()));
    std::iter::once(hex::encode(hash.digest())).chain(by_key)
}

/// Try-lock every package in the lineage and every package with a lock
/// file. An install takes its lock before its lineage entry exists, so the
/// files catch one in flight.
async fn lock_every_package(
    paths: &DomainPaths,
    storage: &(impl Storage + Sync),
    lineage: &DomainLineage,
) -> Res<Vec<LockGuard>> {
    let mut namespaces: BTreeSet<Namespace> = lineage.namespaces().into_iter().collect();
    for owner_dir in list_entries(storage, &paths.locks_dir()).await? {
        let Some(owner) = owner_dir.path.file_name() else {
            continue;
        };
        let owner = owner.to_string_lossy();
        for (lock_file, _, _) in list_files(storage, &owner_dir.path).await? {
            if let Some(name) = lock_file.strip_suffix(".lock") {
                namespaces.insert((owner.as_ref(), name).into());
            }
        }
    }

    let mut held = Vec::with_capacity(namespaces.len());
    for namespace in namespaces {
        match package_lock::try_lock(storage, paths, &namespace).await? {
            Some(guard) => held.push(guard),
            None => return Err(Error::PackageBusy(namespace)),
        }
    }
    Ok(held)
}

/// One entry of a directory, as it is itself: a symlink is not followed.
struct Entry {
    path: PathBuf,
    is_dir: bool,
    len: u64,
}

/// The entries of `dir`, or none if it does not exist.
async fn list_entries(storage: &(impl Storage + Sync), dir: &Path) -> Res<Vec<Entry>> {
    if !storage.exists(dir).await {
        return Ok(Vec::new());
    }
    let mut entries = storage.read_dir(dir).await?;
    let mut found = Vec::new();
    while let Some(entry) = entries.next_entry().await? {
        let metadata = entry.metadata().await?;
        found.push(Entry {
            path: entry.path(),
            is_dir: metadata.is_dir(),
            len: metadata.len(),
        });
    }
    found.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(found)
}

/// The visible files of `dir` as `(name, path, length)`, or none if it does
/// not exist. A hidden file is a write's stranded `.tmp-<uuid>` or the OS's
/// own, and is left alone.
async fn list_files(
    storage: &(impl Storage + Sync),
    dir: &Path,
) -> Res<Vec<(String, PathBuf, u64)>> {
    if !storage.exists(dir).await {
        return Ok(Vec::new());
    }
    let mut entries = storage.read_dir(dir).await?;
    let mut files = Vec::new();
    while let Some(entry) = entries.next_entry().await? {
        let name = entry.file_name().to_string_lossy().into_owned();
        let metadata = entry.metadata().await?;
        if name.starts_with('.') || !metadata.is_file() {
            continue;
        }
        files.push((name, entry.path(), metadata.len()));
    }
    files.sort();
    Ok(files)
}

/// Remove `entry`, returning how many files it held and their summed
/// length; a file counts as one.
async fn remove_entry(storage: &(impl Storage + Sync), entry: Entry) -> Res<(usize, u64)> {
    if !entry.is_dir {
        storage.remove_file(&entry.path).await?;
        return Ok((1, entry.len));
    }
    let (mut files, mut bytes) = (0, 0);
    let mut dirs = vec![entry.path.clone()];
    while let Some(dir) = dirs.pop() {
        let mut entries = storage.read_dir(&dir).await?;
        while let Some(child) = entries.next_entry().await? {
            let metadata = child.metadata().await?;
            if metadata.is_dir() {
                dirs.push(child.path());
            } else {
                files += 1;
                bytes += metadata.len();
            }
        }
    }
    storage.remove_dir_all(&entry.path).await?;
    Ok((files, bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    use test_log::test;

    use crate::LocalDomain;
    use crate::io::storage::LocalStorage;
    use crate::manifest::ManifestRow;

    /// A domain in a temp dir whose home is also there.
    async fn domain() -> Res<(LocalDomain, DomainPaths, tempfile::TempDir)> {
        let dir = tempfile::TempDir::new()?;
        let domain = LocalDomain::new(dir.path());
        domain.set_home(dir.path().join("home")).await?;
        Ok((domain, DomainPaths::new(dir.path().to_path_buf()), dir))
    }

    /// Create `namespace` as a local package holding `files`.
    async fn create(domain: &LocalDomain, namespace: &str, files: &[(&str, &str)]) -> Res {
        let source = tempfile::TempDir::new()?;
        for (name, content) in files {
            std::fs::write(source.path().join(name), content)?;
        }
        let namespace: Namespace = namespace.try_into()?;
        domain
            .create_package(namespace, Some(source.path().to_path_buf()), None)
            .await?;
        Ok(())
    }

    fn object_names(paths: &DomainPaths) -> Res<BTreeSet<String>> {
        Ok(std::fs::read_dir(paths.objects_dir())?
            .map(|entry| Ok(entry?.file_name().to_string_lossy().into_owned()))
            .collect::<Res<_>>()?)
    }

    /// Uninstalling a package orphans its own objects; gc deletes those and
    /// keeps the one another package still uses.
    #[test(tokio::test)]
    async fn deletes_only_the_objects_no_installed_manifest_uses() -> Res {
        let (domain, paths, _dir) = domain().await?;
        create(
            &domain,
            "acme/kept",
            &[("shared.txt", "shared"), ("a.txt", "a")],
        )
        .await?;
        let kept = object_names(&paths)?;
        create(
            &domain,
            "acme/gone",
            &[("shared.txt", "shared"), ("b.txt", "bb")],
        )
        .await?;
        assert_eq!(object_names(&paths)?.len(), 3);
        domain.uninstall_package("acme/gone".try_into()?).await?;

        let report = domain.gc().await?;

        assert_eq!(object_names(&paths)?, kept);
        assert_eq!(
            report,
            GcReport {
                objects: 1,
                bytes: 2,
                ..GcReport::default()
            }
        );
        assert!(domain.gc().await?.is_empty(), "a second run frees nothing");
        Ok(())
    }

    /// The whole manifest cache and every staging entry go, counted.
    #[test(tokio::test)]
    async fn empties_the_manifest_cache_and_staging() -> Res {
        let (domain, paths, _dir) = domain().await?;
        let bucket = paths.cached_manifests_dir("bucket");
        std::fs::create_dir_all(&bucket)?;
        std::fs::write(bucket.join("1111"), "12345")?;
        std::fs::write(bucket.join("2222"), "123")?;
        let stranded = paths.staging_dir().join("some-uuid");
        std::fs::create_dir_all(&stranded)?;
        std::fs::write(stranded.join("file"), "1")?;

        let report = domain.gc().await?;

        assert_eq!(
            report,
            GcReport {
                cached_manifests: 2,
                staging: 1,
                bytes: 9,
                ..GcReport::default()
            }
        );
        assert!(
            paths.cached_manifests_root().is_dir(),
            "the cache dir stays"
        );
        assert_eq!(std::fs::read_dir(paths.cached_manifests_root())?.count(), 0);
        assert_eq!(std::fs::read_dir(paths.staging_dir())?.count(), 0);
        Ok(())
    }

    /// A package another writer holds stops gc before it deletes anything.
    #[test(tokio::test)]
    async fn a_busy_package_stops_it_with_nothing_deleted() -> Res {
        let (domain, paths, _dir) = domain().await?;
        create(&domain, "acme/gone", &[("a.txt", "a")]).await?;
        domain.uninstall_package("acme/gone".try_into()?).await?;
        create(&domain, "acme/busy", &[("b.txt", "b")]).await?;
        let before = object_names(&paths)?;

        let busy: Namespace = "acme/busy".try_into()?;
        let _held = package_lock::lock(&LocalStorage::new(), &paths, &busy).await?;
        let result = domain.gc().await;

        assert!(
            matches!(&result, Err(Error::PackageBusy(ns)) if *ns == busy),
            "{result:?}"
        );
        assert_eq!(object_names(&paths)?, before);
        Ok(())
    }

    /// A package whose install has taken its lock but written no lineage
    /// entry yet is busy too.
    #[test(tokio::test)]
    async fn an_install_holding_its_lock_before_its_entry_is_busy() -> Res {
        let (domain, paths, _dir) = domain().await?;
        let installing: Namespace = "acme/new".try_into()?;
        let _held = package_lock::lock(&LocalStorage::new(), &paths, &installing).await?;

        let result = domain.gc().await;

        assert!(
            matches!(&result, Err(Error::PackageBusy(ns)) if *ns == installing),
            "{result:?}"
        );
        Ok(())
    }

    /// A row names an object by its digest and by a `file://` key into
    /// `objects/`; both count, and nothing else does.
    #[test(tokio::test)]
    async fn a_row_uses_its_digest_and_its_objects_key() -> Res {
        let (paths, _dir) = DomainPaths::from_temp_dir()?;
        let storage = LocalStorage::new();
        let row = ManifestRow {
            logical_key: "a.txt".into(),
            physical_key: url::Url::from_file_path(paths.objects_dir().join("old-name"))
                .expect("absolute")
                .to_string(),
            ..ManifestRow::default()
        };
        let digest = hex::encode(row.hash.digest());
        let manifest = Manifest {
            rows: vec![row],
            ..Manifest::default()
        };
        let installed = paths.installed_manifest(&("acme", "pkg").into(), "abc");
        std::fs::create_dir_all(installed.parent().expect("parent"))?;
        std::fs::write(&installed, manifest.to_jsonlines())?;

        let candidates = [digest.clone(), "old-name".into(), "unused".into()].into();
        let in_use = objects_in_use(&paths, &storage, candidates).await?;

        assert_eq!(in_use, [digest, "old-name".to_string()].into());
        Ok(())
    }

    /// A key elsewhere on disk, or not a file URL, names no object.
    #[test]
    fn a_key_outside_objects_names_nothing_extra() {
        let hash = crate::object_hash::ObjectHash::default();
        for key in ["s3://bucket/objects/x?versionId=1", "file:///tmp/other/x"] {
            assert_eq!(row_objects(&hash, key).count(), 1, "{key}");
        }
    }
}
