//! The app's engine on quilt-rs's dataspace API, on by default since the
//! cutover.
//!
//! The switch is read once when the app starts
//! ([`crate::engine::services::Services::init`]). On, the app imports what
//! the ref engine kept, once, then opens the Domain
//! [`quilt_rs::domain::locate`] names and its default dataspace, and the
//! commands that have a dataspace counterpart route through [`Dataspace`].
//! `"dataspace_opt_out": true` in `experimental_settings.json`, or an import
//! that falls back, leaves it off: the engine then runs on the ref API
//! (`LocalDomain`) as before, and nothing here is built.
//!
//! A command names a package by its namespace, as the pages do; here that
//! is the first entry of the default dataspace's `data.toml` that declares
//! it (see [`Dataspace::entry`]).

use std::collections::BTreeMap;
use std::fmt::Display;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::RwLock;

use quilt_rs::dataspace::autosync::Runner;
use quilt_rs::dataspace::autosync::Settings as RunnerSettings;
use quilt_rs::dataspace::certify_latest::Certified;
use quilt_rs::dataspace::commit::Committed;
use quilt_rs::dataspace::conflict::ConflictMap;
use quilt_rs::dataspace::contract::Contract;
use quilt_rs::dataspace::contract::Entry;
use quilt_rs::dataspace::contract::PackageKey;
use quilt_rs::dataspace::contract::Registry;
use quilt_rs::dataspace::create::Create;
use quilt_rs::dataspace::create::Created;
use quilt_rs::dataspace::gc::Collected;
use quilt_rs::dataspace::history::Revision;
use quilt_rs::dataspace::home::Home;
use quilt_rs::dataspace::install::Install;
use quilt_rs::dataspace::install::Installed;
use quilt_rs::dataspace::keep::Keep;
use quilt_rs::dataspace::keep::Kept;
use quilt_rs::dataspace::list::DeclaredPackage;
use quilt_rs::dataspace::lock::Writer;
use quilt_rs::dataspace::measure::DomainSize;
use quilt_rs::dataspace::prune::Pruned;
use quilt_rs::dataspace::reattach::Unattached;
use quilt_rs::dataspace::remove_revisions::Removed;
use quilt_rs::dataspace::rename::Renamed;
use quilt_rs::dataspace::reset_to_latest::Reset;
use quilt_rs::dataspace::status::EntryStatus;
use quilt_rs::dataspace::sync::DataspaceSync;
use quilt_rs::dataspace::undo_commit::Undone;
use quilt_rs::dataspace::uninstall::Uninstalled;
use quilt_rs::domain::DomainRoot;
use quilt_rs::io::remote::Remote;
use quilt_rs::io::remote::RemoteS3;
use quilt_rs::io::storage::LocalStorage;
use quilt_rs::io::storage::Storage;
use quilt_rs::manifest::Manifest;
use quilt_rs::queue::Queue;
use quilt_rs::quiltignore::Rules;
use quilt_uri::Host;
use quilt_uri::Namespace;

use crate::engine::error::Error;
use crate::engine::telemetry::prelude::*;

/// The message of each revision the autosync tick commits.
pub const AUTOSYNC_MESSAGE: &str = "Autosync";

pub(crate) fn fail(err: impl Display) -> Error {
    Error::Dataspace(err.to_string())
}

/// The Domain and the default dataspace the app acts on with the switch
/// on, and what every dataspace call takes.
pub struct Dataspace<R: Remote = RemoteS3> {
    storage: LocalStorage,
    remote: Arc<R>,
    domain: DomainRoot,
    app_dir: PathBuf,
    /// The default dataspace's folder.
    home: RwLock<PathBuf>,
    writer: Writer,
    user_rules: PathBuf,
    runner: Runner,
    /// Held across a read and write of `conflict.json`, so two decisions
    /// made at once both stand.
    conflict_edits: tokio::sync::Mutex<()>,
    /// Held by a sync, the autosync tick, and a task's commit and sync, so
    /// a commit never lands while a tick holds the lock in memory to write
    /// it whole.
    writes: tokio::sync::Mutex<()>,
}

impl<R: Remote + Sync> Dataspace<R> {
    /// The Domain at `domain_dir`, and the default dataspace the app whose
    /// folder is `app_dir` chose; where it chose none, `fallback` is chosen,
    /// as the ref path's home folder is. `user_rules` is the user-wide
    /// ignore rules file.
    pub async fn open(
        remote: Arc<R>,
        domain_dir: PathBuf,
        app_dir: PathBuf,
        fallback: &Path,
        user_rules: PathBuf,
    ) -> Result<Self, Error> {
        let storage = LocalStorage::new();
        let domain = DomainRoot::new(domain_dir);
        let chosen = quilt_rs::dataspace::home::default_dataspace(&storage, &app_dir)
            .await
            .map_err(fail)?;
        let dataspace = match chosen {
            Some(dataspace) => dataspace,
            None => {
                quilt_rs::dataspace::home::set_home(&storage, &app_dir, fallback)
                    .await
                    .map_err(fail)?
                    .dataspace
            }
        };
        let name = domain.local().names(&storage).await.map_err(fail)?.name;
        let writer = Writer::here(name).map_err(fail)?;
        let runner = Runner::new(
            Queue::open(&domain.queue_db()).await.map_err(fail)?,
            RunnerSettings {
                domain: domain.clone(),
                writer: writer.clone(),
                user_rules: user_rules.clone(),
                message: AUTOSYNC_MESSAGE.to_string(),
            },
        );
        info!("The dataspace engine acts on {}", dataspace.display());
        Ok(Self {
            storage,
            remote,
            domain,
            app_dir,
            home: RwLock::new(dataspace),
            writer,
            user_rules,
            runner,
            conflict_edits: tokio::sync::Mutex::new(()),
            writes: tokio::sync::Mutex::new(()),
        })
    }

    /// Waits for a sync or tick under way to end, and keeps another from
    /// starting until the guard drops.
    pub async fn hold_writes(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.writes.lock().await
    }

    /// The default dataspace's folder.
    pub fn dataspace(&self) -> PathBuf {
        self.home.read().map_or_else(
            |poisoned| poisoned.into_inner().clone(),
            |held| held.clone(),
        )
    }

    pub fn domain(&self) -> &DomainRoot {
        &self.domain
    }

    pub fn runner(&self) -> &Runner {
        &self.runner
    }

    pub fn remote(&self) -> &R {
        &self.remote
    }

    pub fn storage(&self) -> &LocalStorage {
        &self.storage
    }

    /// The default dataspace's contract, its local registry named as this
    /// machine's is.
    pub async fn contract(&self) -> Result<Contract, Error> {
        Contract::read(
            &self.storage,
            &self.dataspace(),
            self.writer.local_registry_name(),
        )
        .await
        .map_err(fail)
    }

    /// The first entry that declares `namespace`, in the contract's order.
    /// A package declared in two registries, or under `read` and `write`
    /// over disjoint paths, is named by the first.
    pub async fn entry(&self, namespace: &Namespace) -> Result<Entry, Error> {
        let contract = self.contract().await?;
        let first = contract
            .entries()
            .iter()
            .find(|entry| entry.package == *namespace)
            .cloned();
        first.ok_or_else(|| {
            Error::Dataspace(format!(
                "{namespace} is not declared in {}",
                self.dataspace().join("data.toml").display()
            ))
        })
    }

    async fn key(&self, namespace: &Namespace) -> Result<PackageKey, Error> {
        Ok(self.entry(namespace).await?.key())
    }

    /// The package lock of `key`'s store in the Domain.
    fn package_lock(&self, key: &PackageKey) -> Result<PathBuf, Error> {
        Ok(match &key.registry {
            Registry::S3(bucket) => self
                .domain
                .registry(bucket)
                .map_err(fail)?
                .package_lock(&key.package),
            // The local registry, the one other store.
            _ => self.domain.local().package_lock(&key.package),
        })
    }

    /// The catalog each bucket's entries name, for the calls that reach a
    /// bucket with no entry at hand.
    async fn catalogs(&self) -> BTreeMap<String, Host> {
        let mut catalogs = BTreeMap::new();
        if let Ok(contract) = self.contract().await {
            for entry in contract.entries() {
                if let (Registry::S3(bucket), Some(host)) = (&entry.registry, &entry.catalog) {
                    catalogs
                        .entry(bucket.clone())
                        .or_insert_with(|| host.clone());
                }
            }
        }
        catalogs
    }

    /// The rules the tick's commit packages `folder` with: the folder's
    /// `.quiltignore`, the user's, and what the contract's `[ignore]` table
    /// sets.
    pub async fn ignore_rules(&self, folder: &Path) -> Result<Rules, Error> {
        let policy = self.contract().await?.ignore_policy(folder);
        Rules::load_with(folder, &self.user_rules, &policy).map_err(fail)
    }

    pub async fn install(&self, request: &Install) -> Result<Installed, Error> {
        quilt_rs::dataspace::install::install(
            &self.storage,
            &*self.remote,
            Some(&self.domain.local()),
            &self.dataspace(),
            &self.writer,
            request,
        )
        .await
        .map_err(fail)
    }

    /// Uninstalls `namespace` holding its package lock, so a tick running
    /// on it ends first and the next finds it gone; then, where `prune`,
    /// clears its published content from its bucket's store, which takes the
    /// package locks itself.
    pub async fn uninstall(
        &self,
        namespace: &Namespace,
        prune: bool,
    ) -> Result<(Uninstalled, Option<Pruned>), Error> {
        let entry = self.entry(namespace).await?;
        let key = entry.key();
        let uninstalled = {
            let _held = self
                .storage
                .lock_exclusive(self.package_lock(&key)?)
                .await
                .map_err(fail)?;
            quilt_rs::dataspace::uninstall::uninstall(
                &self.storage,
                Some(&self.domain.local()),
                &self.dataspace(),
                &self.writer,
                &key,
            )
            .await
            .map_err(fail)?
        };
        let pruned = if prune {
            quilt_rs::dataspace::prune::prune(
                &self.storage,
                &*self.remote,
                &self.domain,
                &key,
                entry.catalog.as_ref(),
            )
            .await
            .map_err(|err| tracing::warn!("Failed to prune {key} after uninstalling it: {err}"))
            .ok()
        } else {
            None
        };
        Ok((uninstalled, pruned))
    }

    /// The status of the entry [`Self::entry`] names `namespace` by.
    pub async fn entry_status(
        &self,
        namespace: &Namespace,
        fetch: bool,
    ) -> Result<EntryStatus, Error> {
        let entry = self.entry(namespace).await?;
        // That entry's alone: no other folder is walked, no other registry
        // asked.
        quilt_rs::dataspace::status::entry_status(
            &self.storage,
            &*self.remote,
            Some(&self.domain),
            Some(self.runner.queue()),
            &self.dataspace(),
            &self.writer,
            &self.user_rules,
            &entry,
            fetch,
        )
        .await
        .map_err(fail)?
        .ok_or_else(|| Error::Dataspace(format!("no status for {namespace}")))
    }

    pub async fn list(&self, fetch: bool) -> Result<Vec<DeclaredPackage>, Error> {
        quilt_rs::dataspace::list::list(
            &self.storage,
            &*self.remote,
            Some(&self.domain),
            Some(self.runner.queue()),
            &self.dataspace(),
            &self.writer,
            &self.user_rules,
            fetch,
        )
        .await
        .map_err(fail)
    }

    pub async fn history(&self, namespace: &Namespace) -> Result<Vec<Revision>, Error> {
        let key = self.key(namespace).await?;
        quilt_rs::dataspace::history::history(
            &self.storage,
            &*self.remote,
            Some(&self.domain),
            &self.dataspace(),
            &self.writer,
            &key,
        )
        .await
        .map_err(fail)
    }

    pub async fn commit(&self, namespace: &Namespace, message: &str) -> Result<Committed, Error> {
        let key = self.key(namespace).await?;
        quilt_rs::dataspace::commit::commit(
            &self.storage,
            &*self.remote,
            &self.domain,
            &self.dataspace(),
            &self.writer,
            &key,
            message,
            &self.user_rules,
        )
        .await
        .map_err(fail)
    }

    pub async fn undo(&self, namespace: &Namespace) -> Result<Undone, Error> {
        let key = self.key(namespace).await?;
        quilt_rs::dataspace::undo_commit::undo_commit(
            &self.storage,
            &*self.remote,
            &self.domain,
            &self.dataspace(),
            &self.writer,
            &key,
        )
        .await
        .map_err(fail)
    }

    pub async fn reset(&self, namespace: &Namespace) -> Result<Reset, Error> {
        let key = self.key(namespace).await?;
        quilt_rs::dataspace::reset_to_latest::reset_to_latest(
            &self.storage,
            &*self.remote,
            &self.domain,
            &self.dataspace(),
            &self.writer,
            &key,
        )
        .await
        .map_err(fail)
    }

    pub async fn certify(&self, namespace: &Namespace) -> Result<Certified, Error> {
        let key = self.key(namespace).await?;
        quilt_rs::dataspace::certify_latest::certify_latest(
            &self.storage,
            &*self.remote,
            &self.domain,
            self.runner.queue(),
            &self.dataspace(),
            &self.writer,
            &key,
        )
        .await
        .map_err(fail)
    }

    pub async fn remove_revisions(
        &self,
        namespace: &Namespace,
        revisions: &[String],
    ) -> Result<Removed, Error> {
        let key = self.key(namespace).await?;
        quilt_rs::dataspace::remove_revisions::remove_revisions(
            &self.storage,
            &*self.remote,
            Some(&self.domain),
            &self.dataspace(),
            &self.writer,
            &key,
            revisions,
        )
        .await
        .map_err(fail)
    }

    /// Changes what the dataspace keeps of a package: its paths, its pin or
    /// its access, an edit to `data.toml`.
    pub async fn keep(&self, request: &Keep) -> Result<Kept, Error> {
        quilt_rs::dataspace::keep::keep(
            &self.storage,
            &*self.remote,
            Some(&self.domain.local()),
            &self.dataspace(),
            &self.writer,
            request,
        )
        .await
        .map_err(fail)
    }

    /// The key [`Self::entry`] names `namespace` by, for a [`Keep`].
    pub async fn package_key(&self, namespace: &Namespace) -> Result<PackageKey, Error> {
        self.key(namespace).await
    }

    pub async fn create(&self, request: &Create) -> Result<Created, Error> {
        quilt_rs::dataspace::create::create(
            &self.storage,
            &*self.remote,
            &self.domain,
            &self.dataspace(),
            &self.writer,
            request,
            &self.user_rules,
        )
        .await
        .map_err(fail)
    }

    /// Makes `folder` the default dataspace, a dataspace if it is not one.
    pub async fn set_home(&self, folder: &Path) -> Result<Home, Error> {
        let home = quilt_rs::dataspace::home::set_home(&self.storage, &self.app_dir, folder)
            .await
            .map_err(fail)?;
        let old = self.dataspace();
        match self.home.write() {
            Ok(mut held) => held.clone_from(&home.dataspace),
            Err(poisoned) => poisoned.into_inner().clone_from(&home.dataspace),
        }
        // The runner ticks what is on autosync: the new home is put on it
        // by `Services::rewatch_dataspace`, the old one leaves it.
        if old != home.dataspace
            && let Err(err) = self.domain.set_autosync(&self.storage, &old, false).await
        {
            tracing::warn!("The old dataspace could not leave autosync: {err}");
        }
        Ok(home)
    }

    pub async fn gc(&self) -> Result<Collected, Error> {
        let catalogs = self.catalogs().await;
        quilt_rs::dataspace::gc::gc(&self.storage, &*self.remote, &self.domain, |bucket| {
            catalogs.get(bucket).cloned()
        })
        .await
        .map_err(fail)
    }

    pub async fn gc_estimate(&self) -> Result<Collected, Error> {
        let catalogs = self.catalogs().await;
        quilt_rs::dataspace::gc::gc_estimate(&self.storage, &*self.remote, &self.domain, |bucket| {
            catalogs.get(bucket).cloned()
        })
        .await
        .map_err(fail)
    }

    pub async fn measure(&self) -> Result<DomainSize, Error> {
        quilt_rs::dataspace::measure::measure_storage(&self.storage, &self.domain)
            .await
            .map_err(fail)
    }

    /// Syncs the default dataspace, as the sync tick does: reads what
    /// `latest` moved, and commits and pushes `sync` entries.
    pub async fn sync(&self, message: &str) -> Result<DataspaceSync, Error> {
        let _writes = self.hold_writes().await;
        Box::pin(self.sync_held(message)).await
    }

    /// Writes `user_meta` as the package's metadata in `folder`, commits it
    /// with `message`, then syncs, with no tick between: a task's claim or
    /// outcome. The commit's failure, as with nothing to commit, is logged
    /// and the sync still runs.
    pub async fn commit_and_sync(
        &self,
        namespace: &Namespace,
        folder: &std::path::Path,
        user_meta: &serde_json::Value,
        message: &str,
    ) -> Result<DataspaceSync, Error> {
        let _writes = self.hold_writes().await;
        quilt_rs::dataspace::set_package_metadata(&self.storage, folder, user_meta)
            .await
            .map_err(fail)?;
        if let Err(err) = self.commit(namespace, message).await {
            debug!("{namespace}: nothing committed before the sync: {err}");
        }
        Box::pin(self.sync_held(AUTOSYNC_MESSAGE)).await
    }

    /// [`Self::sync`], for a caller holding [`Self::hold_writes`].
    async fn sync_held(&self, message: &str) -> Result<DataspaceSync, Error> {
        quilt_rs::dataspace::sync::sync(
            &self.storage,
            &*self.remote,
            &self.domain,
            self.runner.queue(),
            &self.dataspace(),
            &self.writer,
            message,
            &self.user_rules,
        )
        .await
        .map_err(fail)
    }

    /// Reads every `read` entry of the default dataspace.
    pub async fn read(&self) -> Result<quilt_rs::dataspace::read_all::DataspaceRead, Error> {
        quilt_rs::dataspace::read_all::read_all(
            &self.storage,
            &*self.remote,
            Some(&self.domain.local()),
            &self.dataspace(),
            &self.writer,
        )
        .await
        .map_err(fail)
    }

    /// Publishes every `write` and `replace` entry of the default dataspace.
    pub async fn publish(
        &self,
    ) -> Result<quilt_rs::dataspace::publish_all::DataspacePublish, Error> {
        quilt_rs::dataspace::publish_all::publish_all(
            &self.storage,
            &*self.remote,
            Some(&self.domain.local()),
            &self.dataspace(),
            &self.writer,
            &self.user_rules,
        )
        .await
        .map_err(fail)
    }

    /// The local commits of `namespace` the lock does not name.
    pub async fn unattached(&self, namespace: &Namespace) -> Result<Vec<Unattached>, Error> {
        let key = self.key(namespace).await?;
        quilt_rs::dataspace::reattach::unattached(
            &self.storage,
            &*self.remote,
            &self.domain,
            &self.dataspace(),
            &self.writer,
            &key,
        )
        .await
        .map_err(fail)
    }

    /// Puts `commit` back into the lock as `namespace`'s last local commit,
    /// on the base the lock holds for it, or on none for a package the lock
    /// does not name.
    pub async fn reattach(&self, namespace: &Namespace, commit: &str) -> Result<(), Error> {
        let key = self.key(namespace).await?;
        let base = self.entry_status(namespace, false).await?.hashes.base;
        let base = base.as_deref();
        quilt_rs::dataspace::reattach::reattach(
            &self.storage,
            &*self.remote,
            &self.domain,
            &self.dataspace(),
            &self.writer,
            &key,
            commit,
            base,
        )
        .await
        .map_err(fail)
    }

    /// Holds the edit of `conflict.json` against another one.
    pub async fn lock_conflict_edits(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.conflict_edits.lock().await
    }

    /// The default dataspace's `conflict.json`.
    pub async fn conflicts(&self) -> Result<ConflictMap, Error> {
        ConflictMap::read(&self.storage, &self.dataspace())
            .await
            .map_err(fail)
    }

    /// Writes `map` as the default dataspace's `conflict.json`: the
    /// decisions the next read or sync validates and takes.
    pub async fn write_conflicts(&self, map: &ConflictMap) -> Result<(), Error> {
        map.write(&self.storage, &self.dataspace())
            .await
            .map_err(fail)
    }

    /// The manifest of revision `hash` of `entry`'s package: from the
    /// Domain's store where it holds one, else from the bucket.
    pub async fn revision_manifest(&self, entry: &Entry, hash: &str) -> Result<Manifest, Error> {
        quilt_rs::dataspace::status::revision_manifest(
            &self.storage,
            &*self.remote,
            Some(&self.domain),
            entry,
            hash,
        )
        .await
        .map_err(fail)
    }

    /// This machine's local registry's name.
    pub async fn local_name(&self) -> Result<String, Error> {
        Ok(self
            .domain
            .local()
            .names(&self.storage)
            .await
            .map_err(fail)?
            .name)
    }

    /// Renames this machine's local registry, its old name kept as an alias,
    /// and moves what the queue keeps under any name it had (25t item 8).
    pub async fn rename(&self, name: &str) -> Result<Renamed, Error> {
        quilt_rs::dataspace::rename::rename(&self.storage, &self.domain, self.runner.queue(), name)
            .await
            .map_err(fail)
    }

    /// Turns autosync on for the default dataspace, so the runner ticks it.
    pub async fn autosync_on(&self) -> Result<(), Error> {
        self.domain
            .set_autosync(&self.storage, &self.dataspace(), true)
            .await
            .map_err(fail)
    }
}

pub mod autosync;
pub mod conflicts;

#[cfg(test)]
pub(crate) mod tests;
