use std::collections::BTreeMap;
use std::marker::Unpin;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use tracing::{debug, info, trace, warn};

use crate::Error;
use crate::Res;
use crate::checksum;
use crate::error::LineageError;
use crate::flow;
use crate::installed_package::InstalledPackage;
use crate::io::manifest::RowsStream;
use crate::io::manifest::build_manifest_from_rows_stream;
use crate::io::remote::Remote;
use crate::io::remote::RemoteS3;
use crate::io::storage::LocalStorage;
use crate::io::storage::Storage;
use crate::lineage;
use crate::lineage::Home;
use crate::lineage::LineagePaths;
use crate::lineage::PathState;
use crate::manifest::Manifest;
use crate::manifest::ManifestHeader;
use crate::package_lock::PackageLock;
use crate::paths;
use crate::state_store::PackageStateStore;
use quilt_uri::Host;
use quilt_uri::ManifestUri;
use quilt_uri::Namespace;

/// This is the entrypoint for the lib.
/// All the work you can do with packages is done through calling `LocalDomain` methods.
///
/// Cheap to clone: a clone shares the remote and works on the same
/// directory, so a caller holding the domain behind a mutex can clone it and
/// release the mutex before a writer that waits on a package's lock.
#[derive(Debug)]
pub struct LocalDomain<S: Storage = LocalStorage, R: Remote = RemoteS3> {
    paths: paths::DomainPaths,
    lineage: lineage::DomainLineageIo,
    storage: S,
    /// Shared so a caller holding the domain behind a mutex can take a
    /// handle, release the mutex, and only then await a network round trip.
    /// See [`LocalDomain::remote_handle`].
    remote: Arc<R>,
    /// Handed to every package; see [`LocalDomain::with_state_store`].
    state_store: Option<Arc<PackageStateStore>>,
}

impl<S: Storage + Clone, R: Remote> Clone for LocalDomain<S, R> {
    fn clone(&self) -> Self {
        Self {
            paths: self.paths.clone(),
            lineage: self.lineage.clone(),
            storage: self.storage.clone(),
            remote: Arc::clone(&self.remote),
            state_store: self.state_store.clone(),
        }
    }
}

impl LocalDomain {
    #[must_use]
    pub fn get_remote(&self) -> &RemoteS3 {
        &self.remote
    }

    /// A handle to *this* remote — the same client and credential caches,
    /// not a snapshot — that outlives a borrow of the domain.
    ///
    /// Callers that reach the domain through a mutex (the desktop app holds
    /// one around it) must use this for anything that awaits the network:
    /// `lock().await.get_remote().call().await` keeps the domain locked for
    /// the whole round trip, blocking every other command behind it.
    ///
    /// Deliberately not [`RemoteS3::try_clone`], which snapshots the client
    /// cache into a separate `RemoteS3`: a role switch clears the original's
    /// cache and the snapshot would keep signing as the old role.
    #[must_use]
    pub fn remote_handle(&self) -> Arc<RemoteS3> {
        Arc::clone(&self.remote)
    }

    pub fn new(root_dir: impl AsRef<Path>) -> Self {
        let paths = paths::DomainPaths::new(root_dir.as_ref().to_path_buf());
        let lineage = lineage::DomainLineageIo::new(paths.clone());
        let storage = LocalStorage::new();
        let remote = Arc::new(RemoteS3::new(paths.clone(), storage.clone()));
        Self {
            paths,
            lineage,
            storage,
            remote,
            state_store: None,
        }
    }

    /// The stored home, or `None` when the domain has none yet, as on a
    /// domain no command has used. Only reads: it writes and creates nothing.
    /// A record that exists but can't be read, such as for lack of
    /// permission, is an error, not `None`.
    pub async fn stored_home(&self) -> Res<Option<Home>> {
        self.lineage.stored_home(&self.storage).await
    }
}

impl<S: Storage + Clone + Sync, R: Remote> LocalDomain<S, R> {
    /// A domain over `paths` with the given lineage, storage and remote, for
    /// tests that drive it against a mock.
    #[cfg(test)]
    pub(crate) fn with_parts(
        paths: paths::DomainPaths,
        lineage: lineage::DomainLineageIo,
        storage: S,
        remote: Arc<R>,
    ) -> Self {
        Self {
            lineage,
            paths,
            storage,
            remote,
            state_store: None,
        }
    }

    /// The domain with its packages' walks using `store`'s stat cache. Only
    /// the desktop app opens a store.
    #[must_use]
    pub fn with_state_store(mut self, store: Arc<PackageStateStore>) -> Self {
        self.state_store = Some(store);
        self
    }

    /// The domain with `hook` told of every package entry its lineage writes
    /// change, by any of its packages. Only the desktop app sets
    /// one, to tell its own writes from another tool's.
    #[must_use]
    pub fn with_lineage_hook(mut self, hook: lineage::LineageHook) -> Self {
        self.lineage = self.lineage.with_hook(hook);
        self
    }

    /// Hands the domain's lineage back from refs to `data.json`, or finishes
    /// a rollback that was interrupted, and returns the folder the refs were
    /// moved to. Refused while any ref holds what `data.json` cannot. A
    /// domain opened afterwards moves the lineage to refs again, but not
    /// through a value that had already found it there.
    pub async fn rollback_lineage(&self) -> Res<PathBuf> {
        let rolled = lineage::refs::rollback::down(&self.storage, &self.paths).await?;
        Ok(rolled.archive)
    }

    /// The domain with its lineage in `store`: `data.json` stands in for a
    /// build from before refs. Keeps the hook set so far.
    #[cfg(any(test, feature = "testing"))]
    #[must_use]
    pub fn with_lineage_store(mut self, store: lineage::LineageStore) -> Self {
        if store == lineage::LineageStore::DataJson {
            self.lineage = self.lineage.into_data_json(self.paths.lineage());
        }
        self
    }

    /// Deletes the temp files that killed lineage writes left, each package's
    /// only while its lock is free. A temp beside `data.json` is not swept.
    pub async fn sweep_leftovers(&self) -> Res {
        self.lineage.sweep(&self.storage).await
    }

    /// The handle this domain's lineage reads and writes go through, telling
    /// the hook set so far.
    #[must_use]
    pub fn lineage_io(&self) -> &lineage::DomainLineageIo {
        &self.lineage
    }

    /// The user-wide ignore rules file, which every package reads below its
    /// own `.quiltignore`.
    #[must_use]
    pub fn user_quiltignore(&self) -> std::path::PathBuf {
        self.paths.user_quiltignore()
    }

    /// [`crate::quiltignore::set_default_rules`] on the user-wide file, holding
    /// its lock so *Stop ignoring* for all packages never writes between the
    /// read and the write.
    pub async fn set_default_rules(&self, off: &[&str]) -> Res {
        let _held = self
            .storage
            .lock_exclusive(self.paths.user_quiltignore_lock())
            .await?;
        crate::quiltignore::set_default_rules(&self.paths.user_quiltignore(), off)
    }

    pub async fn get_home(&self) -> Res<Home> {
        self.lineage.home(&self.storage).await
    }

    /// See [`lineage::DomainLineageIo::set_home`]: creates the home folder,
    /// and refuses a new home while packages are installed.
    pub async fn set_home(&self, dir: impl AsRef<Path>) -> Res<Home> {
        info!("Setting home directory to {}", dir.as_ref().display());
        self.lineage.set_home(&self.storage, dir).await
    }

    /// [`LocalDomain::set_home`] even while packages are installed: they then
    /// point at empty folders under the new home.
    pub async fn overwrite_home(&self, dir: impl AsRef<Path>) -> Res<Home> {
        info!("Overwriting home directory with {}", dir.as_ref().display());
        self.lineage.overwrite_home(&self.storage, dir).await
    }

    /// Returns the home it scaffolded under.
    pub async fn scaffold_paths_for_installing(&self, namespace: &Namespace) -> Res<Home> {
        debug!(
            "Scaffolding installation paths for namespace: {}",
            namespace
        );
        let home = self.get_home().await?;
        self.paths
            .scaffold_for_installing(&self.storage, &home, namespace)
            .await?;
        Ok(home)
    }

    pub async fn scaffold_paths_for_caching(&self, bucket: &str) -> Res {
        debug!("Scaffolding cache paths for bucket: {}", bucket);
        self.paths.scaffold_for_caching(&self.storage, bucket).await
    }

    pub async fn browse_remote_manifest(&self, uri: &ManifestUri) -> Res<Manifest> {
        self.scaffold_paths_for_caching(&uri.bucket).await?;

        debug!("Initiating browse flow for manifest {}", uri.hash);
        flow::browse(&self.paths, &self.storage, &*self.remote, uri)
            .await
            .inspect(|_| debug!("Successfully browsed manifest {}", uri.hash))
            .inspect_err(|e| warn!("Failed to browse manifest {}: {}", uri.hash, e))
    }

    /// [`flow::resolve_hash`] on this domain's remote: the whole top hash of
    /// the one revision in `bucket`'s registry that starts with `prefix`.
    pub async fn resolve_hash(
        &self,
        host: Option<&Host>,
        bucket: &str,
        prefix: &str,
    ) -> Res<String> {
        flow::resolve_hash(&*self.remote, host, bucket, prefix).await
    }

    /// [`flow::push_onto`] on this domain's remote.
    pub async fn push_onto(
        &self,
        parent: &ManifestUri,
        manifest: Manifest,
    ) -> Res<crate::PushOutcome> {
        self.scaffold_paths_for_caching(&parent.bucket).await?;
        flow::push_onto(&self.paths, &self.storage, &*self.remote, parent, manifest).await
    }

    /// Every package shares *this* domain's remote — the same client,
    /// region and credential caches, not a snapshot. A per-package copy
    /// would start empty and discard whatever it built, and logout could
    /// not reach the clients it is required to drop.
    #[must_use]
    pub fn create_installed_package(&self, namespace: Namespace) -> InstalledPackage<S, R> {
        // TODO: seems like you can use PackageLineage as an argument instead of namespace
        InstalledPackage {
            lineage: self.lineage.create_package_lineage(namespace.clone()),
            namespace,
            paths: self.paths.clone(),
            remote: Arc::clone(&self.remote),
            storage: self.storage.clone(),
            state_store: self.state_store.clone(),
        }
    }

    pub async fn install_package(&self, manifest_uri: &ManifestUri) -> Res<InstalledPackage<S, R>> {
        info!("Installing package: {}", manifest_uri.namespace);
        debug!("Installing from manifest: {}", manifest_uri.display());
        let namespace = &manifest_uri.namespace;
        let held = self
            .lineage
            .lock_package(&self.storage, &self.paths, namespace)
            .await?;

        // Read before anything is created: a record that does not parse
        // stops it here.
        let installed = self
            .lineage
            .contains_to_write(&self.storage, namespace)
            .await?;
        debug!("Preparing paths for installation");
        self.scaffold_paths_for_caching(&manifest_uri.bucket)
            .await?;
        let home = self.get_home().await?;
        let placed = Placed::before(&self.storage, &home, namespace).await;
        let manifest = self.paths.installed_manifest(namespace, &manifest_uri.hash);
        let manifest = (!self.storage.exists(&manifest).await).then_some(manifest);
        self.paths
            .scaffold_for_installing(&self.storage, &home, namespace)
            .await?;

        debug!("Executing package installation flow");
        let lineage = flow::install_package(
            &home,
            installed,
            &self.paths,
            &self.storage,
            &*self.remote,
            manifest_uri,
        )
        .await?;

        debug!("Updating domain lineage");
        crate::failpoint::hit("flow.install.before-create");
        let created = self
            .lineage
            .create_package(&self.storage, &held, lineage, &home)
            .await;
        placed
            .undo_if_unrecorded(&self.storage, created, &LineagePaths::new(), manifest)
            .await?;

        info!("Successfully installed package: {}", namespace);
        Ok(self.create_installed_package(namespace.clone()))
    }

    pub async fn create_package(
        &self,
        namespace: Namespace,
        source: Option<PathBuf>,
        message: Option<String>,
        user_meta: flow::UserMeta,
    ) -> Res<(InstalledPackage<S, R>, flow::LeftOut)> {
        info!("Creating package: {}", namespace);
        let held = self
            .lineage
            .lock_package(&self.storage, &self.paths, &namespace)
            .await?;

        let home = self.get_home().await?;
        let installed = self
            .lineage
            .contains_to_write(&self.storage, &namespace)
            .await?;
        let placed = Placed::before(&self.storage, &home, &namespace).await;

        let (lineage, left_out) = flow::create(
            &home,
            installed,
            &self.paths,
            &self.storage,
            namespace.clone(),
            source,
            message,
            user_meta,
        )
        .await?;

        let files = lineage.paths.clone();
        let manifest = lineage
            .commit
            .as_ref()
            .map(|commit| self.paths.installed_manifest(&namespace, &commit.hash));
        let created = self
            .lineage
            .create_package(&self.storage, &held, lineage, &home)
            .await;
        placed
            .undo_if_unrecorded(&self.storage, created, &files, manifest)
            .await?;

        info!("Successfully created package: {}", namespace);
        Ok((self.create_installed_package(namespace), left_out))
    }

    pub async fn uninstall_package(&self, namespace: Namespace) -> Res<()> {
        info!("Uninstalling package: {}", namespace);
        let held = self
            .lineage
            .lock_package(&self.storage, &self.paths, &namespace)
            .await?;
        self.uninstall_locked(&held).await
    }

    /// Uninstall, then delete the package's objects that no other installed
    /// manifest uses — every revision's. Nothing else: other unused objects,
    /// the manifest cache and staging stay for [`Self::gc`].
    ///
    /// The uninstall stands whatever the prune finds. Another package busy
    /// in another writer keeps the objects, answered as
    /// [`flow::Pruned::Busy`]; a manifest that can't be read keeps them too,
    /// as [`Error::PruneFailed`]. So does one of the package's own manifests
    /// or objects that can't be read: the package is still removed, and what
    /// was read is still pruned.
    pub async fn uninstall_package_pruning(&self, namespace: Namespace) -> Res<flow::Pruned> {
        info!("Uninstalling package and its objects: {}", namespace);
        let (candidates, unread) = {
            let held = self
                .lineage
                .lock_package(&self.storage, &self.paths, &namespace)
                .await?;
            let found = flow::package_objects(&self.paths, &self.storage, &namespace).await?;
            self.uninstall_locked(&held).await?;
            found
        };
        // The package's own lock is released above: the prune try-locks
        // every package, this one among them.
        let prune = async {
            let installed = self
                .lineage
                .namespaces(&self.storage)
                .await
                .map_err(|err| (err, None))?;
            let pruned = flow::prune(&self.paths, &self.storage, &installed, candidates)
                .await
                .map_err(|err| (err, None))?;
            // Some of its files could not be counted, so not all went. The
            // read error goes first, since once the manifests are gone no
            // retry can show it, and a busy package that kept the rest is
            // named beside it.
            match (unread, pruned) {
                (None, pruned) => Ok(pruned),
                (Some(err), flow::Pruned::Busy(busy)) => Err((err, Some(busy))),
                (Some(err), flow::Pruned::Freed(_)) => Err((err, None)),
            }
        };
        prune
            .await
            .map_err(|(err, busy)| Error::PruneFailed(namespace, Box::new(err), busy))
    }

    /// Uninstall the package whose lock the caller holds.
    async fn uninstall_locked(&self, held: &PackageLock) -> Res<()> {
        let namespace = held.namespace();
        // Before any file goes: a package whose lineage can't be removed
        // keeps its files too.
        let installed = self.lineage.removable(&self.storage, namespace).await?;
        debug!("Preparing paths for uninstallation");
        let home = self.scaffold_paths_for_installing(namespace).await?;

        debug!("Executing package uninstallation flow");
        flow::uninstall_package(
            &home,
            installed,
            &self.paths,
            &self.storage,
            namespace.clone(),
        )
        .await?;

        debug!("Updating domain lineage after uninstallation");
        crate::failpoint::hit("flow.uninstall.before-lineage");
        self.lineage.remove_package(&self.storage, held).await?;

        info!("Successfully uninstalled package: {}", namespace);
        Ok(())
    }

    /// Delete what `.quilt/` holds for nothing: objects no installed
    /// manifest uses, the remote-manifest cache and stranded staging dirs.
    /// See [`flow::gc`]; a package busy in another writer stops it with
    /// [`Error::PackageBusy`] and nothing deleted.
    pub async fn gc(&self) -> Res<flow::GcReport> {
        let installed = self.lineage.namespaces(&self.storage).await?;
        flow::gc_under(&self.paths, &self.storage, &installed, async |held| {
            self.lineage.sweep_held(&self.storage, held).await
        })
        .await
    }

    /// How much `.quilt/` holds, and what [`Self::gc`] would free now. See
    /// [`flow::measure_storage`]: it takes no package locks, deletes nothing,
    /// and its freeable part is an estimate that gc's report makes exact.
    pub async fn measure_storage(&self) -> Res<flow::StorageSize> {
        flow::measure_storage(&self.paths, &self.storage).await
    }

    pub async fn list_installed_packages(&self) -> Res<Vec<InstalledPackage<S, R>>> {
        // A pair of lines for a lookup that runs on every tick. The count is the
        // only part worth keeping, and only when it is surprising — so both go to
        // `trace` and the status line reports what was found.
        trace!("Listing installed packages");
        let namespaces = self.lineage.namespaces(&self.storage).await?;

        trace!("Found {} installed packages", namespaces.len());
        let mut packages = Vec::with_capacity(namespaces.len());
        for namespace in namespaces {
            packages.push(self.create_installed_package(namespace));
        }
        Ok(packages)
    }

    /// Every installed package's lineage; see
    /// [`lineage::DomainLineageIo::read_packages`].
    pub async fn read_packages(&self) -> Res<BTreeMap<Namespace, Res<lineage::PackageLineage>>> {
        trace!("Reading every package's lineage");
        self.lineage.read_packages(&self.storage).await
    }

    pub async fn get_installed_package(
        &self,
        namespace: &Namespace,
    ) -> Res<Option<InstalledPackage<S, R>>> {
        trace!("Looking up installed package: {}", namespace);
        if self.lineage.contains(&self.storage, namespace).await? {
            trace!("Found installed package: {}", namespace);
            Ok(Some(self.create_installed_package(namespace.to_owned())))
        } else {
            debug!("Package not found: {}", namespace);
            Ok(None)
        }
    }

    pub async fn build_manifest(
        &self,
        dest_path: PathBuf,
        stream: impl RowsStream + Unpin,
    ) -> Res<(PathBuf, String)> {
        info!("Building manifest at: {}", dest_path.display());
        let dest_dir = dest_path.parent().unwrap_or(&dest_path).to_path_buf();
        build_manifest_from_rows_stream(
            &self.storage,
            dest_dir,
            ManifestHeader::default(),
            Vec::new(),
            stream,
        )
        .await
    }
}

/// The folder an install or a create places a package's files in, and
/// whether it was there before.
struct Placed {
    folder: PathBuf,
    existed: bool,
}

impl Placed {
    async fn before(storage: &impl Storage, home: &Home, namespace: &Namespace) -> Self {
        let folder = paths::package_home(home, namespace);
        Self {
            existed: storage.exists(&folder).await,
            folder,
        }
    }

    /// `created`, unless the package was refused its ref because the home
    /// changed or the lineage is moving: then what was placed goes, but for
    /// files edited since, which a home change's error names. Nothing refers
    /// to them without a ref.
    async fn undo_if_unrecorded(
        self,
        storage: &(impl Storage + Sync),
        created: Res,
        files: &LineagePaths,
        manifest: Option<PathBuf>,
    ) -> Res {
        match created {
            Err(Error::Lineage(LineageError::HomeChanged { placed, home, .. })) => {
                let kept = self.undo(storage, files, manifest).await;
                Err(LineageError::HomeChanged { placed, home, kept }.into())
            }
            Err(err @ Error::Lineage(LineageError::RefsNotWritable { .. })) => {
                for file in self.undo(storage, files, manifest).await {
                    warn!("Kept {}, edited while it was placed", file.display());
                }
                Err(err)
            }
            created => created,
        }
    }

    /// Removes what was placed and returns the files kept. In a folder that
    /// was there before, a file may be the user's own with the same bytes,
    /// so none is removed.
    async fn undo(
        self,
        storage: &(impl Storage + Sync),
        files: &LineagePaths,
        manifest: Option<PathBuf>,
    ) -> Vec<PathBuf> {
        let mut kept = Vec::new();
        let placed_files = if self.existed { None } else { Some(files) };
        for (key, state) in placed_files.into_iter().flatten() {
            let Ok(key) = crate::LogicalKey::new(key) else {
                continue;
            };
            let file = key.join_to(&self.folder);
            if !holds(storage, &file, state).await {
                if storage.exists(&file).await {
                    kept.push(file);
                }
            } else if let Err(err) = storage.remove_file(&file).await {
                warn!("Could not remove {}: {err}", file.display());
            } else {
                // `remove_dir` refuses a folder something was written into.
                for folder in file.ancestors().skip(1) {
                    if folder == self.folder || storage.remove_dir(folder).await.is_err() {
                        break;
                    }
                }
            }
        }
        if let Some(manifest) = manifest
            && let Err(err) = storage.remove_file(&manifest).await
        {
            warn!("Could not remove {}: {err}", manifest.display());
        }
        if !self.existed {
            // Not empty when a file in it was kept.
            let _ = storage.remove_dir(&self.folder).await;
        }
        kept
    }
}

/// Whether `file` still holds the bytes `state` was hashed from.
async fn holds(storage: &impl Storage, file: &Path, state: &PathState) -> bool {
    let Some(hash) = &state.hash else {
        return false;
    };
    match checksum::hash_file(storage, file, hash.code()).await {
        Ok((found, _)) => multihash::Multihash::<256>::from(found) == *hash,
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use tempfile::TempDir;
    use test_log::test;

    /// Setting the built-in rules waits for the user-wide file's lock, which
    /// *Stop ignoring* for all packages holds across its read and write.
    #[test(tokio::test)]
    async fn set_default_rules_waits_for_the_user_rules_lock() -> Res<()> {
        let temp_dir = TempDir::new()?;
        let domain = LocalDomain::new(temp_dir.path());
        let held = domain
            .storage
            .lock_exclusive(domain.paths.user_quiltignore_lock())
            .await?;
        let writer = {
            let domain = domain.clone();
            tokio::spawn(async move { domain.set_default_rules(&[".env"]).await })
        };
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(!domain.user_quiltignore().exists(), "it waited");
        drop(held);
        writer.await.expect("the writer ran")?;
        assert_eq!(
            std::fs::read_to_string(domain.user_quiltignore())?,
            "!.env\n"
        );
        Ok(())
    }

    /// An install over a `data.json` that does not parse fails before it
    /// creates the package's folder.
    #[test(tokio::test)]
    async fn an_install_over_an_unreadable_record_creates_no_folder() -> Res<()> {
        let temp_dir = TempDir::new()?;
        let domain =
            LocalDomain::new(temp_dir.path()).with_lineage_store(lineage::LineageStore::DataJson);
        domain.set_home(temp_dir.path()).await?;
        domain
            .create_package(("foo", "other").into(), None, None, flow::UserMeta::Clear)
            .await?;
        domain
            .lineage
            .damage_fixture(&domain.storage, &("foo", "other").into())
            .await?;

        let result = domain
            .install_package(&ManifestUri {
                bucket: "bucket".to_string(),
                namespace: ("foo", "bar").into(),
                hash: "abcdef0000000000000000000000000000000000000000000000000000000000"
                    .to_string(),
                origin: None,
            })
            .await;

        assert!(result.is_err(), "{result:?}");
        assert!(!temp_dir.path().join("foo").join("bar").exists());
        Ok(())
    }
}

/// What only the ref store does.
#[cfg(test)]
mod ref_store_tests {
    use std::str::FromStr;
    use std::time::Duration;

    use aws_sdk_s3::primitives::ByteStream;
    use tempfile::TempDir;
    use test_log::test;

    use super::*;
    use crate::io::remote::mocks::MockRemote;
    use crate::lineage::LineageHook;
    use crate::manifest::TopHasher;
    use quilt_uri::S3Uri;

    const BUCKET: &str = "b";

    /// A domain on refs under `root`, its home `root/home`.
    async fn domain(root: &Path) -> Res<LocalDomain> {
        let domain = LocalDomain::new(root.join("domain"));
        domain.set_home(root.join("home")).await?;
        Ok(domain)
    }

    /// A remote holding one revision of `acme/x` with no files, `latest`.
    async fn remote_with_one_revision() -> Res<(MockRemote, ManifestUri)> {
        let remote = MockRemote::default();
        let manifest = Manifest::default();
        let mut hasher = TopHasher::new();
        hasher.append_header(&manifest.header)?;
        let hash = hasher.finalize();
        let at = |key: &str| S3Uri::from_str(&format!("s3://{BUCKET}/.quilt/{key}"));
        remote
            .put_object(
                None,
                &at(&format!("packages/{hash}"))?,
                ByteStream::from(&manifest),
            )
            .await?;
        remote
            .put_object(
                None,
                &at("named_packages/acme/x/latest")?,
                hash.as_bytes().to_vec(),
            )
            .await?;
        let uri = ManifestUri {
            origin: None,
            bucket: BUCKET.to_string(),
            namespace: ("acme", "x").into(),
            hash,
        };
        Ok((remote, uri))
    }

    /// [`write_config_once_placed`] moving the home to `root/moved`.
    async fn move_home_once_placed<T>(
        domain: &LocalDomain<impl Storage + Clone + Sync, impl Remote>,
        root: &Path,
        placed: impl Fn() -> bool,
        placing: tokio::task::JoinHandle<Res<T>>,
        meanwhile: impl FnOnce() -> std::io::Result<()>,
    ) -> Res<T> {
        let config = serde_json::json!({
            "format": 1,
            "home": root.join("moved"),
            "migration": "done",
        });
        write_config_once_placed(domain, &config, placed, placing, meanwhile).await
    }

    /// Holds the domain file lock until `placed`, which the placing makes
    /// true last, as a home change or a rollback in another process would,
    /// runs `meanwhile`, writes `config`, then lets `placing` go on to
    /// record the package.
    async fn write_config_once_placed<T>(
        domain: &LocalDomain<impl Storage + Clone + Sync, impl Remote>,
        config: &serde_json::Value,
        placed: impl Fn() -> bool,
        placing: tokio::task::JoinHandle<Res<T>>,
        meanwhile: impl FnOnce() -> std::io::Result<()>,
    ) -> Res<T> {
        let held = LocalStorage::new()
            .lock_exclusive(domain.paths.domain_file_lock())
            .await?;
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        while !placed() && std::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        meanwhile()?;
        std::fs::write(domain.paths.domain_config(), serde_json::to_vec(config)?)?;
        drop(held);
        placing.await.expect("the task ran")
    }

    fn home_changed(root: &Path, result: &Res<impl std::fmt::Debug>) -> Option<Vec<PathBuf>> {
        match result {
            Err(Error::Lineage(LineageError::HomeChanged { placed, home, kept }))
                if *placed == root.join("home") && *home == root.join("moved") =>
            {
                Some(kept.clone())
            }
            _ => None,
        }
    }

    #[test(tokio::test)]
    async fn a_home_changed_under_an_install_records_nothing_and_removes_what_it_placed() -> Res<()>
    {
        let root = TempDir::new()?;
        let (remote, uri) = remote_with_one_revision().await?;
        let paths = paths::DomainPaths::new(root.path().join("domain"));
        let domain = LocalDomain::with_parts(
            paths.clone(),
            lineage::DomainLineageIo::new(paths.clone()),
            LocalStorage::new(),
            Arc::new(remote),
        );
        domain.set_home(root.path().join("home")).await?;
        let manifest = paths.installed_manifest(&uri.namespace, &uri.hash);

        let installing = tokio::spawn({
            let (domain, uri) = (domain.clone(), uri.clone());
            async move { domain.install_package(&uri).await.map(drop) }
        });
        let result = move_home_once_placed(
            &domain,
            root.path(),
            || manifest.is_file(),
            installing,
            || Ok(()),
        )
        .await;

        assert_eq!(
            home_changed(root.path(), &result),
            Some(Vec::new()),
            "{result:?}"
        );
        assert!(
            !domain
                .lineage
                .contains(&domain.storage, &uri.namespace)
                .await?
        );
        assert!(!manifest.exists());
        assert!(!root.path().join("home/acme/x").exists());
        Ok(())
    }

    #[test(tokio::test)]
    async fn a_rollback_begun_under_an_install_records_nothing_and_removes_what_it_placed()
    -> Res<()> {
        let root = TempDir::new()?;
        let (remote, uri) = remote_with_one_revision().await?;
        let paths = paths::DomainPaths::new(root.path().join("domain"));
        let domain = LocalDomain::with_parts(
            paths.clone(),
            lineage::DomainLineageIo::new(paths.clone()),
            LocalStorage::new(),
            Arc::new(remote),
        );
        domain.set_home(root.path().join("home")).await?;
        let manifest = paths.installed_manifest(&uri.namespace, &uri.hash);

        let installing = tokio::spawn({
            let (domain, uri) = (domain.clone(), uri.clone());
            async move { domain.install_package(&uri).await.map(drop) }
        });
        let rolling_back = serde_json::json!({
            "format": 1,
            "home": root.path().join("home"),
            "migration": {"rolling_back": {"archive": "refs.rolled-back-x"}},
        });
        let result = write_config_once_placed(
            &domain,
            &rolling_back,
            || manifest.is_file(),
            installing,
            || Ok(()),
        )
        .await;

        assert!(
            matches!(
                result,
                Err(Error::Lineage(LineageError::RefsNotWritable { .. }))
            ),
            "{result:?}"
        );
        assert!(
            !domain
                .lineage
                .contains(&domain.storage, &uri.namespace)
                .await?
        );
        assert!(!manifest.exists());
        assert!(!root.path().join("home/acme/x").exists());
        Ok(())
    }

    #[test(tokio::test)]
    async fn a_home_changed_under_a_create_keeps_only_the_files_edited_meanwhile() -> Res<()> {
        let root = TempDir::new()?;
        let source = root.path().join("source");
        std::fs::create_dir_all(source.join("sub"))?;
        std::fs::write(source.join("sub/kept.txt"), b"placed")?;
        std::fs::write(source.join("sub/gone.txt"), b"placed")?;
        let domain = domain(root.path()).await?;
        let namespace: Namespace = ("acme", "x").into();
        let manifests = domain.paths.installed_manifests_dir(&namespace);
        let placed = root.path().join("home/acme/x");

        let creating = tokio::spawn({
            let (domain, namespace) = (domain.clone(), namespace.clone());
            async move {
                domain
                    .create_package(namespace, Some(source), None, flow::UserMeta::Clear)
                    .await
                    .map(drop)
            }
        });
        let edited = placed.join("sub/kept.txt");
        let built = || std::fs::read_dir(&manifests).is_ok_and(|mut found| found.next().is_some());
        let result = move_home_once_placed(&domain, root.path(), built, creating, || {
            std::fs::write(&edited, b"edited")
        })
        .await;

        assert_eq!(
            home_changed(root.path(), &result),
            Some(vec![edited.clone()]),
            "{result:?}"
        );
        assert!(!domain.lineage.contains(&domain.storage, &namespace).await?);
        assert_eq!(std::fs::read(&edited)?, b"edited");
        assert!(!placed.join("sub/gone.txt").exists());
        assert_eq!(std::fs::read_dir(&manifests)?.count(), 0);
        Ok(())
    }

    #[test(tokio::test)]
    async fn a_ref_that_does_not_parse_is_uninstalled_and_one_unsupported_is_not() -> Res<()> {
        let root = TempDir::new()?;
        let domain = domain(root.path()).await?;
        for name in ["damaged", "newer"] {
            domain
                .create_package(("acme", name).into(), None, None, flow::UserMeta::Clear)
                .await?;
        }
        let newer = domain.paths.package_ref(&("acme", "newer").into());
        std::fs::write(domain.paths.package_ref(&("acme", "damaged").into()), b"{")?;
        std::fs::write(&newer, br#"{"format": 2}"#)?;

        domain.uninstall_package(("acme", "damaged").into()).await?;
        let refused = domain.uninstall_package(("acme", "newer").into()).await;

        assert!(!root.path().join("home/acme/damaged").exists());
        assert!(
            matches!(
                refused,
                Err(Error::Lineage(LineageError::UnsupportedFormat {
                    found: 2,
                    ..
                }))
            ),
            "{refused:?}"
        );
        assert!(root.path().join("home/acme/newer").exists());
        assert_eq!(std::fs::read(&newer)?, br#"{"format": 2}"#);
        assert_eq!(
            domain.lineage.namespaces(&domain.storage).await?,
            std::collections::BTreeSet::from([("acme", "newer").into()])
        );
        Ok(())
    }

    /// A package created into a folder that was there may hold the user's
    /// own files with the same bytes, so none is removed.
    #[test(tokio::test)]
    async fn a_home_changed_under_a_create_into_an_existing_folder_removes_no_file() -> Res<()> {
        let root = TempDir::new()?;
        let placed = root.path().join("home/acme/x");
        let source = root.path().join("source");
        for folder in [&placed, &source] {
            std::fs::create_dir_all(folder)?;
            std::fs::write(folder.join("own.txt"), b"own")?;
        }
        let domain = domain(root.path()).await?;
        let namespace: Namespace = ("acme", "x").into();
        let manifests = domain.paths.installed_manifests_dir(&namespace);

        let creating = tokio::spawn({
            let (domain, namespace) = (domain.clone(), namespace.clone());
            async move {
                domain
                    .create_package(namespace, Some(source), None, flow::UserMeta::Clear)
                    .await
                    .map(drop)
            }
        });
        let built = || std::fs::read_dir(&manifests).is_ok_and(|mut found| found.next().is_some());
        let result = move_home_once_placed(&domain, root.path(), built, creating, || Ok(())).await;

        assert_eq!(
            home_changed(root.path(), &result),
            Some(Vec::new()),
            "{result:?}"
        );
        assert_eq!(std::fs::read(placed.join("own.txt"))?, b"own");
        Ok(())
    }

    /// Only once the domain's move to refs is done is a ref removed, so a
    /// package's files go only then.
    #[test(tokio::test)]
    async fn an_uninstall_keeps_the_files_while_refs_cannot_be_written() -> Res<()> {
        let root = TempDir::new()?;
        let domain = domain(root.path()).await?;
        domain
            .create_package(("acme", "x").into(), None, None, flow::UserMeta::Clear)
            .await?;
        let config = serde_json::json!({
            "format": 1,
            "home": root.path().join("home"),
            "migration": "in_progress",
        });
        std::fs::write(domain.paths.domain_config(), serde_json::to_vec(&config)?)?;

        let refused = domain.uninstall_package(("acme", "x").into()).await;

        assert!(
            matches!(
                refused,
                Err(Error::Lineage(LineageError::RefsNotWritable { .. }))
            ),
            "{refused:?}"
        );
        assert!(root.path().join("home/acme/x").exists());
        Ok(())
    }

    /// A pull that finds nothing to pull but a moved tip caches the tip and
    /// leaves the ref and the hook alone.
    #[test(tokio::test)]
    async fn a_pull_that_only_moves_the_tip_writes_only_the_tip_cache() -> Res<()> {
        let root = TempDir::new()?;
        let (remote, uri) = remote_with_one_revision().await?;
        let paths = paths::DomainPaths::new(root.path().join("domain"));
        let told = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let hook = LineageHook::new({
            let told = Arc::clone(&told);
            move |_, _, _| {
                told.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
        });
        let domain = LocalDomain::with_parts(
            paths.clone(),
            lineage::DomainLineageIo::new(paths.clone()),
            LocalStorage::new(),
            Arc::new(remote),
        )
        .with_lineage_hook(hook);
        domain.set_home(root.path().join("home")).await?;
        let package = domain.install_package(&uri).await?;
        let stale = "f".repeat(64);
        package
            .lineage
            .edit(&package.storage, |lineage| {
                lineage.latest_hash.clone_from(&stale);
            })
            .await?;
        let ref_file = paths.package_ref(&uri.namespace);
        let (before, told_before) = (
            std::fs::read(&ref_file)?,
            told.load(std::sync::atomic::Ordering::SeqCst),
        );
        assert_eq!(package.lineage().await?.latest_hash, stale);

        package
            .pull(
                Some(crate::io::remote::HostConfig::default()),
                lineage::SyncScope::IndividualFiles,
            )
            .await?;

        assert_eq!(package.lineage().await?.latest_hash, uri.hash);
        assert_eq!(std::fs::read(&ref_file)?, before);
        assert_eq!(told.load(std::sync::atomic::Ordering::SeqCst), told_before);
        Ok(())
    }

    /// A domain under `root/domain` whose `data.json` holds `acme/x`, with
    /// `a.txt`, its home `root/home`.
    async fn on_data_json(root: &Path) -> Res<LocalDomain> {
        let source = root.join("source");
        std::fs::create_dir_all(&source)?;
        std::fs::write(source.join("a.txt"), b"a")?;
        let domain = LocalDomain::new(root.join("domain"))
            .with_lineage_store(lineage::LineageStore::DataJson);
        domain.set_home(root.join("home")).await?;
        domain
            .create_package(
                ("acme", "x").into(),
                Some(source),
                None,
                flow::UserMeta::Clear,
            )
            .await?;
        Ok(domain)
    }

    fn is_stub(path: &Path) -> bool {
        std::fs::read(path).is_ok_and(|bytes| lineage::refs::state::is_stub(&bytes))
    }

    /// The domain config as a rollback killed after its first step leaves
    /// it, at `home`.
    fn rolling_back(paths: &paths::DomainPaths, home: &Path) -> Res<()> {
        let archive = lineage::refs::state::archive_name(uuid::Uuid::new_v4());
        let config = serde_json::json!({
            "format": 1,
            "home": home,
            "migration": {"rolling_back": {"archive": archive}},
        });
        std::fs::write(paths.domain_config(), serde_json::to_vec(&config)?)?;
        Ok(())
    }

    /// The move takes each package's lock, so a writer moves the domain
    /// before it takes its own.
    #[test(tokio::test)]
    async fn a_commit_that_is_a_domain_s_first_use_moves_it_to_refs_and_commits() -> Res<()> {
        let root = TempDir::new()?;
        on_data_json(root.path()).await?;
        std::fs::write(root.path().join("home/acme/x/b.txt"), b"b")?;
        let domain = LocalDomain::new(root.path().join("domain"));
        let package = domain.create_installed_package(("acme", "x").into());

        let committed = tokio::time::timeout(
            Duration::from_secs(30),
            package.commit("b".to_string(), flow::UserMeta::Keep, None, None),
        )
        .await;

        assert!(
            matches!(committed, Ok(Ok(flow::Committed::Recorded(_)))),
            "{committed:?}"
        );
        assert!(is_stub(&domain.paths.lineage()));
        let lineage = package.lineage().await?;
        assert!(
            lineage.paths.contains_key(Path::new("b.txt")),
            "{lineage:?}"
        );
        Ok(())
    }

    /// A domain first used while a rollback runs is refused at once, and is
    /// not moved back to refs once the rollback lets go of the domain.
    #[test(tokio::test)]
    async fn a_first_use_during_a_rollback_is_refused_without_waiting_for_it() -> Res<()> {
        let root = TempDir::new()?;
        on_data_json(root.path()).await?;
        let moved = LocalDomain::new(root.path().join("domain"));
        moved.list_installed_packages().await?;
        let rollback = LocalStorage::new()
            .lock_exclusive(moved.paths.migrate_lock())
            .await?;
        rolling_back(&moved.paths, &root.path().join("home"))?;
        let domain = LocalDomain::new(root.path().join("domain"));

        let listed =
            tokio::time::timeout(Duration::from_secs(5), domain.list_installed_packages()).await;

        assert!(
            matches!(
                listed,
                Ok(Err(Error::Lineage(LineageError::RollingBack { .. })))
            ),
            "{listed:?}"
        );
        drop(rollback);
        Ok(())
    }

    /// Only `quilt lineage rollback` finishes a rollback that was
    /// interrupted; every other operation is refused, naming it.
    #[test(tokio::test)]
    async fn an_interrupted_rollback_refuses_every_other_operation_until_it_is_finished() -> Res<()>
    {
        let root = TempDir::new()?;
        on_data_json(root.path()).await?;
        let moved = LocalDomain::new(root.path().join("domain"));
        moved.list_installed_packages().await?;
        rolling_back(&moved.paths, &root.path().join("home"))?;
        let domain = LocalDomain::new(root.path().join("domain"));

        let refused = [
            domain.get_home().await.map(drop),
            domain.list_installed_packages().await.map(drop),
            domain
                .create_installed_package(("acme", "x").into())
                .commit("c".to_string(), flow::UserMeta::Keep, None, None)
                .await
                .map(drop),
            domain.uninstall_package(("acme", "x").into()).await,
        ];
        domain.rollback_lineage().await?;

        for result in refused {
            assert!(
                matches!(
                    &result,
                    Err(err @ Error::Lineage(LineageError::RollingBack { .. }))
                        if err.to_string().contains("quilt lineage rollback")
                ),
                "{result:?}"
            );
        }
        let rolled = LocalDomain::new(root.path().join("domain"));
        assert_eq!(rolled.list_installed_packages().await?.len(), 1);
        Ok(())
    }

    /// A domain value that has found the domain on refs never moves it again,
    /// so an app left running does not undo a rollback another process ran.
    #[test(tokio::test)]
    async fn a_domain_rolled_back_under_a_running_app_is_not_moved_again() -> Res<()> {
        let root = TempDir::new()?;
        on_data_json(root.path()).await?;
        let running = LocalDomain::new(root.path().join("domain"));
        running.list_installed_packages().await?;
        LocalDomain::new(root.path().join("domain"))
            .rollback_lineage()
            .await?;

        let listed = running.list_installed_packages().await;

        assert!(listed.is_err(), "{listed:?}");
        assert!(!is_stub(&running.paths.lineage()));
        assert!(!running.paths.domain_config().exists());
        Ok(())
    }

    /// Two first uses at once by clones of one domain value move the domain
    /// once: a rollback that lands between them is not undone by the second.
    #[test(tokio::test(flavor = "multi_thread"))]
    async fn clones_used_first_at_once_move_the_domain_once() -> Res<()> {
        let root = TempDir::new()?;
        on_data_json(root.path()).await?;
        let running = LocalDomain::new(root.path().join("domain"));
        let held = LocalStorage::new()
            .lock_exclusive(running.paths.migrate_lock())
            .await?;
        let settle = || tokio::time::sleep(Duration::from_millis(200));
        let first = tokio::spawn({
            let running = running.clone();
            async move { running.list_installed_packages().await.map(drop) }
        });
        settle().await;
        let rollback = tokio::spawn({
            let domain = LocalDomain::new(root.path().join("domain"));
            async move {
                // Refused if it gets the lock before the first move.
                loop {
                    match domain.rollback_lineage().await {
                        Err(Error::Lineage(LineageError::RollbackRefused(_))) => settle().await,
                        rolled => break rolled.map(drop),
                    }
                }
            }
        });
        settle().await;
        let second = tokio::spawn({
            let running = running.clone();
            async move { running.list_installed_packages().await.map(drop) }
        });
        settle().await;

        drop(held);
        let (first, rollback, second) = tokio::time::timeout(Duration::from_secs(30), async {
            (first.await, rollback.await, second.await)
        })
        .await
        .expect("no task waits forever");
        rollback.expect("the task ran")?;
        // Each listed before the rollback, or refused after it.
        for listed in [first, second] {
            let listed = listed.expect("the task ran");
            assert!(
                matches!(listed, Ok(()) | Err(Error::Lineage(_))),
                "{listed:?}"
            );
        }

        assert!(!is_stub(&running.paths.lineage()));
        assert!(!running.paths.domain_config().exists());
        Ok(())
    }

    /// Reading the home moves nothing: a domain not yet on refs, or one whose
    /// move stopped before its config, is read where its home is now.
    #[test(tokio::test)]
    async fn the_stored_home_of_a_domain_not_yet_on_refs_is_read_without_moving_it() -> Res<()> {
        for stopped_after_the_stub in [false, true] {
            let root = TempDir::new()?;
            let paths = on_data_json(root.path()).await?.paths;
            if stopped_after_the_stub {
                std::fs::rename(paths.lineage(), paths.lineage_migrating())?;
                std::fs::write(paths.lineage(), lineage::refs::state::STUB)?;
            }
            let files = |paths: &paths::DomainPaths| {
                [
                    paths.lineage(),
                    paths.lineage_migrating(),
                    paths.domain_config(),
                ]
                .map(|path| std::fs::read(path).ok())
            };
            let before = files(&paths);
            let domain = LocalDomain::new(root.path().join("domain"));

            let home = domain.stored_home().await;

            assert!(
                matches!(&home, Ok(Some(home)) if *home == Home::from(root.path().join("home"))),
                "{stopped_after_the_stub}: {home:?}"
            );
            assert_eq!(files(&paths), before, "{stopped_after_the_stub}");
        }
        Ok(())
    }

    /// An install or a create that would be refused its ref downloads and
    /// places nothing first.
    #[test(tokio::test)]
    async fn an_install_or_a_create_is_refused_before_it_places_anything_once_a_rollback_began()
    -> Res<()> {
        let root = TempDir::new()?;
        let (remote, uri) = remote_with_one_revision().await?;
        let paths = paths::DomainPaths::new(root.path().join("domain"));
        let domain = LocalDomain::with_parts(
            paths.clone(),
            lineage::DomainLineageIo::new(paths.clone()),
            LocalStorage::new(),
            Arc::new(remote),
        );
        domain.set_home(root.path().join("home")).await?;
        rolling_back(&paths, &root.path().join("home"))?;
        let source = root.path().join("source");
        std::fs::create_dir_all(&source)?;
        std::fs::write(source.join("a.txt"), b"a")?;

        let installed = domain.install_package(&uri).await.map(drop);
        let created = domain
            .create_package(
                ("acme", "y").into(),
                Some(source),
                None,
                flow::UserMeta::Clear,
            )
            .await
            .map(drop);

        for result in [installed, created] {
            assert!(
                matches!(
                    result,
                    Err(Error::Lineage(LineageError::RefsNotWritable { .. }))
                ),
                "{result:?}"
            );
        }
        assert!(!paths.cached_manifest(&uri).exists());
        for namespace in [uri.namespace, ("acme", "y").into()] {
            assert!(!paths.installed_manifests_dir(&namespace).exists());
            assert!(
                !root
                    .path()
                    .join("home")
                    .join(namespace.to_string())
                    .exists()
            );
        }
        Ok(())
    }

    /// A temp a killed write left is swept at start and by gc, which holds
    /// the package's lock already.
    #[test(tokio::test)]
    async fn leftover_temps_are_swept_at_start_and_by_gc() -> Res<()> {
        let root = TempDir::new()?;
        let domain = domain(root.path()).await?;
        domain
            .create_package(("acme", "x").into(), None, None, flow::UserMeta::Clear)
            .await?;
        let owner = domain.paths.refs_dir().join("acme");
        let leftover = || {
            let path = owner.join(format!("x.json.tmp-{}", uuid::Uuid::new_v4()));
            std::fs::write(&path, b"{").map(|()| path)
        };

        let at_start = leftover()?;
        domain.sweep_leftovers().await?;
        assert!(!at_start.exists());

        let before_gc = leftover()?;
        domain.gc().await?;
        assert!(!before_gc.exists());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let dot_quilt = domain.paths.dot_quilt_dir();
            let stuck = [
                leftover()?,
                dot_quilt.join(format!("domain.json.tmp-{}", uuid::Uuid::new_v4())),
            ];
            std::fs::write(&stuck[1], b"{")?;
            let mode = |mode| std::fs::Permissions::from_mode(mode);
            std::fs::set_permissions(&owner, mode(0o500))?;
            std::fs::set_permissions(&dot_quilt, mode(0o500))?;
            // Root removes them despite the modes.
            let enforced = std::fs::write(owner.join("probe"), b"").is_err();
            let swept = domain.sweep_leftovers().await;
            let collected = domain.gc().await;
            std::fs::set_permissions(&dot_quilt, mode(0o700))?;
            std::fs::set_permissions(&owner, mode(0o700))?;
            if enforced {
                assert!(stuck.iter().all(|path| path.exists()), "they stay");
            }
            assert!(swept.is_ok(), "a temp that stays fails no start: {swept:?}");
            assert!(
                collected.is_ok(),
                "a temp that stays stops no gc: {collected:?}"
            );
            for path in stuck {
                let _ = std::fs::remove_file(path);
            }
        }
        assert!(
            domain
                .lineage
                .contains(&domain.storage, &("acme", "x").into())
                .await?
        );
        Ok(())
    }
}

#[cfg(test)]
#[path = "local_domain"]
mod on_data_json {
    use super::*;

    const BACKEND: lineage::LineageStore = lineage::LineageStore::DataJson;

    mod backend_tests;
}
#[cfg(test)]
#[path = "local_domain"]
#[expect(clippy::duplicate_mod, reason = "the tests run once on each store")]
mod on_refs {
    use super::*;

    const BACKEND: lineage::LineageStore = lineage::LineageStore::Refs;

    mod backend_tests;
}

#[cfg(test)]
mod lineage_timing;
