//! A package's local commits that its dataspace's lock does not name, as
//! after the lock was deleted, and attaching one to the lock again.
//!
//! The commits are looked up in the bucket's Package Store under
//! [`DomainRoot`], or in the local registry for its own packages, where a
//! lock's commits are kept. A `LocalDomain`'s `.quilt/installed/` is not
//! searched: its commits are named by their package's ref, never by a lock.

use std::path::Path;

use thiserror::Error;

use crate::dataspace::contract::Access;
use crate::dataspace::contract::CONTRACT_FILE;
use crate::dataspace::contract::Contract;
use crate::dataspace::contract::ContractError;
use crate::dataspace::contract::PackageKey;
use crate::dataspace::contract::Registry;
use crate::dataspace::lock::LOCK_FILE;
use crate::dataspace::lock::Lock;
use crate::dataspace::lock::LockEntry;
use crate::dataspace::lock::LockError;
use crate::dataspace::lock::Mismatch;
use crate::dataspace::lock::Writer;
use crate::dataspace::store::Store;
use crate::domain::DomainRoot;
use crate::flow::list_files;
use crate::io::remote::Remote;
use crate::io::storage::Storage;
use crate::manifest::Manifest;
use quilt_uri::Host;
use quilt_uri::S3Uri;
use quilt_uri::is_top_hash;
use quilt_uri::paths::get_manifest_key;

/// A local commit of a package that the lock does not name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unattached {
    pub hash: String,
    pub message: Option<String>,
}

/// Why the commits could not be listed, or one not attached.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ReattachError {
    #[error(transparent)]
    Contract(Box<ContractError>),

    #[error(transparent)]
    Lock(Box<LockError>),

    #[error(
        "{LOCK_FILE} was written for another {}; fix or remove it first",
        .0.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ")
    )]
    Mismatch(Vec<Mismatch>),

    #[error("{CONTRACT_FILE} does not list the package under `write`, `replace` or `sync`")]
    Undeclared,

    #[error("the package is in a local registry that is not this Domain's")]
    LocalRegistry,

    #[error("{LOCK_FILE} already names unpushed commits of the package")]
    Attached,

    #[error("{0} is not a local commit of the package that {LOCK_FILE} leaves out")]
    NotUnattached(String),

    #[error("the base {0:?} is not the whole top hash of a revision the registry holds")]
    Base(String),

    #[error(transparent)]
    Failed(Box<crate::Error>),
}

impl From<ContractError> for ReattachError {
    fn from(err: ContractError) -> Self {
        Self::Contract(Box::new(err))
    }
}

impl From<LockError> for ReattachError {
    fn from(err: LockError) -> Self {
        Self::Lock(Box::new(err))
    }
}

impl From<crate::Error> for ReattachError {
    fn from(err: crate::Error) -> Self {
        Self::Failed(Box::new(err))
    }
}

/// The local commits of `package` in `domain` that the lock of the dataspace
/// in the folder `dataspace` does not name. A commit is local while its
/// registry lacks its manifest, so the bucket, or the local registry, is
/// asked about each.
pub async fn unattached(
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    domain: &DomainRoot,
    dataspace: &Path,
    writer: &Writer,
    package: &PackageKey,
) -> Result<Vec<Unattached>, ReattachError> {
    let opened = Opened::open(storage, domain, dataspace, writer, package).await?;
    let mut found = Vec::new();
    for (hash, path, _) in list_files(
        storage,
        &opened.store.installed_manifests_dir(&package.package),
    )
    .await?
    {
        if opened.is_unattached(storage, remote, &hash).await? {
            let manifest = Manifest::from_path(storage, &path).await?;
            let message = manifest.header.message().map(str::to_string);
            found.push(Unattached { hash, message });
        }
    }
    Ok(found)
}

/// Attaches `commit`, one of [`unattached`]'s, to the lock as `package`'s
/// one unpushed commit on `base`, a revision its registry holds, which its
/// next publish expects `latest` to be. With no `base`, the commit is of a
/// package never published, whose next publish expects no `latest`. A lock
/// entry for the package that names no unpushed commits is replaced. A
/// package of the local registry, named by one of the names it had, is
/// written under its current name.
#[allow(clippy::too_many_arguments, reason = "each is a distinct input")]
pub async fn reattach(
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    domain: &DomainRoot,
    dataspace: &Path,
    writer: &Writer,
    package: &PackageKey,
    commit: &str,
    base: Option<&str>,
) -> Result<(), ReattachError> {
    if let Some(base) = base.filter(|base| !is_top_hash(base)) {
        return Err(ReattachError::Base(base.to_string()));
    }
    let mut opened = Opened::open(storage, domain, dataspace, writer, package).await?;
    if opened
        .lock
        .packages
        .get(&opened.package)
        .is_some_and(|entry| !entry.commits.is_empty())
    {
        return Err(ReattachError::Attached);
    }
    // The hash is checked before it names a path.
    let listed = opened.is_unattached(storage, remote, commit).await?
        && storage
            .is_file(opened.store.installed_manifest(&package.package, commit))
            .await;
    if !listed {
        return Err(ReattachError::NotUnattached(commit.to_string()));
    }
    // A base the registry lacks could never be `latest`, so no publish would
    // pass.
    if let Some(base) = base
        && !opened.holds(storage, remote, base).await?
    {
        return Err(ReattachError::Base(base.to_string()));
    }
    opened.lock.packages.insert(
        opened.package.clone(),
        LockEntry {
            base: base.map(str::to_string),
            commits: vec![commit.to_string()],
        },
    );
    opened
        .lock
        .write(storage, &opened.contract, &opened.writer)
        .await?;
    Ok(())
}

/// A dataspace's contract and lock, read for one package.
struct Opened {
    contract: Contract,
    lock: Lock,
    /// The writer, keeping the local registry the lock names, since none is
    /// kept yet to compare it with.
    writer: Writer,
    /// The package, a local registry named by its current name.
    package: PackageKey,
    store: Store,
    catalog: Option<Host>,
}

impl Opened {
    /// Refuses a package the contract does not publish, and a lock written
    /// for another dataspace, host or user. One written for another
    /// `data.toml` is used, as `read_all` does: the contract was edited here.
    /// A key naming the Domain's local registry by a name it had is named by
    /// its current one, in the contract, the lock and `package`.
    async fn open(
        storage: &(impl Storage + Sync),
        domain: &DomainRoot,
        dataspace: &Path,
        writer: &Writer,
        package: &PackageKey,
    ) -> Result<Self, ReattachError> {
        // The Domain names this machine's local registry, which a contract's
        // `quilt+local://` with no name stands for.
        let local = domain.local();
        let names = local.names(storage).await.ok();
        let name = names.as_ref().map(|names| names.name.as_str());
        let name = name.or(writer.local_registry_name());
        let mut contract = Contract::read(storage, dataspace, name).await?;
        let mut package = package.clone();
        if let Some(names) = &names {
            contract.name_local(names)?;
            if let Registry::Local(name) = &package.registry
                && names.answers_to(name)
            {
                package.registry = Registry::Local(names.name.clone());
            }
        }
        let catalog = contract
            .entries()
            .iter()
            .find(|entry| entry.key() == package && entry.access != Access::Read)
            .ok_or(ReattachError::Undeclared)?
            .catalog
            .clone();
        let store = match &package.registry {
            Registry::S3(bucket) => Store::Bucket(domain.registry(bucket)?),
            Registry::Local(name) if names.as_ref().is_some_and(|names| names.name == *name) => {
                Store::Local(local)
            }
            Registry::Local(_) => return Err(ReattachError::LocalRegistry),
        };
        let read = Lock::read(storage, &contract).await?;
        // A lock keeps the registry it names; a new one names the registry its
        // keys were resolved with.
        let mut writer = Writer {
            local_registry: read.as_ref().map_or_else(
                || name.unwrap_or_default().to_string(),
                |lock| lock.header.writer.local_registry.clone(),
            ),
            ..writer.clone()
        };
        let mut lock = if let Some(lock) = read {
            let foreign: Vec<Mismatch> = lock
                .mismatches(&contract, &writer)
                .into_iter()
                .filter(|mismatch| *mismatch != Mismatch::DataToml)
                .collect();
            if !foreign.is_empty() {
                return Err(ReattachError::Mismatch(foreign));
            }
            lock
        } else {
            Lock::new(&contract, &writer)
        };
        if let Some(names) = &names {
            lock.name_local(names);
            if names.answers_to(&writer.local_registry) {
                writer.local_registry.clone_from(&names.name);
            }
        }
        Ok(Self {
            contract,
            lock,
            writer,
            package,
            store,
            catalog,
        })
    }

    /// Whether `hash` is a top hash the registry lacks and the lock names as
    /// none of the package's commits. A base is a revision the registry
    /// holds, so it is not compared.
    async fn is_unattached(
        &self,
        storage: &(impl Storage + Sync),
        remote: &impl Remote,
        hash: &str,
    ) -> Result<bool, ReattachError> {
        let named = self
            .lock
            .packages
            .get(&self.package)
            .is_some_and(|entry| entry.commits.iter().any(|commit| commit == hash));
        if !is_top_hash(hash) || named {
            return Ok(false);
        }
        Ok(!self.holds(storage, remote, hash).await?)
    }

    /// Whether the package's registry holds the manifest `hash`: its bucket,
    /// or the local registry, which keeps its revisions beside its commits.
    async fn holds(
        &self,
        storage: &(impl Storage + Sync),
        remote: &impl Remote,
        hash: &str,
    ) -> Result<bool, ReattachError> {
        match &self.store {
            Store::Bucket(store) => {
                let manifest = S3Uri {
                    bucket: store.bucket().to_string(),
                    key: get_manifest_key(hash),
                    version: None,
                };
                Ok(remote.exists(self.catalog.as_ref(), &manifest).await?)
            }
            Store::Local(local) => Ok(storage.is_file(local.revision(hash)).await),
        }
    }
}

#[cfg(test)]
mod tests;
