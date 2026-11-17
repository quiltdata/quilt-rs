//!
//! A domain's lineage: each installed package's origin, commit and tracked
//! paths, kept in a ref per package under `.quilt/refs/`, beside the home in
//! `.quilt/domain.json`.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use tracing::log;

use crate::Error;
use crate::InstallPackageError;
use crate::Res;
use crate::error::LineageError;
use crate::io::storage::Storage;
use crate::package_lock::PackageLock;
use crate::paths;
use quilt_uri::Namespace;

mod status;
pub use status::Change;
pub use status::ChangeSet;
pub use status::InstalledPackageStatus;
pub use status::UpstreamState;
pub use status::WorkingCopy;

mod state;
pub use state::PackageState;
pub use state::PackageStateReport;

mod package;
pub use package::CommitState;
pub use package::LineagePaths;
pub use package::PackageLineage;
pub use package::PathState;
pub use package::SyncScope;

mod downloads;
pub use downloads::DownloadCounts;
pub use downloads::DownloadSize;

mod home;
pub use home::DEFAULT_HOME_DIR_NAME;
pub use home::Home;

mod legacy;
pub use legacy::DomainLineage;

pub(crate) mod refs;

/// A step for one package's entry that applies `edit`, refusing a package
/// that is not installed.
fn edit_entry<'a>(
    namespace: &'a Namespace,
    edit: impl FnOnce(&mut PackageLineage) + 'a,
) -> impl FnOnce(&mut Option<PackageLineage>) -> Res<PackageLineage> + 'a {
    move |entry| {
        let package = entry.as_mut().ok_or_else(|| {
            Error::InstallPackage(InstallPackageError::NotInstalled(namespace.clone()))
        })?;
        edit(package);
        Ok(package.clone())
    }
}

type HookFn = dyn Fn(&Namespace, Option<&PackageLineage>, Option<&PackageLineage>) + Send + Sync;

/// Told of each package entry a lineage write changed: the entry as the
/// write found it and as it left it, `None` when not installed.
///
/// It runs under the lock the write took, after the write landed, so it must
/// return at once. It sees only this process's writes: another process, the quilt
/// CLI among them, writes without it.
#[derive(Clone)]
pub struct LineageHook(Arc<HookFn>);

impl LineageHook {
    pub fn new(
        hook: impl Fn(&Namespace, Option<&PackageLineage>, Option<&PackageLineage>)
        + Send
        + Sync
        + 'static,
    ) -> Self {
        Self(Arc::new(hook))
    }
}

impl std::fmt::Debug for LineageHook {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LineageHook")
    }
}

impl PartialEq for LineageHook {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for LineageHook {}

/// Where a domain keeps its packages' lineage.
#[cfg(any(test, feature = "testing"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineageStore {
    /// `.quilt/data.json`, every package's lineage in one file, as a build
    /// from before refs keeps it.
    DataJson,
    /// A ref per package under `.quilt/refs/`.
    Refs,
}

/// Reads and writes the lineage of a domain's packages, a ref per package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DomainLineageIo {
    refs: refs::RefStore,
    ready: Ready,
    /// The domain's `data.json` is read and written instead, as an older
    /// build does.
    #[cfg(any(test, feature = "testing"))]
    on_data_json: bool,
}

/// Whether a handle or one of its clones has found its domain on refs; see
/// [`DomainLineageIo::ready`].
#[derive(Clone, Debug, Default)]
struct Ready(Arc<tokio::sync::OnceCell<()>>);

/// What a handle has seen is no part of which domain it is.
impl PartialEq for Ready {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl Eq for Ready {}

impl DomainLineageIo {
    /// The domain at `paths`, its lineage moved there from `data.json` on
    /// first use: see [`Self::ready`].
    #[must_use]
    pub(crate) fn new(paths: paths::DomainPaths) -> Self {
        DomainLineageIo {
            refs: refs::RefStore::new(paths),
            ready: Ready::default(),
            #[cfg(any(test, feature = "testing"))]
            on_data_json: false,
        }
    }

    /// Moves the lineage to refs from `data.json` until this handle or a
    /// clone first finds it there, one at a time, so a running app never
    /// undoes a rollback; an interrupted one is [`LineageError::RollingBack`].
    /// The move takes package locks: the caller holds none.
    async fn ready(&self, storage: &(impl Storage + Sync)) -> Res {
        #[cfg(any(test, feature = "testing"))]
        if self.on_data_json {
            return Ok(());
        }
        let paths = self.refs.paths();
        self.ready
            .0
            .get_or_try_init(|| async {
                match refs::state::detect(storage, paths).await? {
                    refs::state::DomainState::Refs => {}
                    // Waiting for the rollback's lock would move the
                    // domain back once it finished.
                    refs::state::DomainState::RollingBack { .. } => {
                        return Err(LineageError::RollingBack {
                            path: paths.domain_config(),
                        }
                        .into());
                    }
                    _ => {
                        let migrated = refs::migrate::up(storage, paths).await?;
                        log::info!(
                            "Moved the lineage of {} packages to refs, leaving out {}",
                            migrated.written.len(),
                            migrated.skipped.len()
                        );
                    }
                }
                Ok::<_, Error>(())
            })
            .await?;
        Ok(())
    }

    /// The package's lock, for a writer: [`Self::ready`] runs first, since
    /// the move takes every package's lock and would wait on the writer's.
    pub(crate) async fn lock_package(
        &self,
        storage: &(impl Storage + Sync),
        paths: &paths::DomainPaths,
        namespace: &Namespace,
    ) -> Res<PackageLock> {
        self.ready(storage).await?;
        PackageLock::take(storage, paths, namespace).await
    }

    /// [`Self::lock_package`] without waiting for the package's lock: `None`
    /// while another writer holds it. A move on first use still waits.
    pub(crate) async fn try_lock_package(
        &self,
        storage: &(impl Storage + Sync),
        paths: &paths::DomainPaths,
        namespace: &Namespace,
    ) -> Res<Option<PackageLock>> {
        self.ready(storage).await?;
        PackageLock::try_take(storage, paths, namespace).await
    }

    /// This handle, with its hook, on the domain's `data.json`.
    #[cfg(any(test, feature = "testing"))]
    #[must_use]
    pub(crate) fn into_data_json(self) -> Self {
        Self {
            on_data_json: true,
            ..self
        }
    }

    /// The domain's `data.json`, when this handle is on it.
    #[cfg(any(test, feature = "testing"))]
    fn data_json(&self) -> Option<legacy::DataJson<'_>> {
        self.on_data_json
            .then(|| legacy::DataJson::new(self.refs.paths(), self.refs.hook()))
    }

    /// Tell `hook` of every package entry this handle's writes change.
    #[must_use]
    pub fn with_hook(self, hook: LineageHook) -> Self {
        Self {
            refs: self.refs.with_hook(hook),
            ..self
        }
    }

    /// Show `look` each installed package's entry under the lock its writes
    /// take, so none lands while `look` runs, then hand `finish` every
    /// installed package's namespace before that lock is released.
    ///
    /// Each package is read under its own lock, and one whose lock is busy,
    /// or whose ref can't be read, is not shown to `look`. `finish` runs
    /// after the last, under the lock that installing and uninstalling take,
    /// with every installed package, or with `None` when that lock is busy or
    /// the packages can't be listed.
    pub async fn check_each<T>(
        &self,
        storage: &(impl Storage + Sync),
        look: impl FnMut(&Namespace, &PackageLineage),
        finish: impl FnOnce(Option<&BTreeSet<Namespace>>) -> T,
    ) -> Res<T> {
        #[cfg(any(test, feature = "testing"))]
        if let Some(legacy) = self.data_json() {
            return legacy.check_each(storage, look, finish).await;
        }
        self.ready(storage).await?;
        self.refs.check_each(storage, look, finish).await
    }

    pub async fn home(&self, storage: &(impl Storage + Sync)) -> Res<Home> {
        #[cfg(any(test, feature = "testing"))]
        if let Some(legacy) = self.data_json() {
            return legacy.home(storage).await;
        }
        self.ready(storage).await?;
        let home = self.refs.home(storage).await?;
        if home.as_ref().as_os_str().is_empty() {
            return Err(LineageError::MissingHome.into());
        }
        Ok(home)
    }

    /// The stored home, or `None` when the domain has none yet. A record that
    /// exists but can't be read, such as for lack of permission, is an error.
    /// It only reads: a domain not yet moved to refs is not moved.
    pub async fn stored_home(&self, storage: &(impl Storage + Sync)) -> Res<Option<Home>> {
        use refs::state::DomainState;

        #[cfg(any(test, feature = "testing"))]
        if let Some(legacy) = self.data_json() {
            return legacy.stored_home(storage).await;
        }
        let paths = self.refs.paths();
        let unmoved = match refs::state::detect(storage, paths).await? {
            DomainState::Legacy => paths.lineage(),
            DomainState::StubOnly => paths.lineage_migrating(),
            _ => {
                let home = self.refs.home(storage).await?;
                return Ok(Some(home).filter(|home| !home.as_ref().as_os_str().is_empty()));
            }
        };
        legacy::stored_home(storage, &unmoved).await
    }

    pub async fn namespaces(&self, storage: &(impl Storage + Sync)) -> Res<BTreeSet<Namespace>> {
        #[cfg(any(test, feature = "testing"))]
        if let Some(legacy) = self.data_json() {
            return legacy.namespaces(storage).await;
        }
        self.ready(storage).await?;
        self.refs.namespaces(storage).await
    }

    /// Whether `namespace` is installed. A ref that does not parse counts.
    pub async fn contains(
        &self,
        storage: &(impl Storage + Sync),
        namespace: &Namespace,
    ) -> Res<bool> {
        #[cfg(any(test, feature = "testing"))]
        if let Some(legacy) = self.data_json() {
            return legacy.contains(storage, namespace).await;
        }
        self.ready(storage).await?;
        Ok(self.refs.contains(storage, namespace).await)
    }

    /// [`Self::contains`] for a writer about to place a package's files,
    /// refusing first while its lineage can't be written, until the move to
    /// refs is done, so nothing is placed only to be removed.
    pub(crate) async fn contains_to_write(
        &self,
        storage: &(impl Storage + Sync),
        namespace: &Namespace,
    ) -> Res<bool> {
        let installed = self.contains(storage, namespace).await?;
        #[cfg(any(test, feature = "testing"))]
        if self.on_data_json {
            return Ok(installed);
        }
        self.refs.writable_config(storage).await?;
        Ok(installed)
    }

    /// [`Self::contains`], refusing a package this build can't remove: one
    /// whose ref is in a format or state it does not support.
    pub(crate) async fn removable(
        &self,
        storage: &(impl Storage + Sync),
        namespace: &Namespace,
    ) -> Res<bool> {
        #[cfg(any(test, feature = "testing"))]
        if let Some(legacy) = self.data_json() {
            return legacy.contains(storage, namespace).await;
        }
        self.refs.removable(storage, namespace).await
    }

    /// Every installed package's entry, each read on its own: an entry that
    /// can't be read is an error for that package alone.
    pub async fn read_packages(
        &self,
        storage: &(impl Storage + Sync),
    ) -> Res<BTreeMap<Namespace, Res<PackageLineage>>> {
        #[cfg(any(test, feature = "testing"))]
        if let Some(legacy) = self.data_json() {
            return legacy.read_packages(storage).await;
        }
        self.ready(storage).await?;
        self.refs.read_all(storage).await
    }

    /// Store the locked package's first entry, as an install or a create
    /// made it, its files placed under `placed`. A home changed since is
    /// refused with [`LineageError::HomeChanged`] and nothing is stored.
    pub(crate) async fn create_package(
        &self,
        storage: &(impl Storage + Sync),
        held: &PackageLock,
        lineage: PackageLineage,
        placed: &Home,
    ) -> Res {
        #[cfg(any(test, feature = "testing"))]
        if let Some(legacy) = self.data_json() {
            return legacy
                .update_entry(storage, held.namespace(), |entry| {
                    *entry = Some(lineage);
                    Ok(())
                })
                .await;
        }
        self.refs.create(storage, held, &lineage, placed).await
    }

    /// Forget the locked package, as an uninstall does.
    pub(crate) async fn remove_package(
        &self,
        storage: &(impl Storage + Sync),
        held: &PackageLock,
    ) -> Res {
        #[cfg(any(test, feature = "testing"))]
        if let Some(legacy) = self.data_json() {
            return legacy
                .update_entry(storage, held.namespace(), |entry| {
                    *entry = None;
                    Ok(())
                })
                .await;
        }
        self.refs.remove(storage, held).await
    }

    /// Change the locked package's entry: `step` sees `None` when the package
    /// is not installed, and setting it to `None` removes the entry.
    pub(crate) async fn update_locked<T>(
        &self,
        storage: &(impl Storage + Sync),
        held: &PackageLock,
        step: impl FnOnce(&mut Option<PackageLineage>) -> Res<T>,
    ) -> Res<T> {
        #[cfg(any(test, feature = "testing"))]
        if let Some(legacy) = self.data_json() {
            return legacy.update_entry(storage, held.namespace(), step).await;
        }
        self.refs.update_entry(storage, held, step).await
    }

    /// Delete the leftovers of writes a kill interrupted.
    pub(crate) async fn sweep(&self, storage: &(impl Storage + Sync)) -> Res {
        #[cfg(any(test, feature = "testing"))]
        if self.on_data_json {
            return Ok(());
        }
        self.refs.sweep(storage).await
    }

    /// [`Self::sweep`] by a caller holding the locks in `held`, sweeping only
    /// those packages'.
    pub(crate) async fn sweep_held(
        &self,
        storage: &(impl Storage + Sync),
        held: &[PackageLock],
    ) -> Res {
        #[cfg(any(test, feature = "testing"))]
        if self.on_data_json {
            return Ok(());
        }
        self.refs.sweep_held(storage, held).await
    }

    /// Read a specific package lineage from the domain lineage
    pub async fn read_package_lineage(
        &self,
        storage: &(impl Storage + Sync),
        namespace: &Namespace,
    ) -> Res<(PathBuf, PackageLineage)> {
        #[cfg(any(test, feature = "testing"))]
        if let Some(legacy) = self.data_json() {
            return legacy.read_package_lineage(storage, namespace).await;
        }
        self.ready(storage).await?;
        let lineage = self.refs.read(storage, namespace).await?;
        let home = self.home(storage).await?;
        Ok((paths::package_home(&home, namespace), lineage))
    }

    /// `update_locked` for a test, which holds no package lock. It takes
    /// that lock, so a caller holding it must not call it.
    #[cfg(any(test, feature = "testing"))]
    pub async fn update_package_lineage<T>(
        &self,
        storage: &(impl Storage + Sync),
        namespace: &Namespace,
        step: impl FnOnce(&mut Option<PackageLineage>) -> Res<T>,
    ) -> Res<T> {
        if let Some(legacy) = self.data_json() {
            return legacy.update_entry(storage, namespace, step).await;
        }
        let held = self
            .lock_package(storage, self.refs.paths(), namespace)
            .await?;
        self.refs.update_entry(storage, &held, step).await
    }

    /// [`Self::update_package_lineage`] applying `edit` to the entry. A
    /// package that is not installed is an error.
    #[cfg(any(test, feature = "testing"))]
    pub async fn edit_package_lineage(
        &self,
        storage: &(impl Storage + Sync),
        namespace: &Namespace,
        edit: impl FnOnce(&mut PackageLineage),
    ) -> Res<PackageLineage> {
        self.update_package_lineage(storage, namespace, edit_entry(namespace, edit))
            .await
    }

    /// Point the domain at `home`, creating its folder.
    ///
    /// The folder is created before the home is stored, so a home that
    /// can't be created is never stored. When `home` is already the stored
    /// one, only a missing folder is created and nothing is rewritten.
    /// Refuses with
    /// [`LineageError::HomeInUse`] and changes nothing when `home` differs and
    /// any package is installed, because their working folders live under the
    /// current home. [`DomainLineageIo::overwrite_home`] skips that check.
    pub async fn set_home(
        &self,
        storage: &(impl Storage + Sync),
        home: impl AsRef<Path>,
    ) -> Res<Home> {
        self.change_home(storage, home, false).await
    }

    /// [`DomainLineageIo::set_home`] without the installed-packages check.
    ///
    /// Installed packages keep their entries, so they then point at folders
    /// under the new home, where none of their files are.
    pub async fn overwrite_home(
        &self,
        storage: &(impl Storage + Sync),
        home: impl AsRef<Path>,
    ) -> Res<Home> {
        self.change_home(storage, home, true).await
    }

    async fn change_home(
        &self,
        storage: &(impl Storage + Sync),
        home: impl AsRef<Path>,
        overwrite: bool,
    ) -> Res<Home> {
        #[cfg(any(test, feature = "testing"))]
        if let Some(legacy) = self.data_json() {
            return legacy.change_home(storage, home, overwrite).await;
        }
        self.ready(storage).await?;
        let home = Home::from(home);
        if overwrite {
            self.refs.overwrite_home(storage, &home).await?;
        } else {
            self.refs.set_home(storage, &home).await?;
        }
        Ok(home)
    }

    #[must_use]
    pub fn create_package_lineage(&self, namespace: Namespace) -> PackageLineageIo {
        PackageLineageIo::new(self.clone(), namespace)
    }

    /// Store a test's domain in `store`: `home` and `packages` replace
    /// whatever the domain at `paths` held there. Returns the handle its
    /// packages read through.
    #[cfg(any(test, feature = "testing"))]
    pub async fn write_fixture(
        store: LineageStore,
        storage: &(impl Storage + Sync),
        paths: &paths::DomainPaths,
        home: impl AsRef<Path>,
        packages: impl IntoIterator<Item = (Namespace, PackageLineage)>,
    ) -> Res<Self> {
        let home = Home::from(home);
        match store {
            LineageStore::DataJson => {
                let lineage = DomainLineage {
                    packages: packages.into_iter().collect(),
                    home,
                };
                legacy::DataJson::new(paths, None)
                    .write(storage, lineage)
                    .await?;
                Ok(Self::new(paths.clone()).into_data_json())
            }
            LineageStore::Refs => {
                let refs = refs::RefStore::new(paths.clone());
                refs.replace_fixture(storage, &home).await?;
                for (namespace, lineage) in packages {
                    let held = PackageLock::take(storage, paths, &namespace).await?;
                    refs.create(storage, &held, &lineage, &home).await?;
                }
                Ok(Self::new(paths.clone()))
            }
        }
    }

    /// Leave `namespace`'s stored entry one no reader can parse.
    #[cfg(any(test, feature = "testing"))]
    pub async fn damage_fixture(
        &self,
        storage: &(impl Storage + Sync),
        namespace: &Namespace,
    ) -> Res {
        if let Some(legacy) = self.data_json() {
            return legacy.damage(storage, namespace).await;
        }
        self.refs.damage_fixture(storage, namespace).await
    }

    /// The `data.json` this handle reads and writes.
    #[cfg(test)]
    pub(crate) fn legacy(&self) -> legacy::DataJson<'_> {
        self.data_json().expect("a handle on data.json")
    }
}

/// Wrapper for reading and writing `PackageLineage`
/// It re-uses `DomainLineageIo`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageLineageIo {
    domain_lineage: DomainLineageIo,
    namespace: Namespace,
}

impl PackageLineageIo {
    /// [`DomainLineageIo::edit_package_lineage`] for this package.
    #[cfg(any(test, feature = "testing"))]
    pub async fn edit(
        &self,
        storage: &(impl Storage + Sync),
        edit: impl FnOnce(&mut PackageLineage),
    ) -> Res<PackageLineage> {
        self.domain_lineage
            .edit_package_lineage(storage, &self.namespace, edit)
            .await
    }

    #[must_use]
    pub fn new(domain_lineage: DomainLineageIo, namespace: Namespace) -> Self {
        PackageLineageIo {
            domain_lineage,
            namespace,
        }
    }

    pub async fn read(&self, storage: &(impl Storage + Sync)) -> Res<(PathBuf, PackageLineage)> {
        self.domain_lineage
            .read_package_lineage(storage, &self.namespace)
            .await
    }

    pub(crate) fn domain(&self) -> &DomainLineageIo {
        &self.domain_lineage
    }

    pub async fn package_home(&self, storage: &(impl Storage + Sync)) -> Res<PathBuf> {
        Ok(self
            .domain_home(storage)
            .await?
            .join(self.namespace.to_string()))
    }

    pub async fn domain_home(&self, storage: &(impl Storage + Sync)) -> Res<Home> {
        self.domain_lineage.home(storage).await
    }

    /// [`DomainLineageIo::update_package_lineage`] setting this package's
    /// entry to `lineage`.
    #[cfg(any(test, feature = "testing"))]
    pub async fn write(
        &self,
        storage: &(impl Storage + Sync),
        lineage: PackageLineage,
    ) -> Res<PackageLineage> {
        self.domain_lineage
            .update_package_lineage(storage, &self.namespace, |entry| {
                *entry = Some(lineage.clone());
                Ok(lineage)
            })
            .await
    }

    /// Store `lineage` as this package's entry, leaving every other
    /// package's entry as it is on disk now. The caller holds the package's
    /// lock, so the entry it replaces is the one it read.
    pub(crate) async fn write_locked(
        &self,
        storage: &(impl Storage + Sync),
        held: &PackageLock,
        lineage: PackageLineage,
    ) -> Res<PackageLineage> {
        debug_assert_eq!(held.namespace(), &self.namespace);
        self.domain_lineage
            .update_locked(storage, held, |entry| {
                *entry = Some(lineage.clone());
                Ok(lineage)
            })
            .await
    }

    /// Apply `edit` to this package's entry under the lock `held` proves. A
    /// package that is not installed is an error.
    pub(crate) async fn edit_locked(
        &self,
        storage: &(impl Storage + Sync),
        held: &PackageLock,
        edit: impl FnOnce(&mut PackageLineage),
    ) -> Res<PackageLineage> {
        debug_assert_eq!(held.namespace(), &self.namespace);
        self.domain_lineage
            .update_locked(storage, held, edit_entry(&self.namespace, edit))
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use test_log::test;

    #[test]
    fn test_deserialize_package_lineage_without_remote() {
        let json = r#"{
            "commit": null,
            "base_hash": "",
            "latest_hash": "",
            "paths": {}
        }"#;
        let lineage: PackageLineage = serde_json::from_str(json).unwrap();
        assert_eq!(lineage.remote_uri, None);
    }

    #[test]
    fn test_serialize_package_lineage_without_remote() {
        let lineage = PackageLineage::default();
        let json = serde_json::to_string(&lineage).unwrap();
        // "remote" key should be omitted when remote_uri is None
        assert!(!json.contains("remote"));
    }

    /// A record written before the sync scope existed still parses, and reads
    /// as the sparse-checkout default — the whole reason adopting the field
    /// costs no migration.
    #[test]
    fn a_record_without_a_sync_scope_reads_as_individual_files() {
        let json = r#"{
            "commit": null,
            "base_hash": "",
            "latest_hash": "",
            "paths": {}
        }"#;
        let lineage: PackageLineage = serde_json::from_str(json).unwrap();
        assert_eq!(lineage.sync_scope, SyncScope::IndividualFiles);
    }

    /// And the other direction: an opted-in package round-trips.
    #[test]
    fn an_entire_package_scope_survives_a_round_trip() {
        let lineage = PackageLineage {
            sync_scope: SyncScope::EntirePackage,
            ..PackageLineage::default()
        };
        let json = serde_json::to_string(&lineage).unwrap();
        assert!(json.contains(r#""sync_scope":"entire_package""#), "{json}");

        let parsed: PackageLineage = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.sync_scope, SyncScope::EntirePackage);
    }

    /// The default scope writes **no key**, so turning the feature on does not
    /// rewrite `sync_scope` into the record of every installed package — and
    /// dropping the field later erases it on the next write, since the record
    /// is re-serialized whole.
    #[test]
    fn the_default_scope_writes_no_key() {
        let json = serde_json::to_string(&PackageLineage::default()).unwrap();
        assert!(!json.contains("sync_scope"), "{json}");
    }

    #[test]
    fn home_in_use_counts_packages_in_plural() {
        let err = LineageError::HomeInUse {
            home: PathBuf::from("/old/home"),
            installed: 3,
        };
        assert_eq!(
            err.to_string(),
            "Cannot change the home: 3 packages are installed in /old/home"
        );
    }

    #[test(tokio::test)]
    async fn test_domain_lineage_create_package_lineage() -> Res {
        let namespace = ("foo", "bar");
        let domain_lineage = DomainLineageIo::new(paths::DomainPaths::new(PathBuf::new()));
        let lineage = domain_lineage.create_package_lineage(namespace.into());
        assert_eq!(
            lineage,
            PackageLineageIo {
                namespace: namespace.into(),
                domain_lineage,
            }
        );
        Ok(())
    }
}
