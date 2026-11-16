//!
//! Reads and writes the files of per-package lineage. A ref is written only
//! under its package's lock, which the caller holds and proves with a
//! [`PackageLock`]: the lock is not reentrant, so the store never takes one
//! to write. Creating or removing a ref and changing the home also take the
//! domain file lock, always after any package lock.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::Path;
use std::path::PathBuf;

use super::RefEntry;
use super::format::DomainConfig;
use super::format::MigrationState;
use super::format::PackageRef;
use super::format::TipCache;
use crate::Error;
use crate::InstallPackageError;
use crate::Res;
use crate::error::LineageError;
use crate::io::storage::ByteStream;
use crate::io::storage::LockGuard;
use crate::io::storage::Storage;
use crate::io::storage::StorageExt;
use crate::lineage::Home;
use crate::lineage::LineageHook;
use crate::lineage::PackageLineage;
use crate::package_lock::PackageLock;
use crate::paths::DomainPaths;
use chrono::Utc;
use quilt_uri::Namespace;
use tracing::warn;

/// The refs, tip caches and domain config of one domain.
pub(crate) struct RefStore {
    paths: DomainPaths,
    hook: Option<LineageHook>,
}

/// A file in `refs/<owner>/` whose name parses as a package's.
struct RefFile {
    path: PathBuf,
    namespace: Namespace,
    entry: RefEntry,
}

impl RefStore {
    pub(crate) fn new(paths: DomainPaths) -> Self {
        Self { paths, hook: None }
    }

    /// Tell `hook` of every ref this store's writes change. It runs under
    /// the package's lock, after the write landed, so it must return at once.
    pub(crate) fn with_hook(mut self, hook: LineageHook) -> Self {
        self.hook = Some(hook);
        self
    }

    /// The home in the domain config, empty when the config has none.
    pub(crate) async fn home(&self, storage: &(impl Storage + Sync)) -> Res<Home> {
        match self.read_config(storage).await? {
            Some(config) => Ok(config.home),
            None => Err(LineageError::Missing.into()),
        }
    }

    /// Every package with a ref, whether or not it parses. Leftover temps,
    /// tip caches and names that are no namespace are skipped, as
    /// `lock_every_package` skips lock files.
    pub(crate) async fn namespaces(
        &self,
        storage: &(impl Storage + Sync),
    ) -> Res<BTreeSet<Namespace>> {
        Ok(self
            .files(storage)
            .await?
            .into_iter()
            .filter(|file| matches!(file.entry, RefEntry::Ref(_)))
            .map(|file| file.namespace)
            .collect())
    }

    pub(crate) async fn contains(&self, storage: &impl Storage, namespace: &Namespace) -> bool {
        storage.is_file(self.paths.package_ref(namespace)).await
    }

    /// A package with no ref is not installed. A ref that does not parse is
    /// an error naming its file, for that package alone.
    pub(crate) async fn read(
        &self,
        storage: &(impl Storage + Sync),
        namespace: &Namespace,
    ) -> Res<PackageLineage> {
        let (stored, tip) = self.read_ref(storage, namespace).await?;
        Ok(stored.into_lineage(tip.as_ref()))
    }

    /// Each installed package's lineage, read on its own.
    pub(crate) async fn read_all(
        &self,
        storage: &(impl Storage + Sync),
    ) -> Res<BTreeMap<Namespace, Res<PackageLineage>>> {
        let mut all = BTreeMap::new();
        for namespace in self.namespaces(storage).await? {
            let lineage = self.read(storage, &namespace).await;
            all.insert(namespace, lineage);
        }
        Ok(all)
    }

    /// Re-reads the locked package's ref, lets `step` change its lineage,
    /// and stores the change before returning: the tip cache, then the ref,
    /// then the hook, which is not told of a change to the tip alone. An
    /// error from `step` writes nothing.
    pub(crate) async fn update<T>(
        &self,
        storage: &(impl Storage + Sync),
        held: &PackageLock,
        step: impl FnOnce(&mut PackageLineage) -> Res<T>,
    ) -> Res<T> {
        let namespace = held.namespace();
        self.writable_config(storage).await?;
        let (stored, tip) = self.read_ref(storage, namespace).await?;
        let before = stored.clone().into_lineage(tip.as_ref());
        let mut lineage = before.clone();
        let out = step(&mut lineage)?;
        if let Some(after) = self
            .write(storage, namespace, Some(stored), tip, &lineage)
            .await?
        {
            self.tell(namespace, Some(&before), Some(&after));
        }
        Ok(out)
    }

    /// Records the locked package, whose files were placed under `placed`.
    /// Under the domain file lock, so no home change lands in between, it
    /// reads the home again and refuses with [`LineageError::HomeChanged`],
    /// writing nothing, if it is no longer `placed`.
    pub(crate) async fn create(
        &self,
        storage: &(impl Storage + Sync),
        held: &PackageLock,
        lineage: &PackageLineage,
        placed: &Home,
    ) -> Res {
        let namespace = held.namespace();
        let (_domain, config) = self.lock_domain(storage).await?;
        if config.home != *placed {
            return Err(LineageError::HomeChanged {
                placed: placed.as_ref().clone(),
                home: config.home.as_ref().clone(),
            }
            .into());
        }
        if self.contains(storage, namespace).await {
            return Err(Error::InstallPackage(
                InstallPackageError::AlreadyInstalled(namespace.clone()),
            ));
        }
        self.create_folders(storage, namespace).await?;
        // A tip a killed removal left behind is replaced or removed.
        let tip = self.read_tip(storage, namespace).await;
        if let Some(after) = self.write(storage, namespace, None, tip, lineage).await? {
            self.tell(namespace, None, Some(&after));
        }
        Ok(())
    }

    /// Removes the locked package's ref, then its tip. A ref that does not
    /// parse is removed too; one in a format or state this build does not
    /// support is refused, since it may be a newer build's to remove.
    pub(crate) async fn remove(&self, storage: &(impl Storage + Sync), held: &PackageLock) -> Res {
        let namespace = held.namespace();
        let _domain = self.lock_domain(storage).await?;
        let was = match self.read(storage, namespace).await {
            Ok(lineage) => Some(lineage),
            Err(Error::Lineage(LineageError::FileParse { .. })) => None,
            Err(err) => return Err(err),
        };
        let path = self.paths.package_ref(namespace);
        storage.remove_file(&path).await?;
        crate::failpoint::hit("ref.remove.after-unlink");
        storage.sync_dir(folder_of(&path)).await;
        self.remove_tip(storage, namespace).await;
        self.tell(namespace, was.as_ref(), None);
        Ok(())
    }

    /// Points the domain at `home`, creating its folder first. Refuses with
    /// [`LineageError::HomeInUse`] while any ref exists, since each package's
    /// files are under the current home, unless the config has no home.
    pub(crate) async fn set_home(&self, storage: &(impl Storage + Sync), home: &Home) -> Res {
        self.change_home(storage, home, false).await
    }

    /// [`Self::set_home`] without the check for installed packages, whose
    /// refs then point at folders under the new home.
    pub(crate) async fn overwrite_home(&self, storage: &(impl Storage + Sync), home: &Home) -> Res {
        self.change_home(storage, home, true).await
    }

    /// Deletes the temps that killed writes left. A package's go only while
    /// its lock is free, since a writer makes them only under it, and the
    /// domain config's only while the domain file lock is free; a busy one
    /// waits for the next sweep. It takes package locks, so a caller holding
    /// one would see its own package busy.
    pub(crate) async fn sweep(&self, storage: &(impl Storage + Sync)) -> Res {
        let mut temps: BTreeMap<Namespace, Vec<PathBuf>> = BTreeMap::new();
        for file in self.files(storage).await? {
            if matches!(file.entry, RefEntry::RefTemp(_) | RefEntry::TipTemp(_)) {
                temps.entry(file.namespace).or_default().push(file.path);
            }
        }
        for (namespace, paths) in temps {
            if let Some(_held) = PackageLock::try_take(storage, &self.paths, &namespace).await? {
                remove_leftovers(storage, &paths).await?;
            }
        }

        let config_temps: Vec<PathBuf> = entries(storage, &self.paths.dot_quilt_dir())
            .await?
            .into_iter()
            .filter(|(path, is_dir)| {
                !is_dir
                    && path.file_name().is_some_and(|name| {
                        matches!(RefEntry::classify(name), RefEntry::RefTemp(name) if name == "domain")
                    })
            })
            .map(|(path, _)| path)
            .collect();
        if !config_temps.is_empty()
            && let Some(_domain) = storage
                .try_lock_exclusive(self.paths.domain_file_lock())
                .await?
        {
            remove_leftovers(storage, &config_temps).await?;
        }
        Ok(())
    }

    fn tell(
        &self,
        namespace: &Namespace,
        was: Option<&PackageLineage>,
        is: Option<&PackageLineage>,
    ) {
        if let Some(LineageHook(hook)) = &self.hook {
            hook(namespace, was, is);
        }
    }

    async fn read_config(&self, storage: &(impl Storage + Sync)) -> Res<Option<DomainConfig>> {
        let path = self.paths.domain_config();
        read_if_exists(storage, &path)
            .await?
            .map(|bytes| DomainConfig::from_slice(&path, &bytes))
            .transpose()
    }

    /// The domain config, if it lets lineage be written: only once the
    /// domain's lineage has moved to refs, so neither that move nor its
    /// rollback meets a ref or the home changing under it. Each writer reads
    /// it again under the lock its write is made under.
    async fn writable_config(&self, storage: &(impl Storage + Sync)) -> Res<DomainConfig> {
        let config = self.read_config(storage).await?;
        let state = match config.as_ref().and_then(|config| config.migration.as_ref()) {
            Some(MigrationState::Done) => return Ok(config.expect("a config says done")),
            Some(MigrationState::InProgress) => "is in progress",
            Some(MigrationState::RollingBack { .. }) => "is being rolled back",
            None => "has not started",
        };
        Err(LineageError::RefsNotWritable {
            path: self.paths.domain_config(),
            state,
        }
        .into())
    }

    /// Takes the domain file lock, then reads the config under it.
    async fn lock_domain(&self, storage: &(impl Storage + Sync)) -> Res<(LockGuard, DomainConfig)> {
        let held = storage
            .lock_exclusive(self.paths.domain_file_lock())
            .await?;
        Ok((held, self.writable_config(storage).await?))
    }

    async fn read_ref(
        &self,
        storage: &(impl Storage + Sync),
        namespace: &Namespace,
    ) -> Res<(PackageRef, Option<TipCache>)> {
        let path = self.paths.package_ref(namespace);
        let Some(bytes) = read_if_exists(storage, &path).await? else {
            return Err(Error::InstallPackage(InstallPackageError::NotInstalled(
                namespace.clone(),
            )));
        };
        let stored = PackageRef::from_slice(&path, &bytes)?;
        Ok((stored, self.read_tip(storage, namespace).await))
    }

    /// `None` when the cache is missing or unreadable: the tip is unknown.
    async fn read_tip(
        &self,
        storage: &(impl Storage + Sync),
        namespace: &Namespace,
    ) -> Option<TipCache> {
        let bytes = storage
            .read_bytes(self.paths.package_tip(namespace))
            .await
            .ok()?;
        TipCache::from_slice(&bytes)
    }

    /// Stores `lineage` over `stored` and its `tip`. The tip cache changes
    /// before the ref only where the ref on disk cannot lose a tip by it: when
    /// there is no ref yet, or for a new tip of the stored ref's own remote.
    /// Otherwise it changes after, and a failure then is logged, since the
    /// ref has landed. Returns the lineage as it now reads when the ref
    /// changed.
    async fn write(
        &self,
        storage: &(impl Storage + Sync),
        namespace: &Namespace,
        stored: Option<PackageRef>,
        tip: Option<TipCache>,
        lineage: &PackageLineage,
    ) -> Res<Option<PackageLineage>> {
        let new_ref = PackageRef::from_lineage(lineage, stored.as_ref())?;
        let had_tip = tip.is_some();
        let (mut tip, fresh) = wanted_tip(&new_ref, lineage, tip);
        let tip_first = match &stored {
            None => true,
            Some(stored) => fresh && same_package(stored, &new_ref),
        };
        if tip_first {
            self.change_tip(storage, namespace, tip.as_ref(), fresh, had_tip)
                .await?;
        }
        // Against the stored ref as this build would write it, so a form it
        // reads but does not write, such as a copy's explicit `base`, is not
        // taken for a change.
        let changed = match &stored {
            Some(stored) => {
                let as_read = stored.clone().into_lineage(None);
                PackageRef::from_lineage(&as_read, Some(stored))? != new_ref
            }
            None => true,
        };
        if changed {
            put(
                storage,
                &self.paths.package_ref(namespace),
                new_ref.to_vec()?,
                Some("ref"),
            )
            .await?;
        }
        if !tip_first
            && let Err(err) = self
                .change_tip(storage, namespace, tip.as_ref(), fresh, had_tip)
                .await
        {
            // The tip left is one for another bucket, which the new ref ignores.
            warn!("❌ Could not cache the tip of {namespace}: {err}");
            tip = None;
        }
        Ok(changed.then(|| new_ref.into_lineage(tip.as_ref())))
    }

    /// Writes `tip` where it is `fresh`, and removes the cache where none is
    /// wanted and there `had` been one.
    async fn change_tip(
        &self,
        storage: &impl Storage,
        namespace: &Namespace,
        tip: Option<&TipCache>,
        fresh: bool,
        had: bool,
    ) -> Res {
        match tip {
            Some(tip) if fresh => {
                let path = self.paths.package_tip(namespace);
                put(storage, &path, tip.to_vec()?, None).await
            }
            None if had => {
                self.remove_tip(storage, namespace).await;
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// A tip left behind costs nothing: it is replaced or removed when the
    /// package is next written.
    async fn remove_tip(&self, storage: &impl Storage, namespace: &Namespace) {
        let path = self.paths.package_tip(namespace);
        if let Err(err) = remove_if_present(storage, &path).await {
            warn!("❌ Could not remove {}: {}", path.display(), err);
        }
    }

    /// Creates `refs/` and the owner's folder where missing, and syncs the
    /// parent of each, even one that existed: a create killed before its
    /// sync leaves a folder that a power loss can still take.
    async fn create_folders(&self, storage: &(impl Storage + Sync), namespace: &Namespace) -> Res {
        let refs = self.paths.refs_dir();
        let owner = self.paths.package_ref(namespace);
        let owner = folder_of(&owner);
        for folder in [refs.as_path(), owner] {
            if !storage.exists(folder).await {
                storage.create_dir_all(folder).await?;
                if folder == owner {
                    crate::failpoint::hit("ref.create.after-owner-dir");
                }
            }
            storage.sync_dir(folder_of(folder)).await;
        }
        Ok(())
    }

    async fn change_home(
        &self,
        storage: &(impl Storage + Sync),
        home: &Home,
        overwrite: bool,
    ) -> Res {
        let (_domain, mut config) = self.lock_domain(storage).await?;
        if config.home == *home {
            return storage.create_dir_all(home.as_ref()).await;
        }
        if !overwrite && !config.home.as_ref().as_os_str().is_empty() {
            let installed = self.namespaces(storage).await?.len();
            if installed > 0 {
                return Err(LineageError::HomeInUse {
                    home: config.home.as_ref().clone(),
                    installed,
                }
                .into());
            }
        }
        storage.create_dir_all(home.as_ref()).await?;
        config.home = home.clone();
        put(storage, &self.paths.domain_config(), config.to_vec()?, None).await
    }

    /// Each file in `refs/<owner>/` whose owner and name make a namespace.
    async fn files(&self, storage: &(impl Storage + Sync)) -> Res<Vec<RefFile>> {
        let mut files = Vec::new();
        for (owner_dir, is_dir) in entries(storage, &self.paths.refs_dir()).await? {
            let Some(owner) = owner_dir.file_name().filter(|_| is_dir) else {
                continue;
            };
            let owner = owner.to_string_lossy();
            for (path, is_dir) in entries(storage, &owner_dir).await? {
                let Some(file_name) = path.file_name().filter(|_| !is_dir) else {
                    continue;
                };
                let entry = RefEntry::classify(file_name);
                let (RefEntry::Ref(name)
                | RefEntry::Tip(name)
                | RefEntry::RefTemp(name)
                | RefEntry::TipTemp(name)) = &entry
                else {
                    continue;
                };
                if let Ok(namespace) = Namespace::try_from(format!("{owner}/{name}")) {
                    files.push(RefFile {
                        path,
                        namespace,
                        entry,
                    });
                }
            }
        }
        Ok(files)
    }
}

/// The bytes of `path`, or `None` when there is no such file.
async fn read_if_exists(storage: &(impl Storage + Sync), path: &Path) -> Res<Option<Vec<u8>>> {
    match storage.read_bytes(path).await {
        Ok(bytes) => Ok(Some(bytes)),
        Err(_) if !storage.exists(path).await => Ok(None),
        Err(err) => Err(err),
    }
}

/// Writes `bytes` over `path` through a temp named after it, so a sweep can
/// tell whose leftover it is.
async fn put(
    storage: &impl Storage,
    path: &Path,
    bytes: Vec<u8>,
    tag: Option<&'static str>,
) -> Res {
    let temp_name = RefEntry::temp_name(path.file_name().unwrap_or_default());
    storage
        .write_byte_stream_via(path, PathBuf::from(temp_name), ByteStream::from(bytes), tag)
        .await
}

/// The tip cache `lineage` wants beside `new_ref`, and whether it differs
/// from `tip`, the cache there now. A tip the ref already implies is not
/// cached afresh, so a fallback is not made a tip; but a cache of the same
/// bucket is overwritten rather than removed, so no kill leaves its old tip
/// beside the new ref.
fn wanted_tip(
    new_ref: &PackageRef,
    lineage: &PackageLineage,
    tip: Option<TipCache>,
) -> (Option<TipCache>, bool) {
    let Some(remote) = &lineage.remote_uri else {
        return (None, false);
    };
    let bucket = &remote.bucket;
    let cached = tip.as_ref().is_some_and(|tip| tip.bucket == *bucket);
    if !cached && lineage.latest_hash == untipped_latest(new_ref) {
        return (None, false);
    }
    match tip {
        Some(tip) if tip.bucket == *bucket && tip.latest_hash == lineage.latest_hash => {
            (Some(tip), false)
        }
        _ => {
            let tip = TipCache {
                format: TipCache::FORMAT,
                latest_hash: lineage.latest_hash.clone(),
                bucket: bucket.clone(),
                checked_at: Utc::now(),
            };
            (Some(tip), true)
        }
    }
}

/// Whether both refs' remotes name one package, at whatever revision.
fn same_package(a: &PackageRef, b: &PackageRef) -> bool {
    match (&a.remote, &b.remote) {
        (Some(a), Some(b)) => {
            a.origin == b.origin && a.bucket == b.bucket && a.namespace == b.namespace
        }
        (a, b) => a.is_none() && b.is_none(),
    }
}

/// The `latest_hash` a ref reads with no tip cached for its bucket, by
/// [`PackageRef::into_lineage`]'s rule.
fn untipped_latest(stored: &PackageRef) -> &str {
    match &stored.remote {
        Some(remote) if !remote.hash.is_empty() => &stored.base_hash,
        _ => "",
    }
}

/// `false` when there was no such file.
async fn remove_if_present(storage: &impl Storage, path: &Path) -> std::io::Result<bool> {
    match storage.remove_file(path).await {
        Ok(()) => Ok(true),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(err),
    }
}

fn folder_of(path: &Path) -> &Path {
    path.parent().unwrap_or(Path::new(""))
}

/// The entries of `dir` with whether each is a folder, none if it is missing.
async fn entries(storage: &(impl Storage + Sync), dir: &Path) -> Res<Vec<(PathBuf, bool)>> {
    let mut read_dir = match storage.read_dir(dir).await {
        Ok(read_dir) => read_dir,
        Err(_) if !storage.exists(dir).await => return Ok(Vec::new()),
        Err(err) => return Err(err),
    };
    let mut entries = Vec::new();
    while let Some(entry) = read_dir.next_entry().await? {
        match entry.file_type().await {
            Ok(file_type) => entries.push((entry.path(), file_type.is_dir())),
            // Renamed or removed since the folder was read.
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(err.into()),
        }
    }
    Ok(entries)
}

async fn remove_leftovers(storage: &impl Storage, paths: &[PathBuf]) -> Res {
    for path in paths {
        remove_if_present(storage, path).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::Mutex;
    use std::time::Duration;

    use test_log::test;
    use uuid::Uuid;

    use super::*;
    use crate::io::storage::LocalStorage;
    use crate::io::storage::lock::tests::OtherProcess;
    use crate::lineage::PathState;
    use crate::lineage::refs::format::Extra;
    use quilt_uri::ManifestUri;

    const BASE: &str = "1111111111111111111111111111111111111111111111111111111111111111";
    const NEXT: &str = "2222222222222222222222222222222222222222222222222222222222222222";
    const TIP: &str = "3333333333333333333333333333333333333333333333333333333333333333";

    /// The base hash the hook was told the package went from and to, and
    /// the one its ref held on disk as the hook ran.
    type Told = (Option<String>, Option<String>, Option<String>);

    /// A domain whose lineage has moved to refs, its home `<root>/home`.
    struct Domain {
        root: tempfile::TempDir,
        paths: DomainPaths,
        store: RefStore,
        storage: LocalStorage,
        home: Home,
    }

    impl Domain {
        fn new() -> Res<Self> {
            let root = tempfile::TempDir::new()?;
            let paths = DomainPaths::new(root.path().to_path_buf());
            std::fs::create_dir_all(paths.dot_quilt_dir())?;
            let domain = Self {
                home: Home::from(root.path().join("home")),
                store: RefStore::new(paths.clone()),
                storage: LocalStorage::new(),
                paths,
                root,
            };
            domain.write_config(&domain.home, Some(MigrationState::Done))?;
            Ok(domain)
        }

        /// A domain whose hook notes each change it is told of.
        fn hooked() -> Res<(Self, Arc<Mutex<Vec<Told>>>)> {
            let mut domain = Self::new()?;
            let told = Arc::new(Mutex::new(Vec::new()));
            let (paths, notes) = (domain.paths.clone(), Arc::clone(&told));
            let hook = LineageHook::new(move |namespace, was, is| {
                let on_disk = std::fs::read(paths.package_ref(namespace))
                    .ok()
                    .and_then(|bytes| PackageRef::from_slice(Path::new(""), &bytes).ok())
                    .map(|stored| stored.base_hash);
                let base = |lineage: Option<&PackageLineage>| lineage.map(|l| l.base_hash.clone());
                notes.lock().unwrap().push((base(was), base(is), on_disk));
            });
            domain.store = RefStore::new(domain.paths.clone()).with_hook(hook);
            Ok((domain, told))
        }

        fn write_config(&self, home: &Home, migration: Option<MigrationState>) -> Res {
            let config = DomainConfig {
                format: DomainConfig::FORMAT,
                home: home.clone(),
                migration,
                extra: Extra::new(),
            };
            Ok(std::fs::write(
                self.paths.domain_config(),
                config.to_vec()?,
            )?)
        }

        async fn lock(&self, namespace: &Namespace) -> Res<PackageLock> {
            PackageLock::take(&self.storage, &self.paths, namespace).await
        }

        async fn install(&self, lineage: &PackageLineage) -> Res {
            let namespace = &lineage.remote_uri.as_ref().unwrap().namespace;
            let held = self.lock(namespace).await?;
            self.store
                .create(&self.storage, &held, lineage, &self.home)
                .await
        }

        async fn base_hash(&self, namespace: &Namespace) -> Res<String> {
            Ok(self.store.read(&self.storage, namespace).await?.base_hash)
        }
    }

    fn package(name: &str) -> Namespace {
        ("acme", name).into()
    }

    /// `name`'s lineage, pushed at `base`, with `latest` its remote's tip.
    fn lineage(name: &str, base: &str, latest: &str) -> PackageLineage {
        PackageLineage {
            remote_uri: Some(ManifestUri {
                origin: None,
                bucket: "b".to_string(),
                namespace: package(name),
                hash: base.to_string(),
            }),
            base_hash: base.to_string(),
            latest_hash: latest.to_string(),
            paths: BTreeMap::from([(PathBuf::from("a.csv"), PathState::default())]),
            ..PackageLineage::default()
        }
    }

    /// The name a killed write of `file_name` leaves its temp under.
    fn leftover(file_name: &str) -> String {
        format!("{file_name}.tmp-{}", Uuid::new_v4())
    }

    #[test(tokio::test)]
    async fn a_ref_that_cannot_be_read_costs_only_its_package() -> Res {
        let domain = Domain::new()?;
        domain.install(&lineage("good", BASE, BASE)).await?;
        domain.install(&lineage("bad", BASE, BASE)).await?;
        domain.install(&lineage("copies", BASE, BASE)).await?;
        let bad = domain.paths.package_ref(&package("bad"));
        std::fs::write(&bad, b"{")?;
        let copies = domain.paths.package_ref(&package("copies"));
        let copy = serde_json::json!({"sync_scope": "entire_package", "tracked": []});
        let two_copies = serde_json::json!({"format": 1, "copies": [copy, copy]});
        std::fs::write(&copies, serde_json::to_vec(&two_copies)?)?;

        let all = domain.store.read_all(&domain.storage).await;
        assert!(all.is_ok(), "{all:?}");
        let all = all?;
        assert_eq!(
            all.keys().collect::<Vec<_>>(),
            [&package("bad"), &package("copies"), &package("good")]
        );
        assert!(
            matches!(
                &all[&package("copies")],
                Err(Error::Lineage(LineageError::UnsupportedCopies { path, .. })) if *path == copies
            ),
            "{:?}",
            all[&package("copies")]
        );
        let held = domain.lock(&package("copies")).await?;
        let removed = domain.store.remove(&domain.storage, &held).await;
        assert!(removed.is_err(), "a state this build does not support");
        assert!(copies.exists());
        assert_eq!(
            all[&package("good")].as_ref().ok(),
            Some(&lineage("good", BASE, BASE))
        );
        assert!(
            matches!(
                &all[&package("bad")],
                Err(Error::Lineage(LineageError::FileParse { path, .. })) if *path == bad
            ),
            "{:?}",
            all[&package("bad")]
        );
        assert!(
            domain
                .store
                .contains(&domain.storage, &package("bad"))
                .await
        );
        Ok(())
    }

    #[test(tokio::test)]
    async fn a_package_without_a_ref_is_not_installed() -> Res {
        let domain = Domain::new()?;
        let missing = package("missing");
        let not_installed = |result: &Res| {
            matches!(
                result,
                Err(Error::InstallPackage(InstallPackageError::NotInstalled(namespace)))
                    if *namespace == missing
            )
        };

        assert!(!domain.store.contains(&domain.storage, &missing).await);
        let read = domain.store.read(&domain.storage, &missing).await.map(drop);
        assert!(not_installed(&read), "{read:?}");
        let held = domain.lock(&missing).await?;
        let updated = domain
            .store
            .update(&domain.storage, &held, |_| Ok(()))
            .await;
        assert!(not_installed(&updated), "{updated:?}");
        let removed = domain.store.remove(&domain.storage, &held).await;
        assert!(not_installed(&removed), "{removed:?}");
        Ok(())
    }

    /// Package `a` is mid-create: it holds its own lock and the domain file
    /// lock. An update of `b` waits on neither.
    #[test(tokio::test)]
    async fn an_update_does_not_wait_on_another_package_being_created() -> Res {
        let domain = Domain::new()?;
        domain.install(&lineage("b", BASE, BASE)).await?;
        let _creating_a = domain.lock(&package("a")).await?;
        let _domain_file_lock = domain
            .storage
            .lock_exclusive(domain.paths.domain_file_lock())
            .await?;

        let held = domain.lock(&package("b")).await?;
        let update = domain.store.update(&domain.storage, &held, |lineage| {
            lineage.base_hash = NEXT.to_string();
            Ok(())
        });
        tokio::time::timeout(Duration::from_secs(5), update)
            .await
            .expect("the update does not wait")?;
        assert_eq!(domain.base_hash(&package("b")).await?, NEXT);
        Ok(())
    }

    /// The other order, a create before the home change, is refused by the
    /// home change: see the test below.
    #[test(tokio::test)]
    async fn a_create_after_the_home_moved_records_nothing() -> Res {
        let domain = Domain::new()?;
        let placed = domain.store.home(&domain.storage).await?;
        let moved = Home::from(domain.root.path().join("moved"));
        domain.store.set_home(&domain.storage, &moved).await?;

        let created = domain.install(&lineage("x", BASE, BASE)).await;
        assert!(
            matches!(
                &created,
                Err(Error::Lineage(LineageError::HomeChanged { placed: was, home }))
                    if was == placed.as_ref() && home == moved.as_ref()
            ),
            "{created:?}"
        );
        assert!(!domain.store.contains(&domain.storage, &package("x")).await);
        Ok(())
    }

    #[test(tokio::test)]
    async fn the_home_moves_while_a_ref_needs_it_only_when_forced_or_missing() -> Res {
        let domain = Domain::new()?;
        domain.install(&lineage("x", BASE, BASE)).await?;
        let moved = Home::from(domain.root.path().join("moved"));

        let refused = domain.store.set_home(&domain.storage, &moved).await;
        assert!(
            matches!(
                refused,
                Err(Error::Lineage(LineageError::HomeInUse { installed: 1, .. }))
            ),
            "{refused:?}"
        );
        assert_eq!(domain.store.home(&domain.storage).await?, domain.home);

        domain.store.overwrite_home(&domain.storage, &moved).await?;
        assert_eq!(domain.store.home(&domain.storage).await?, moved);
        assert!(moved.as_ref().is_dir());

        domain.write_config(&Home::default(), Some(MigrationState::Done))?;
        let repaired = domain.store.set_home(&domain.storage, &domain.home).await;
        assert!(repaired.is_ok(), "{repaired:?}");
        assert_eq!(domain.store.home(&domain.storage).await?, domain.home);
        Ok(())
    }

    #[test(tokio::test)]
    async fn leftover_temps_are_skipped_and_swept_only_while_their_lock_is_free() -> Res {
        let domain = Domain::new()?;
        domain.install(&lineage("a", BASE, BASE)).await?;
        domain.install(&lineage("b", BASE, BASE)).await?;
        let owner = domain.paths.refs_dir().join("acme");
        let free_temps = [
            owner.join(leftover("a.json")),
            owner.join(leftover("a.json.latest")),
            // A killed create's: `c` has no ref.
            owner.join(leftover("c.json")),
        ];
        let b_temp = owner.join(leftover("b.json"));
        let config_temp = domain.paths.dot_quilt_dir().join(leftover("domain.json"));
        for path in free_temps.iter().chain([&b_temp, &config_temp]) {
            std::fs::write(path, b"{")?;
        }

        assert_eq!(
            domain.store.namespaces(&domain.storage).await?,
            BTreeSet::from([package("a"), package("b")])
        );

        let held_b = domain.lock(&package("b")).await?;
        let held_domain = domain
            .storage
            .lock_exclusive(domain.paths.domain_file_lock())
            .await?;
        domain.store.sweep(&domain.storage).await?;
        assert!(
            free_temps.iter().all(|path| !path.exists()),
            "a's and c's locks are free"
        );
        assert!(b_temp.exists(), "b's lock is held");
        assert!(config_temp.exists(), "the domain file lock is held");

        drop((held_b, held_domain));
        domain.store.sweep(&domain.storage).await?;
        assert!(!b_temp.exists());
        assert!(!config_temp.exists());
        assert_eq!(
            domain.store.namespaces(&domain.storage).await?,
            BTreeSet::from([package("a"), package("b")])
        );
        Ok(())
    }

    #[test(tokio::test)]
    async fn the_hook_is_told_once_of_each_ref_change_after_it_lands() -> Res {
        let (domain, told) = Domain::hooked()?;
        domain.install(&lineage("x", BASE, BASE)).await?;
        let held = domain.lock(&package("x")).await?;
        let update = |step: fn(&mut PackageLineage)| {
            domain.store.update(&domain.storage, &held, move |lineage| {
                step(lineage);
                Ok(())
            })
        };
        update(|lineage| lineage.base_hash = NEXT.to_string()).await?;
        update(|lineage| lineage.latest_hash = TIP.to_string()).await?;
        update(|_| {}).await?;
        assert_eq!(
            domain
                .store
                .read(&domain.storage, &package("x"))
                .await?
                .latest_hash,
            TIP,
            "the tip alone was stored, untold"
        );
        domain.store.remove(&domain.storage, &held).await?;

        let some = |hash: &str| Some(hash.to_string());
        assert_eq!(
            *told.lock().unwrap(),
            [
                (None, some(BASE), some(BASE)),
                (some(BASE), some(NEXT), some(NEXT)),
                (some(NEXT), None, None),
            ]
        );
        Ok(())
    }

    /// A folder that cannot be synced is made unreadable, before each write
    /// that syncs it: `.quilt` as `refs/` is created, `refs/` as an owner's
    /// folder is created or reused, and the owner's folder as a ref is
    /// removed from it.
    #[cfg(unix)]
    #[test(tokio::test)]
    async fn a_folder_sync_that_fails_is_logged_and_the_write_stands() -> Res {
        use std::os::unix::fs::PermissionsExt;

        use crate::io::storage::Logged;

        let set_mode = |path: &Path, mode| {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
        };
        let domain = Domain::new()?;
        let dot_quilt = domain.paths.dot_quilt_dir();
        let held_x = domain.lock(&package("x")).await?;
        let held_y = domain.lock(&("other", "y").into()).await?;
        set_mode(&dot_quilt, 0o300)?;
        if std::fs::File::open(&dot_quilt).is_ok() {
            // Root ignores the mode, so the sync would not fail.
            return Ok(set_mode(&dot_quilt, 0o700)?);
        }

        let (created, logged) = Logged::during(domain.store.create(
            &domain.storage,
            &held_x,
            &lineage("x", BASE, BASE),
            &domain.home,
        ))
        .await;
        set_mode(&dot_quilt, 0o700)?;
        created?;
        assert_eq!(logged, [tracing::Level::WARN], "creating refs/");

        let refs = domain.paths.refs_dir();
        set_mode(&refs, 0o300)?;
        let (created, logged) = Logged::during(domain.store.create(
            &domain.storage,
            &held_y,
            &lineage("y", BASE, BASE),
            &domain.home,
        ))
        .await;
        set_mode(&refs, 0o700)?;
        created?;
        assert_eq!(logged, [tracing::Level::WARN], "creating refs/other/");

        let held_z = domain.lock(&package("z")).await?;
        set_mode(&refs, 0o300)?;
        let (created, logged) = Logged::during(domain.store.create(
            &domain.storage,
            &held_z,
            &lineage("z", BASE, BASE),
            &domain.home,
        ))
        .await;
        set_mode(&refs, 0o700)?;
        created?;
        assert_eq!(logged, [tracing::Level::WARN], "refs/acme/ already there");

        let owner = refs.join("acme");
        set_mode(&owner, 0o300)?;
        let (removed, logged) = Logged::during(domain.store.remove(&domain.storage, &held_x)).await;
        set_mode(&owner, 0o700)?;
        removed?;
        assert_eq!(logged, [tracing::Level::WARN], "removing acme/x.json");

        assert_eq!(
            domain.store.namespaces(&domain.storage).await?,
            BTreeSet::from([("other", "y").into(), package("z")])
        );
        Ok(())
    }

    #[test(tokio::test)]
    async fn a_ref_in_a_newer_format_is_neither_rewritten_nor_removed() -> Res {
        let domain = Domain::new()?;
        domain.install(&lineage("x", BASE, BASE)).await?;
        let path = domain.paths.package_ref(&package("x"));
        let newer = br#"{"format": 2}"#;
        std::fs::write(&path, newer)?;
        let refused = |result: &Res| {
            matches!(
                result,
                Err(Error::Lineage(LineageError::UnsupportedFormat {
                    found: 2,
                    ..
                }))
            )
        };

        let held = domain.lock(&package("x")).await?;
        let updated = domain
            .store
            .update(&domain.storage, &held, |lineage| {
                lineage.base_hash = NEXT.to_string();
                Ok(())
            })
            .await;
        assert!(refused(&updated), "{updated:?}");
        let removed = domain.store.remove(&domain.storage, &held).await;
        assert!(refused(&removed), "{removed:?}");
        assert_eq!(std::fs::read(&path)?, newer);
        Ok(())
    }

    #[test(tokio::test)]
    async fn nothing_is_written_unless_the_move_to_refs_is_done() -> Res {
        for (migration, state) in [
            (None, "has not started"),
            (Some(None), "has not started"),
            (Some(Some(MigrationState::InProgress)), "is in progress"),
            (
                Some(Some(MigrationState::RollingBack {
                    archive: "refs.rolled-back-x".to_string(),
                })),
                "is being rolled back",
            ),
        ] {
            let domain = Domain::new()?;
            domain.install(&lineage("x", BASE, BASE)).await?;
            let x_ref = std::fs::read(domain.paths.package_ref(&package("x")))?;
            match migration {
                Some(migration) => domain.write_config(&domain.home, migration)?,
                None => std::fs::remove_file(domain.paths.domain_config())?,
            }
            let config = std::fs::read(domain.paths.domain_config()).ok();
            let refused = |result: Res| {
                matches!(
                    result,
                    Err(Error::Lineage(LineageError::RefsNotWritable { state: found, .. }))
                        if found == state
                )
            };

            let (held_x, held_y) = (
                domain.lock(&package("x")).await?,
                domain.lock(&package("y")).await?,
            );
            let moved = Home::from(domain.root.path().join("moved"));
            let store = &domain.store;
            let storage = &domain.storage;
            let updated = store.update(storage, &held_x, |lineage| {
                lineage.base_hash = NEXT.to_string();
                Ok(())
            });
            assert!(refused(updated.await), "update, {state}");
            let y = lineage("y", BASE, BASE);
            let created = store.create(storage, &held_y, &y, &domain.home);
            assert!(refused(created.await), "create, {state}");
            assert!(
                refused(store.remove(storage, &held_x).await),
                "remove, {state}"
            );
            assert!(
                refused(store.set_home(storage, &moved).await),
                "set_home, {state}"
            );
            assert!(
                refused(store.overwrite_home(storage, &moved).await),
                "overwrite_home, {state}"
            );

            assert_eq!(
                std::fs::read(domain.paths.package_ref(&package("x")))?,
                x_ref
            );
            assert!(!store.contains(storage, &package("y")).await);
            assert_eq!(std::fs::read(domain.paths.domain_config()).ok(), config);
            assert!(!moved.as_ref().exists());
        }
        Ok(())
    }

    #[test(tokio::test)]
    async fn a_ref_that_does_not_parse_can_be_removed() -> Res {
        let (domain, told) = Domain::hooked()?;
        domain.install(&lineage("x", BASE, TIP)).await?;
        let tip = domain.paths.package_tip(&package("x"));
        assert!(tip.exists());
        std::fs::write(domain.paths.package_ref(&package("x")), b"{")?;

        let held = domain.lock(&package("x")).await?;
        let removed = domain.store.remove(&domain.storage, &held).await;
        assert!(removed.is_ok(), "{removed:?}");
        assert!(!domain.store.contains(&domain.storage, &package("x")).await);
        assert!(!tip.exists());
        assert_eq!(told.lock().unwrap().last(), Some(&(None, None, None)));
        Ok(())
    }

    #[test(tokio::test)]
    async fn writers_wait_for_another_process_holding_the_domain_file_lock() -> Res {
        let domain = Domain::new()?;
        domain.install(&lineage("y", BASE, BASE)).await?;
        let (held_x, held_y) = (
            domain.lock(&package("x")).await?,
            domain.lock(&package("y")).await?,
        );
        let moved = Home::from(domain.root.path().join("moved"));
        let x = lineage("x", BASE, BASE);
        let other = OtherProcess::holding(&domain.paths.domain_file_lock()).await?;

        let waits = Duration::from_millis(300);
        let storage = &domain.storage;
        let create = domain.store.create(storage, &held_x, &x, &domain.home);
        assert!(tokio::time::timeout(waits, create).await.is_err(), "create");
        let remove = domain.store.remove(storage, &held_y);
        assert!(tokio::time::timeout(waits, remove).await.is_err(), "remove");
        let set_home = domain.store.set_home(storage, &moved);
        assert!(
            tokio::time::timeout(waits, set_home).await.is_err(),
            "set_home"
        );

        other.release()?;
        let create = domain.store.create(storage, &held_x, &x, &domain.home);
        tokio::time::timeout(Duration::from_secs(30), create)
            .await
            .expect("the lock is free once the other process lets go")?;
        assert!(domain.store.contains(storage, &package("x")).await);
        Ok(())
    }

    #[test(tokio::test)]
    async fn a_tip_read_from_another_bucket_is_never_taken_for_this_one() -> Res {
        let domain = Domain::new()?;
        domain.install(&lineage("x", BASE, TIP)).await?;
        assert_eq!(
            domain
                .store
                .read(&domain.storage, &package("x"))
                .await?
                .latest_hash,
            TIP
        );
        let held = domain.lock(&package("x")).await?;
        let to_bucket = |bucket: &'static str| {
            domain.store.update(&domain.storage, &held, move |lineage| {
                lineage.remote_uri.as_mut().unwrap().bucket = bucket.to_string();
                lineage.latest_hash = BASE.to_string();
                Ok(())
            })
        };

        to_bucket("c").await?;
        to_bucket("b").await?;
        assert_eq!(
            domain
                .store
                .read(&domain.storage, &package("x"))
                .await?
                .latest_hash,
            BASE
        );
        assert!(!domain.paths.package_tip(&package("x")).exists());
        Ok(())
    }

    /// A copy that names its base though it is the head's reads as it would
    /// be written without it, so a change to the tip alone leaves it be.
    #[test(tokio::test)]
    async fn a_ref_in_a_form_this_build_does_not_write_is_not_rewritten_for_its_tip() -> Res {
        let (domain, told) = Domain::hooked()?;
        domain.install(&lineage("x", BASE, BASE)).await?;
        let path = domain.paths.package_ref(&package("x"));
        let mut stored: serde_json::Value = serde_json::from_slice(&std::fs::read(&path)?)?;
        stored["copies"][0]["base"] = serde_json::json!(BASE);
        let stored = serde_json::to_vec(&stored)?;
        std::fs::write(&path, &stored)?;

        let held = domain.lock(&package("x")).await?;
        domain
            .store
            .update(&domain.storage, &held, |lineage| {
                lineage.latest_hash = TIP.to_string();
                Ok(())
            })
            .await?;
        assert_eq!(std::fs::read(&path)?, stored);
        assert_eq!(told.lock().unwrap().len(), 1, "the install alone");
        let x = domain.store.read(&domain.storage, &package("x")).await?;
        assert_eq!(x.latest_hash, TIP);
        Ok(())
    }

    /// A move to another bucket writes its ref before the new bucket's tip,
    /// so a tip that then cannot be cached fails nothing.
    #[test(tokio::test)]
    async fn a_tip_that_cannot_be_cached_after_the_ref_lands_fails_nothing() -> Res {
        let (domain, told) = Domain::hooked()?;
        domain.install(&lineage("x", BASE, BASE)).await?;
        let tip = domain.paths.package_tip(&package("x"));
        std::fs::create_dir(&tip)?;
        std::fs::write(tip.join("in-the-way"), b"")?;

        let held = domain.lock(&package("x")).await?;
        let moved = domain
            .store
            .update(&domain.storage, &held, |lineage| {
                lineage.remote_uri.as_mut().unwrap().bucket = "c".to_string();
                lineage.latest_hash = NEXT.to_string();
                Ok(())
            })
            .await;
        assert!(moved.is_ok(), "{moved:?}");
        let x = domain.store.read(&domain.storage, &package("x")).await?;
        assert_eq!(x.remote_uri.unwrap().bucket, "c");
        assert_eq!(x.latest_hash, BASE, "the tip is unknown");
        assert_eq!(told.lock().unwrap().len(), 2, "the install and the move");
        Ok(())
    }

    #[test(tokio::test)]
    async fn a_package_already_recorded_is_not_created_again() -> Res {
        let domain = Domain::new()?;
        domain.install(&lineage("x", BASE, BASE)).await?;
        let again = domain.install(&lineage("x", NEXT, NEXT)).await;
        assert!(
            matches!(
                again,
                Err(Error::InstallPackage(
                    InstallPackageError::AlreadyInstalled(_)
                ))
            ),
            "{again:?}"
        );
        assert_eq!(domain.base_hash(&package("x")).await?, BASE);
        Ok(())
    }

    /// Updates killed with nothing unwinding, by a child process.
    #[cfg(unix)]
    mod killed {
        use super::*;

        use test_log::test;

        const CHILD_ROOT: &str = "QUILT_RS_TEST_REF_STORE_ROOT";
        const CHILD_UPDATE: &str = "QUILT_RS_TEST_REF_STORE_UPDATE";

        /// Not a test on its own: the child half of the tests below. It
        /// updates `x` as `QUILT_RS_TEST_REF_STORE_UPDATE` says: `pull` moves
        /// its base to the tip it had cached, `pull-behind` does too but finds
        /// the tip moved on to `NEXT`, `push` makes `NEXT` its base and its
        /// remote's latest, `move` points it at another bucket
        /// whose tip is `NEXT`, `rename` points it at another package in its
        /// bucket, at `TIP` with tip `BASE`, and `create` records it afresh at
        /// `NEXT`.
        #[test]
        #[ignore = "run only as the child of killed_before_the_ref_lands"]
        fn child_updates() -> Res {
            let (Ok(root), Ok(update)) = (std::env::var(CHILD_ROOT), std::env::var(CHILD_UPDATE))
            else {
                return Ok(());
            };
            tokio::runtime::Runtime::new()?.block_on(async {
                let paths = DomainPaths::new(PathBuf::from(root));
                let storage = LocalStorage::new();
                let held = PackageLock::take(&storage, &paths, &package("x")).await?;
                let store = RefStore::new(paths);
                if update == "create" {
                    let home = store.home(&storage).await?;
                    let x = lineage("x", NEXT, NEXT);
                    return store.create(&storage, &held, &x, &home).await;
                }
                store
                    .update(&storage, &held, |lineage| {
                        let remote = lineage.remote_uri.as_mut().unwrap();
                        match update.as_str() {
                            "push" => {
                                remote.hash = NEXT.to_string();
                                lineage.base_hash = NEXT.to_string();
                                lineage.latest_hash = NEXT.to_string();
                            }
                            "rename" => {
                                remote.namespace = package("renamed");
                                remote.hash = TIP.to_string();
                                lineage.base_hash = TIP.to_string();
                                lineage.latest_hash = BASE.to_string();
                            }
                            "pull" | "pull-behind" => {
                                remote.hash = TIP.to_string();
                                lineage.base_hash = TIP.to_string();
                                lineage.latest_hash =
                                    if update == "pull" { TIP } else { NEXT }.to_string();
                            }
                            _ => {
                                remote.bucket = "c".to_string();
                                lineage.latest_hash = NEXT.to_string();
                            }
                        }
                        Ok(())
                    })
                    .await
            })
        }

        /// Installs `x` behind its remote, has a child run `update` on it and
        /// kills the child at the failpoint `name`, and returns `x` as it
        /// then reads.
        async fn killed_at(update: &str, name: &str) -> Res<PackageLineage> {
            let domain = Domain::new()?;
            domain.install(&lineage("x", BASE, TIP)).await?;
            if update == "create" {
                // A removal killed after the unlink leaves the tip behind.
                std::fs::remove_file(domain.paths.package_ref(&package("x")))?;
            }
            let signals = tempfile::TempDir::new()?;
            let mut child = std::process::Command::new(std::env::current_exe()?)
                .args([
                    "--exact",
                    "lineage::refs::store::tests::killed::child_updates",
                    "--ignored",
                    "--nocapture",
                ])
                .env(CHILD_ROOT, domain.root.path())
                .env(CHILD_UPDATE, update)
                .env(crate::failpoint::NAME_VAR, name)
                .env(crate::failpoint::DIR_VAR, signals.path())
                .spawn()?;

            let reached = signals.path().join(format!("{name}.reached"));
            let deadline = std::time::Instant::now() + Duration::from_secs(30);
            let mut exited = None;
            while !reached.exists() && exited.is_none() && std::time::Instant::now() < deadline {
                exited = child.try_wait()?;
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            let _ = child.kill();
            child.wait()?;
            assert!(reached.exists(), "the child exited with {exited:?}");
            domain.store.read(&domain.storage, &package("x")).await
        }

        /// The pull no longer needs the cached tip, which is removed only
        /// once the new ref is in place.
        #[test(tokio::test)]
        async fn a_pull_killed_before_its_ref_lands_still_reads_as_behind() -> Res {
            let lineage = killed_at("pull", "ref.before-temp").await?;
            assert_eq!(
                (lineage.base_hash.as_str(), lineage.latest_hash.as_str()),
                (BASE, TIP)
            );
            Ok(())
        }

        /// The new bucket's tip is cached only once the ref names that bucket.
        #[test(tokio::test)]
        async fn a_move_killed_before_its_ref_lands_still_reads_as_behind() -> Res {
            let lineage = killed_at("move", "ref.before-temp").await?;
            assert_eq!(
                (lineage.base_hash.as_str(), lineage.latest_hash.as_str()),
                (BASE, TIP)
            );
            Ok(())
        }

        /// The cached tip is overwritten before the ref lands, not removed
        /// after, so the new ref never reads the old tip.
        #[test(tokio::test)]
        async fn a_push_killed_after_its_ref_lands_reads_up_to_date() -> Res {
            let lineage = killed_at("push", "ref.after-rename").await?;
            assert_eq!(
                (lineage.base_hash.as_str(), lineage.latest_hash.as_str()),
                (NEXT, NEXT)
            );
            Ok(())
        }

        /// Another package's tip is cached only once the ref names it, even in
        /// the same bucket.
        #[test(tokio::test)]
        async fn a_rename_killed_before_its_ref_lands_still_reads_as_behind() -> Res {
            let lineage = killed_at("rename", "ref.before-temp").await?;
            assert_eq!(
                (lineage.base_hash.as_str(), lineage.latest_hash.as_str()),
                (BASE, TIP)
            );
            Ok(())
        }

        /// A tip an earlier install left is gone before the new ref lands.
        #[test(tokio::test)]
        async fn a_create_killed_after_its_ref_lands_reads_no_earlier_tip() -> Res {
            let lineage = killed_at("create", "ref.after-rename").await?;
            assert_eq!(
                (lineage.base_hash.as_str(), lineage.latest_hash.as_str()),
                (NEXT, NEXT)
            );
            Ok(())
        }

        /// A tip the new ref's bucket shares is cached before the ref lands.
        #[test(tokio::test)]
        async fn a_pull_that_finds_a_newer_tip_killed_after_its_ref_lands_reads_as_behind() -> Res {
            let lineage = killed_at("pull-behind", "ref.after-rename").await?;
            assert_eq!(
                (lineage.base_hash.as_str(), lineage.latest_hash.as_str()),
                (TIP, NEXT)
            );
            Ok(())
        }
    }
}
