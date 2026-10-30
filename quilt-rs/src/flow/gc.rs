//! Reclaim what `.quilt/` holds for nothing: objects no installed manifest
//! uses, the remote-manifest cache, and staging dirs a killed write left.
//!
//! There is no reference count. An object is proven dead by reading every
//! installed manifest, of every package, while holding every package's lock:
//! an object none of them names is one no package can reach.

use std::collections::BTreeMap;
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

/// One sentence for every surface that reports a gc: "Freed 630.2 kB: 6
/// objects, 2 cached manifests", naming only what was removed, or "Nothing
/// to free".
impl std::fmt::Display for GcReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.is_empty() {
            return write!(f, "Nothing to free");
        }
        let parts: Vec<String> = [
            (self.objects, "object", "objects"),
            (self.cached_manifests, "cached manifest", "cached manifests"),
            (self.staging, "staging dir", "staging dirs"),
        ]
        .into_iter()
        .filter(|(count, _, _)| *count > 0)
        .map(|(count, one, many)| format!("{count} {}", if count == 1 { one } else { many }))
        .collect();
        write!(
            f,
            "Freed {}: {}",
            format_bytes(self.bytes),
            parts.join(", ")
        )
    }
}

/// Decimal units, as Finder counts them, with one decimal past bytes: the
/// size every report sentence says, as in "freed 630.2 kB", up to "18.4 EB"
/// for `u64::MAX`. Rounds as `format_size` in quilt-sync/ui does, so the
/// app's pages and its toasts read the same figure; the space here is a
/// plain one.
#[must_use]
pub fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["kB", "MB", "GB", "TB", "PB", "EB"];
    if bytes < 1000 {
        return format!("{bytes} B");
    }
    // In u128, so `n * 10 + scale / 2` cannot overflow for any u64. Integers,
    // so an exact half rounds up rather than as a float happens to land.
    let n = u128::from(bytes);
    let mut scale = 1000u128;
    let mut tenths = 0;
    let mut unit = UNITS[0];
    for next in UNITS {
        unit = next;
        tenths = (n * 10 + scale / 2) / scale;
        // Rounding up into the next unit's "1000.0" moves on to that unit.
        if tenths < 10_000 {
            break;
        }
        scale *= 1000;
    }
    format!("{}.{} {unit}", tenths / 10, tenths % 10)
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
    let _held = lock_every_package(paths, storage, lineage, None).await?;
    let report = sweep(paths, storage, Sweep::Delete).await?;
    info!("✔️ Collected garbage: {report:?}");
    Ok(report)
}

/// What [`gc`] would free now, deleting nothing: the same walk, by the same
/// rules, with every count and byte it would report.
///
/// Takes no package locks, so it never waits on or refuses for a busy
/// package. A writer meanwhile can change the answer, so it is an estimate;
/// [`gc`]'s report stays the exact one. A manifest it cannot read fails it,
/// as it fails gc, rather than count that manifest's objects as unused.
pub async fn gc_estimate(paths: &DomainPaths, storage: &(impl Storage + Sync)) -> Res<GcReport> {
    sweep(paths, storage, Sweep::Count).await
}

/// How much `.quilt/` holds, and how much of it [`gc`] would free.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StorageSize {
    /// Every file's length under `.quilt/`, summed; a symlink counts as
    /// itself, not what it points at.
    pub total_bytes: u64,
    /// What [`gc_estimate`] says gc would free.
    pub freeable: GcReport,
}

/// Measure `.quilt/`: its total size and [`gc_estimate`]'s answer. Takes no
/// package locks and deletes nothing.
pub async fn measure_storage(
    paths: &DomainPaths,
    storage: &(impl Storage + Sync),
) -> Res<StorageSize> {
    let dot_quilt = paths.dot_quilt_dir();
    let total_bytes = if storage.exists(&dot_quilt).await {
        let entry = Entry {
            path: dot_quilt,
            is_dir: true,
            len: 0,
        };
        sweep_entry(storage, entry, Sweep::Count).await?.1
    } else {
        0
    };
    let freeable = gc_estimate(paths, storage).await?;
    Ok(StorageSize {
        total_bytes,
        freeable,
    })
}

/// Whether a [`sweep`] deletes what it finds or only counts it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Sweep {
    Delete,
    Count,
}

/// gc's walk: every object no installed manifest uses, everything in the
/// `packages/` cache and everything in `staging/`, deleted or only counted.
/// The caller of a deleting sweep holds every package's lock.
async fn sweep(paths: &DomainPaths, storage: &(impl Storage + Sync), mode: Sweep) -> Res<GcReport> {
    let mut report = GcReport::default();

    let objects = list_files(storage, &paths.objects_dir()).await?;
    let candidates = objects.iter().map(|(name, _, _)| name.clone()).collect();
    let in_use = objects_in_use(paths, storage, candidates, None).await?;
    for (name, path, len) in objects {
        if in_use.contains(&name) {
            continue;
        }
        if mode == Sweep::Delete {
            debug!("🗑️ Removing unused object {name}");
            storage.remove_file(&path).await?;
        }
        report.objects += 1;
        report.bytes += len;
    }

    for bucket in list_entries(storage, &paths.cached_manifests_root()).await? {
        let (files, bytes) = sweep_entry(storage, bucket, mode).await?;
        report.cached_manifests += files;
        report.bytes += bytes;
    }

    for entry in list_entries(storage, &paths.staging_dir()).await? {
        let (_, bytes) = sweep_entry(storage, entry, mode).await?;
        report.staging += 1;
        report.bytes += bytes;
    }

    Ok(report)
}

/// What a pruning uninstall did with its package's objects.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pruned {
    /// The objects no other installed manifest uses were deleted; only
    /// `objects` and `bytes` are counted.
    Freed(GcReport),
    /// Another writer held this package's lock, so nothing was deleted.
    Busy(Namespace),
}

/// gc's sentence for what was freed, or the busy package that kept it.
impl std::fmt::Display for Pruned {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Freed(report) => write!(f, "{report}"),
            Self::Busy(namespace) => write!(f, "Kept downloaded files: {namespace} is busy"),
        }
    }
}

/// The objects `namespace`'s installed manifests use, every revision, with
/// their lengths: the candidates a pruning uninstall may delete. Read before
/// the uninstall removes the manifests.
///
/// Only names present in `objects/` count, each looked up by name rather
/// than by listing the store, so the work follows the package's size.
///
/// A manifest or an object that can't be read does not stop it: the rest
/// are still candidates, and the first such error comes back beside them,
/// so the caller can say not every file was deleted. Refusing instead would
/// leave a surface that always prunes unable to remove the package.
pub(crate) async fn package_objects(
    paths: &DomainPaths,
    storage: &(impl Storage + Sync),
    namespace: &Namespace,
) -> Res<(BTreeMap<String, u64>, Option<Error>)> {
    let objects = paths.objects_dir();
    let mut seen = BTreeSet::new();
    let mut candidates = BTreeMap::new();
    let mut unread = None;
    let manifests = paths.installed_manifests_dir(namespace);
    for (_, manifest_path, _) in list_files(storage, &manifests).await? {
        let manifest = match Manifest::from_path(storage, &manifest_path).await {
            Ok(manifest) => manifest,
            Err(err) => {
                unread.get_or_insert(err);
                continue;
            }
        };
        for row in &manifest.rows {
            for name in row_objects(&row.hash, &row.physical_key) {
                if !seen.insert(name.clone()) {
                    continue;
                }
                match object_len(storage, &objects.join(&name)).await {
                    Ok(Some(len)) => {
                        candidates.insert(name, len);
                    }
                    Ok(None) => {}
                    Err(err) => {
                        unread.get_or_insert(err);
                    }
                }
            }
        }
    }
    Ok((candidates, unread))
}

/// The length of the object at `path`, or `None` if there is none. Any
/// other failure to read it is an error, not an absence.
async fn object_len(storage: &(impl Storage + Sync), path: &Path) -> Res<Option<u64>> {
    let file = match storage.open_file(path).await {
        Ok(file) => file,
        Err(err) if err.is_not_found() => return Ok(None),
        Err(err) => return Err(err),
    };
    Ok(Some(file.metadata().await?.len()))
}

/// Delete the `candidates` no installed manifest uses, after an uninstall
/// removed the manifests that named them.
///
/// Try-locks every package first, as [`gc`] does: a commit elsewhere files
/// an object before it writes the manifest naming it, so without the locks
/// this could delete an object a new manifest is about to name. A busy one
/// deletes nothing and is answered as [`Pruned::Busy`]. The caller must not
/// hold any package's lock.
pub(crate) async fn prune(
    paths: &DomainPaths,
    storage: &(impl Storage + Sync),
    lineage: &DomainLineage,
    candidates: BTreeMap<String, u64>,
) -> Res<Pruned> {
    // Nothing to delete needs no lock, and a busy package then keeps nothing.
    if candidates.is_empty() {
        return Ok(Pruned::Freed(GcReport::default()));
    }
    let _held = match lock_every_package(paths, storage, lineage, None).await {
        Ok(held) => held,
        Err(Error::PackageBusy(namespace)) => return Ok(Pruned::Busy(namespace)),
        Err(err) => return Err(err),
    };
    let in_use = objects_in_use(paths, storage, candidates.keys().cloned().collect(), None).await?;
    let mut report = GcReport::default();
    for (name, len) in candidates {
        if in_use.contains(&name) {
            continue;
        }
        let path = paths.objects_dir().join(&name);
        if !storage.exists(&path).await {
            continue;
        }
        debug!("🗑️ Removing object {name}");
        storage.remove_file(&path).await?;
        report.objects += 1;
        report.bytes += len;
    }
    info!("✔️ Pruned: {report:?}");
    Ok(Pruned::Freed(report))
}

/// Of `candidates` — object file names — those some installed manifest
/// uses: any revision of any package under `installed/`.
///
/// A row uses an object by its hash (the name is the digest's hex) and, for
/// a `file://` physical key into an `objects/` directory, by that file's
/// name. The two usually agree; a row carried over by a recommit can keep
/// its old key under a new hash, so both count.
///
/// `skip` names a package whose manifests are not read: a caller that reads
/// them itself asks only about everyone else's.
///
/// Stops reading once every candidate is found. Fails on a manifest it
/// cannot read, rather than treat its objects as unused.
pub(crate) async fn objects_in_use(
    paths: &DomainPaths,
    storage: &(impl Storage + Sync),
    mut candidates: BTreeSet<String>,
    skip: Option<&Namespace>,
) -> Res<BTreeSet<String>> {
    let skipped = skip.map(|namespace| paths.installed_manifests_dir(namespace));
    let mut in_use = BTreeSet::new();
    for owner in list_dirs(storage, &paths.installed_dir()).await? {
        for package in list_dirs(storage, &owner).await? {
            if skipped.as_ref() == Some(&package) {
                continue;
            }
            for (_, manifest_path, _) in list_files(storage, &package).await? {
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
pub(crate) fn row_objects(
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
///
/// A package created or first installed after this scan takes a lock that
/// is not held here, so its new objects are not protected. That is by
/// design: only the user starts a new package, background updates touch
/// only installed ones, and starting one while gc runs is unsupported.
///
/// `except` is a package the caller already holds: the lock is not
/// reentrant, so trying it again would find it busy.
pub(crate) async fn lock_every_package(
    paths: &DomainPaths,
    storage: &(impl Storage + Sync),
    lineage: &DomainLineage,
    except: Option<&Namespace>,
) -> Res<Vec<LockGuard>> {
    let mut namespaces: BTreeSet<Namespace> = lineage.namespaces().into_iter().collect();
    for owner_dir in list_dirs(storage, &paths.locks_dir()).await? {
        let Some(owner) = owner_dir.file_name() else {
            continue;
        };
        let owner = owner.to_string_lossy();
        // Every file, hidden ones included: a package named `.foo` locks
        // `.foo.lock`.
        for entry in list_entries(storage, &owner_dir).await? {
            if entry.is_dir {
                continue;
            }
            let Some(lock_file) = entry.path.file_name() else {
                continue;
            };
            if let Some(name) = lock_file.to_string_lossy().strip_suffix(".lock") {
                namespaces.insert((owner.as_ref(), name).into());
            }
        }
    }

    if let Some(except) = except {
        namespaces.remove(except);
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

/// The entries of `dir`, or none if it does not exist. An entry that goes
/// while it is read, as a lock-free [`gc_estimate`] can see a finishing
/// download's staging dir go, is not one.
async fn list_entries(storage: &(impl Storage + Sync), dir: &Path) -> Res<Vec<Entry>> {
    let Some(mut entries) = read_dir_if_there(storage, dir).await? else {
        return Ok(Vec::new());
    };
    let mut found = Vec::new();
    while let Some(entry) = entries.next_entry().await? {
        let Some(metadata) = metadata_if_there(&entry).await? else {
            continue;
        };
        found.push(Entry {
            path: entry.path(),
            is_dir: metadata.is_dir(),
            len: metadata.len(),
        });
    }
    found.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(found)
}

/// The subdirectories of `dir`: a stray file beside them, such as the OS's
/// own, is not one.
async fn list_dirs(storage: &(impl Storage + Sync), dir: &Path) -> Res<Vec<PathBuf>> {
    Ok(list_entries(storage, dir)
        .await?
        .into_iter()
        .filter(|entry| entry.is_dir)
        .map(|entry| entry.path)
        .collect())
}

/// The visible files of `dir` as `(name, path, length)`, or none if it does
/// not exist. A hidden file is a write's stranded `.tmp-<uuid>` or the OS's
/// own, and is left alone; a file that goes while it is read is not one.
pub(crate) async fn list_files(
    storage: &(impl Storage + Sync),
    dir: &Path,
) -> Res<Vec<(String, PathBuf, u64)>> {
    let Some(mut entries) = read_dir_if_there(storage, dir).await? else {
        return Ok(Vec::new());
    };
    let mut files = Vec::new();
    while let Some(entry) = entries.next_entry().await? {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(metadata) = metadata_if_there(&entry).await? else {
            continue;
        };
        if name.starts_with('.') || !metadata.is_file() {
            continue;
        }
        files.push((name, entry.path(), metadata.len()));
    }
    files.sort();
    Ok(files)
}

/// `dir`'s entries, or `None` if it is not there. Any other failure to read
/// it is an error.
async fn read_dir_if_there(
    storage: &(impl Storage + Sync),
    dir: &Path,
) -> Res<Option<tokio::fs::ReadDir>> {
    match storage.read_dir(dir).await {
        Ok(entries) => Ok(Some(entries)),
        Err(err) if err.is_not_found() => Ok(None),
        Err(err) => Err(err),
    }
}

/// `entry`'s own metadata, or `None` if it went since it was listed. Any
/// other failure to read it is an error.
async fn metadata_if_there(entry: &tokio::fs::DirEntry) -> Res<Option<std::fs::Metadata>> {
    match entry.metadata().await {
        Ok(metadata) => Ok(Some(metadata)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err.into()),
    }
}

/// Count how many files `entry` holds and their summed length, a file
/// counting as one, and remove it if `mode` deletes. A symlink is not
/// followed.
async fn sweep_entry(
    storage: &(impl Storage + Sync),
    entry: Entry,
    mode: Sweep,
) -> Res<(usize, u64)> {
    if !entry.is_dir {
        if mode == Sweep::Delete {
            storage.remove_file(&entry.path).await?;
        }
        return Ok((1, entry.len));
    }
    let (mut files, mut bytes) = (0, 0);
    let mut dirs = vec![entry.path.clone()];
    while let Some(dir) = dirs.pop() {
        // Gone since it was listed: a lock-free count races the writers.
        let Some(mut entries) = read_dir_if_there(storage, &dir).await? else {
            continue;
        };
        while let Some(child) = entries.next_entry().await? {
            let Some(metadata) = metadata_if_there(&child).await? else {
                continue;
            };
            if metadata.is_dir() {
                dirs.push(child.path());
            } else {
                files += 1;
                bytes += metadata.len();
            }
        }
    }
    if mode == Sweep::Delete {
        storage.remove_dir_all(&entry.path).await?;
    }
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
        std::fs::read_dir(paths.objects_dir())?
            .map(|entry| Ok(entry?.file_name().to_string_lossy().into_owned()))
            .collect()
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

    /// A package whose name starts with a dot has a hidden lock file; one
    /// held before the package has a lineage entry is busy too.
    #[test(tokio::test)]
    async fn a_held_lock_of_a_dot_named_package_is_busy() -> Res {
        let (domain, paths, _dir) = domain().await?;
        let creating: Namespace = "acme/.foo".try_into()?;
        let _held = package_lock::lock(&LocalStorage::new(), &paths, &creating).await?;
        assert!(paths.locks_dir().join("acme/.foo.lock").is_file());

        let result = domain.gc().await;

        assert!(
            matches!(&result, Err(Error::PackageBusy(ns)) if *ns == creating),
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
        let in_use = objects_in_use(&paths, &storage, candidates, None).await?;

        assert_eq!(in_use, [digest, "old-name".to_string()].into());
        Ok(())
    }

    #[test]
    fn the_report_names_only_what_was_removed() {
        assert_eq!(GcReport::default().to_string(), "Nothing to free");
        let report = GcReport {
            objects: 6,
            cached_manifests: 2,
            staging: 0,
            bytes: 630_200,
        };
        assert_eq!(
            report.to_string(),
            "Freed 630.2 kB: 6 objects, 2 cached manifests"
        );
        let report = GcReport {
            objects: 1,
            cached_manifests: 0,
            staging: 1,
            bytes: 12,
        };
        assert_eq!(report.to_string(), "Freed 12 B: 1 object, 1 staging dir");
    }

    /// The same table as `format_size`'s in quilt-sync/ui, so the two read
    /// the same figure for the same size.
    #[test]
    fn bytes_read_in_decimal_units() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(999), "999 B");
        assert_eq!(format_bytes(1500), "1.5 kB");
        assert_eq!(format_bytes(44_000), "44.0 kB");
        assert_eq!(format_bytes(114_100), "114.1 kB");
        assert_eq!(format_bytes(211_900), "211.9 kB");
        assert_eq!(format_bytes(1_900_000), "1.9 MB");
        assert_eq!(format_bytes(2_500_000), "2.5 MB");
        assert_eq!(format_bytes(2_560_000), "2.6 MB");
        assert_eq!(format_bytes(999_960), "1.0 MB");
        assert_eq!(format_bytes(3_000_000_000), "3.0 GB");
        assert_eq!(format_bytes(1_200_000_000_000), "1.2 TB");
        assert_eq!(format_bytes(2_500_000_000_000), "2.5 TB");
    }

    /// An exact half rounds up, in integers, as `format_size` does.
    #[test]
    fn an_exact_half_rounds_up() {
        assert_eq!(format_bytes(1_250), "1.3 kB");
        assert_eq!(format_bytes(1_350), "1.4 kB");
        assert_eq!(format_bytes(2_250_000), "2.3 MB");
    }

    /// No u64 overflows the arithmetic, and rounding at each unit's edge moves
    /// on to the next unit rather than reading "1000.0".
    #[test]
    fn bytes_hold_at_the_unit_edges_and_at_the_top() {
        assert_eq!(format_bytes(1_000), "1.0 kB");
        assert_eq!(format_bytes(999_949), "999.9 kB");
        assert_eq!(format_bytes(999_950), "1.0 MB");
        assert_eq!(format_bytes(999_949_999), "999.9 MB");
        assert_eq!(format_bytes(999_950_000), "1.0 GB");
        assert_eq!(format_bytes(999_950_000_000), "1.0 TB");
        assert_eq!(format_bytes(999_949_999_999_999), "999.9 TB");
        assert_eq!(format_bytes(999_950_000_000_000), "1.0 PB");
        assert_eq!(format_bytes(999_950_000_000_000_000), "1.0 EB");
        assert_eq!(format_bytes(u64::MAX - 1), "18.4 EB");
        assert_eq!(format_bytes(u64::MAX), "18.4 EB");
    }

    /// A stray file where a package dir or an owner dir belongs is skipped,
    /// not read as one.
    #[test(tokio::test)]
    async fn a_stray_file_among_the_dirs_is_skipped() -> Res {
        let (domain, paths, _dir) = domain().await?;
        create(&domain, "acme/kept", &[("a.txt", "a")]).await?;
        std::fs::write(paths.installed_dir().join(".DS_Store"), "")?;
        std::fs::write(paths.installed_dir().join("acme/.DS_Store"), "")?;
        std::fs::write(paths.locks_dir().join(".DS_Store"), "")?;

        assert!(domain.gc().await?.is_empty());
        Ok(())
    }

    /// Uninstalling with prune deletes the package's own objects and keeps
    /// the one another package still uses.
    #[test(tokio::test)]
    async fn a_pruning_uninstall_deletes_only_its_unshared_objects() -> Res {
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

        let pruned = domain
            .uninstall_package_pruning("acme/gone".try_into()?)
            .await?;

        assert_eq!(
            pruned,
            Pruned::Freed(GcReport {
                objects: 1,
                bytes: 2,
                ..GcReport::default()
            })
        );
        assert_eq!(pruned.to_string(), "Freed 2 B: 1 object");
        assert_eq!(object_names(&paths)?, kept);
        assert!(
            domain
                .get_installed_package(&"acme/gone".try_into()?)
                .await?
                .is_none()
        );
        Ok(())
    }

    /// Every revision's objects are candidates, not only the latest one's.
    #[test(tokio::test)]
    async fn a_pruning_uninstall_deletes_every_revisions_objects() -> Res {
        let (domain, paths, dir) = domain().await?;
        create(&domain, "acme/gone", &[("b.txt", "b1")]).await?;
        let namespace: Namespace = "acme/gone".try_into()?;
        std::fs::write(dir.path().join("home/acme/gone/b.txt"), "b22")?;
        domain
            .get_installed_package(&namespace)
            .await?
            .expect("installed")
            .commit(
                "second".to_string(),
                crate::flow::UserMeta::Keep,
                None,
                None,
            )
            .await?;
        assert_eq!(object_names(&paths)?.len(), 2);

        let pruned = domain.uninstall_package_pruning(namespace).await?;

        assert_eq!(pruned.to_string(), "Freed 5 B: 2 objects");
        assert!(object_names(&paths)?.is_empty());
        Ok(())
    }

    /// The prune is scoped: another uninstall's leftovers, the manifest
    /// cache and staging stay for gc.
    #[test(tokio::test)]
    async fn a_pruning_uninstall_leaves_other_leftovers() -> Res {
        let (domain, paths, _dir) = domain().await?;
        create(&domain, "acme/earlier", &[("x.txt", "x")]).await?;
        domain.uninstall_package("acme/earlier".try_into()?).await?;
        let leftover = object_names(&paths)?;
        let bucket = paths.cached_manifests_dir("bucket");
        std::fs::create_dir_all(&bucket)?;
        std::fs::write(bucket.join("1111"), "12345")?;
        let stranded = paths.staging_dir().join("some-uuid");
        std::fs::create_dir_all(&stranded)?;

        create(&domain, "acme/gone", &[("b.txt", "bb")]).await?;
        let pruned = domain
            .uninstall_package_pruning("acme/gone".try_into()?)
            .await?;

        assert_eq!(pruned.to_string(), "Freed 2 B: 1 object");
        assert_eq!(object_names(&paths)?, leftover);
        assert!(bucket.join("1111").is_file());
        assert!(stranded.is_dir());
        Ok(())
    }

    /// Every object still in use by another package: nothing to free.
    #[test(tokio::test)]
    async fn a_pruning_uninstall_of_shared_content_frees_nothing() -> Res {
        let (domain, paths, _dir) = domain().await?;
        create(&domain, "acme/kept", &[("a.txt", "a")]).await?;
        create(&domain, "acme/gone", &[("a.txt", "a")]).await?;
        let before = object_names(&paths)?;

        let pruned = domain
            .uninstall_package_pruning("acme/gone".try_into()?)
            .await?;

        assert_eq!(pruned.to_string(), "Nothing to free");
        assert_eq!(object_names(&paths)?, before);
        Ok(())
    }

    /// Another package busy in another writer: the uninstall stands, the
    /// objects stay, and the result names the busy package.
    #[test(tokio::test)]
    async fn a_busy_package_keeps_the_objects_but_not_the_package() -> Res {
        let (domain, paths, _dir) = domain().await?;
        create(&domain, "acme/busy", &[("a.txt", "a")]).await?;
        create(&domain, "acme/gone", &[("b.txt", "bb")]).await?;
        let before = object_names(&paths)?;
        let busy: Namespace = "acme/busy".try_into()?;
        let _held = package_lock::lock(&LocalStorage::new(), &paths, &busy).await?;

        let pruned = domain
            .uninstall_package_pruning("acme/gone".try_into()?)
            .await?;

        assert_eq!(pruned, Pruned::Busy(busy));
        assert_eq!(
            pruned.to_string(),
            "Kept downloaded files: acme/busy is busy"
        );
        assert_eq!(object_names(&paths)?, before);
        assert!(
            domain
                .get_installed_package(&"acme/gone".try_into()?)
                .await?
                .is_none()
        );
        Ok(())
    }

    /// Another package's manifest that can't be read stops the prune with
    /// nothing deleted; the uninstall has happened, and the error says so.
    #[test(tokio::test)]
    async fn an_unreadable_manifest_keeps_the_objects() -> Res {
        let (domain, paths, _dir) = domain().await?;
        create(&domain, "acme/broken", &[("a.txt", "a")]).await?;
        create(&domain, "acme/gone", &[("b.txt", "bb")]).await?;
        let before = object_names(&paths)?;
        let broken = paths.installed_manifests_dir(&"acme/broken".try_into()?);
        for entry in std::fs::read_dir(&broken)? {
            std::fs::write(entry?.path(), "not a manifest")?;
        }

        let result = domain
            .uninstall_package_pruning("acme/gone".try_into()?)
            .await;

        let Err(err @ Error::PruneFailed(_, _, None)) = result else {
            panic!("expected PruneFailed, got {result:?}");
        };
        assert!(
            err.to_string().starts_with(
                "Uninstalled acme/gone, but not all of its downloaded files were deleted: "
            ),
            "{err}"
        );
        assert_eq!(object_names(&paths)?, before);
        assert!(
            domain
                .get_installed_package(&"acme/gone".try_into()?)
                .await?
                .is_none()
        );
        Ok(())
    }

    /// The package's own manifest that can't be read does not stop the
    /// removal: the package goes, the objects of what could be read go, and
    /// the error says not every file was deleted.
    #[test(tokio::test)]
    async fn its_own_unreadable_manifest_removes_and_says_files_were_left() -> Res {
        let (domain, paths, dir) = domain().await?;
        create(&domain, "acme/broken", &[("b.txt", "b1")]).await?;
        let namespace: Namespace = "acme/broken".try_into()?;
        let first: Vec<_> = std::fs::read_dir(paths.installed_manifests_dir(&namespace))?
            .map(|entry| entry.map(|e| e.path()))
            .collect::<Result<_, _>>()?;
        std::fs::write(dir.path().join("home/acme/broken/b.txt"), "b22")?;
        domain
            .get_installed_package(&namespace)
            .await?
            .expect("installed")
            .commit(
                "second".to_string(),
                crate::flow::UserMeta::Keep,
                None,
                None,
            )
            .await?;
        for path in first {
            std::fs::write(path, "not a manifest")?;
        }

        let result = domain.uninstall_package_pruning(namespace.clone()).await;

        let Err(err @ Error::PruneFailed(_, _, None)) = result else {
            panic!("expected PruneFailed, got {result:?}");
        };
        assert!(
            err.to_string().starts_with(
                "Uninstalled acme/broken, but not all of its downloaded files were deleted: "
            ),
            "{err}"
        );
        assert!(domain.get_installed_package(&namespace).await?.is_none());
        assert_eq!(
            object_names(&paths)?.len(),
            1,
            "the readable revision's object went; the unread one's stays for gc"
        );
        assert_eq!(domain.gc().await?.objects, 1, "and gc frees it");
        Ok(())
    }

    /// An unreadable revision of its own and a busy package together: the
    /// error names both, the read error for the files it could not count and
    /// the busy package for the ones it counted and kept.
    #[test(tokio::test)]
    async fn an_unreadable_revision_and_a_busy_package_are_both_named() -> Res {
        let (domain, paths, dir) = domain().await?;
        create(&domain, "acme/busy", &[("a.txt", "a")]).await?;
        create(&domain, "acme/broken", &[("b.txt", "b1")]).await?;
        let namespace: Namespace = "acme/broken".try_into()?;
        let first: Vec<_> = std::fs::read_dir(paths.installed_manifests_dir(&namespace))?
            .map(|entry| entry.map(|e| e.path()))
            .collect::<Result<_, _>>()?;
        std::fs::write(dir.path().join("home/acme/broken/b.txt"), "b22")?;
        domain
            .get_installed_package(&namespace)
            .await?
            .expect("installed")
            .commit(
                "second".to_string(),
                crate::flow::UserMeta::Keep,
                None,
                None,
            )
            .await?;
        for path in first {
            std::fs::write(path, "not a manifest")?;
        }
        let before = object_names(&paths)?;
        let busy: Namespace = "acme/busy".try_into()?;
        let _held = package_lock::lock(&LocalStorage::new(), &paths, &busy).await?;

        let result = domain.uninstall_package_pruning(namespace).await;

        let Err(err @ Error::PruneFailed(_, _, Some(_))) = result else {
            panic!("expected PruneFailed naming the busy package, got {result:?}");
        };
        let words = err.to_string();
        assert!(
            words.starts_with(
                "Uninstalled acme/broken, but not all of its downloaded files were deleted: "
            ),
            "{words}"
        );
        assert!(
            words.ends_with("; acme/busy is busy, so the rest were kept too"),
            "{words}"
        );
        assert_eq!(
            object_names(&paths)?,
            before,
            "the busy package kept them all"
        );
        Ok(())
    }

    /// No candidates: nothing to free, and no other package's lock is
    /// asked for, so a busy one does not read as files kept.
    #[test(tokio::test)]
    async fn no_candidates_free_nothing_without_taking_locks() -> Res {
        let (domain, paths, _dir) = domain().await?;
        create(&domain, "acme/busy", &[("a.txt", "a")]).await?;
        create(&domain, "acme/empty", &[]).await?;
        let busy: Namespace = "acme/busy".try_into()?;
        let _held = package_lock::lock(&LocalStorage::new(), &paths, &busy).await?;

        let pruned = domain
            .uninstall_package_pruning("acme/empty".try_into()?)
            .await?;

        assert_eq!(pruned, Pruned::Freed(GcReport::default()));
        Ok(())
    }

    /// A package that is not installed is refused before anything else.
    #[test(tokio::test)]
    async fn a_pruning_uninstall_of_a_missing_package_is_refused() -> Res {
        let (domain, _paths, _dir) = domain().await?;
        let result = domain
            .uninstall_package_pruning("acme/none".try_into()?)
            .await;
        assert!(
            matches!(
                result,
                Err(Error::InstallPackage(
                    crate::InstallPackageError::NotInstalled(_)
                ))
            ),
            "{result:?}"
        );
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

    /// Every file's length under `dir`, summed, read straight off disk.
    fn bytes_on_disk(dir: &Path) -> Res<u64> {
        let mut bytes = 0;
        let mut dirs = vec![dir.to_path_buf()];
        while let Some(dir) = dirs.pop() {
            for entry in std::fs::read_dir(dir)? {
                let entry = entry?;
                let metadata = entry.metadata()?;
                if metadata.is_dir() {
                    dirs.push(entry.path());
                } else {
                    bytes += metadata.len();
                }
            }
        }
        Ok(bytes)
    }

    /// An uninstalled package's objects, a cached manifest and a stranded
    /// staging dir: something of every kind gc frees.
    async fn domain_with_garbage() -> Res<(LocalDomain, DomainPaths, tempfile::TempDir)> {
        let (domain, paths, dir) = domain().await?;
        create(&domain, "acme/kept", &[("shared.txt", "shared")]).await?;
        create(
            &domain,
            "acme/gone",
            &[("shared.txt", "shared"), ("b.txt", "bb")],
        )
        .await?;
        domain.uninstall_package("acme/gone".try_into()?).await?;
        let bucket = paths.cached_manifests_dir("bucket");
        std::fs::create_dir_all(&bucket)?;
        std::fs::write(bucket.join("1111"), "12345")?;
        let stranded = paths.staging_dir().join("some-uuid");
        std::fs::create_dir_all(&stranded)?;
        std::fs::write(stranded.join("file"), "1")?;
        Ok((domain, paths, dir))
    }

    /// The estimate deletes nothing, and is what gc then frees when nothing
    /// changes in between.
    #[test(tokio::test)]
    async fn the_estimate_is_what_gc_then_frees() -> Res {
        let (domain, paths, _dir) = domain_with_garbage().await?;
        let objects = object_names(&paths)?;
        let total = bytes_on_disk(&paths.dot_quilt_dir())?;

        let estimate = gc_estimate(&paths, &LocalStorage::new()).await?;

        assert_eq!(object_names(&paths)?, objects, "nothing is deleted");
        assert_eq!(bytes_on_disk(&paths.dot_quilt_dir())?, total);
        assert_eq!(
            estimate,
            GcReport {
                objects: 1,
                cached_manifests: 1,
                staging: 1,
                bytes: 8,
            }
        );
        assert_eq!(domain.gc().await?, estimate);
        Ok(())
    }

    /// The estimate takes no locks, so a package busy elsewhere neither stops
    /// nor holds it up.
    #[test(tokio::test)]
    async fn the_estimate_answers_while_a_package_is_busy() -> Res {
        let (domain, paths, _dir) = domain_with_garbage().await?;
        let busy: Namespace = "acme/kept".try_into()?;
        let _held = package_lock::lock(&LocalStorage::new(), &paths, &busy).await?;

        let size = domain.measure_storage().await?;

        assert_eq!(size.freeable.bytes, 8);
        assert!(
            matches!(domain.gc().await, Err(Error::PackageBusy(_))),
            "gc itself still refuses"
        );
        Ok(())
    }

    /// The total is every byte under `.quilt/`, and gc takes exactly what it
    /// reports off it.
    #[test(tokio::test)]
    async fn the_total_is_every_byte_under_dot_quilt() -> Res {
        let (domain, paths, _dir) = domain_with_garbage().await?;

        let before = domain.measure_storage().await?;

        assert_eq!(before.total_bytes, bytes_on_disk(&paths.dot_quilt_dir())?);
        let freed = domain.gc().await?;
        let after = domain.measure_storage().await?;
        assert_eq!(after.total_bytes, before.total_bytes - freed.bytes);
        assert!(after.freeable.is_empty());
        Ok(())
    }

    /// A dir that goes between its listing and its walk, as a finishing
    /// download's staging dir does, counts as nothing rather than failing
    /// the measure.
    #[test(tokio::test)]
    async fn a_dir_gone_before_its_walk_counts_nothing() -> Res {
        let dir = tempfile::TempDir::new()?;
        let storage = LocalStorage::new();
        let staging = dir.path().join("staging");
        std::fs::create_dir_all(staging.join("some-uuid/nested"))?;
        std::fs::write(staging.join("some-uuid/nested/file"), "1")?;
        let listed = list_entries(&storage, &staging).await?;
        std::fs::remove_dir_all(&staging)?;

        for entry in listed {
            assert_eq!(sweep_entry(&storage, entry, Sweep::Count).await?, (0, 0));
        }
        assert!(list_entries(&storage, &staging).await?.is_empty());
        assert_eq!(list_files(&storage, &staging).await?, Vec::new());
        Ok(())
    }

    /// A domain that has never written anything holds nothing.
    #[test(tokio::test)]
    async fn an_empty_domain_measures_nothing() -> Res {
        let dir = tempfile::TempDir::new()?;
        let paths = DomainPaths::new(dir.path().to_path_buf());

        let size = measure_storage(&paths, &LocalStorage::new()).await?;

        assert_eq!(size, StorageSize::default());
        Ok(())
    }
}
