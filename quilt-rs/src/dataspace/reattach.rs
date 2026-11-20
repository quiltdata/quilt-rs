//! A package's local commits that its dataspace's lock does not name, as
//! after the lock was deleted, and attaching one to the lock again.
//!
//! The commits are looked up in the bucket's Package Store under
//! [`DomainRoot`], where a lock's commits are kept. A `LocalDomain`'s
//! `.quilt/installed/` is not searched: its commits are named by their
//! package's ref, never by a lock.

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
use crate::domain::DomainRoot;
use crate::domain::RegistryStore;
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

    #[error("the package is in a local registry, whose commits are not reattached here")]
    LocalRegistry,

    #[error("{LOCK_FILE} already names unpushed commits of the package")]
    Attached,

    #[error("{0} is not a local commit of the package that {LOCK_FILE} leaves out")]
    NotUnattached(String),

    #[error("the base {0:?} is not the whole top hash of a revision the bucket holds")]
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
/// bucket lacks its manifest, so the bucket is asked about each.
pub async fn unattached(
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    domain: &DomainRoot,
    dataspace: &Path,
    writer: &Writer,
    package: &PackageKey,
) -> Result<Vec<Unattached>, ReattachError> {
    let opened = Opened::open(storage, domain, dataspace, writer, package).await?;
    let store = opened.store(domain)?;
    let mut found = Vec::new();
    for (hash, path, _) in
        list_files(storage, &store.installed_manifests_dir(&package.package)).await?
    {
        if opened.is_unattached(remote, &hash).await? {
            let manifest = Manifest::from_path(storage, &path).await?;
            let message = manifest.header.message().map(str::to_string);
            found.push(Unattached { hash, message });
        }
    }
    Ok(found)
}

/// Attaches `commit`, one of [`unattached`]'s, to the lock as `package`'s
/// one unpushed commit on `base`, a revision the bucket holds, which its next
/// publish expects `latest` to be. A lock entry for the package that names no
/// unpushed commits is replaced.
#[allow(clippy::too_many_arguments, reason = "each is a distinct input")]
pub async fn reattach(
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    domain: &DomainRoot,
    dataspace: &Path,
    writer: &Writer,
    package: &PackageKey,
    commit: &str,
    base: &str,
) -> Result<(), ReattachError> {
    if !is_top_hash(base) {
        return Err(ReattachError::Base(base.to_string()));
    }
    let mut opened = Opened::open(storage, domain, dataspace, writer, package).await?;
    if opened
        .lock
        .packages
        .get(package)
        .is_some_and(|entry| !entry.commits.is_empty())
    {
        return Err(ReattachError::Attached);
    }
    // The hash is checked before it names a path.
    let listed = opened.is_unattached(remote, commit).await?
        && storage
            .is_file(
                opened
                    .store(domain)?
                    .installed_manifest(&package.package, commit),
            )
            .await;
    if !listed {
        return Err(ReattachError::NotUnattached(commit.to_string()));
    }
    // A base the bucket lacks could never be `latest`, so no publish would pass.
    if !opened.holds(remote, base).await? {
        return Err(ReattachError::Base(base.to_string()));
    }
    opened.lock.packages.insert(
        package.clone(),
        LockEntry {
            base: base.to_string(),
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
    package: PackageKey,
    bucket: String,
    catalog: Option<Host>,
}

impl Opened {
    /// Refuses a package the contract does not publish, and a lock written
    /// for another dataspace, host or user. One written for another
    /// `data.toml` is used, as `read_all` does: the contract was edited here.
    async fn open(
        storage: &(impl Storage + Sync),
        domain: &DomainRoot,
        dataspace: &Path,
        writer: &Writer,
        package: &PackageKey,
    ) -> Result<Self, ReattachError> {
        // The Domain names this machine's local registry, which a contract's
        // `quilt+local://` with no name stands for.
        let local = domain
            .local()
            .names(storage)
            .await
            .ok()
            .map(|names| names.name);
        let local = local.as_deref().or(writer.local_registry_name());
        let contract = Contract::read(storage, dataspace, local).await?;
        let catalog = contract
            .entries()
            .iter()
            .find(|entry| entry.key() == *package && entry.access != Access::Read)
            .ok_or(ReattachError::Undeclared)?
            .catalog
            .clone();
        let Registry::S3(bucket) = &package.registry else {
            return Err(ReattachError::LocalRegistry);
        };
        let read = Lock::read(storage, &contract).await?;
        // A lock keeps the registry it names; a new one names the registry its
        // keys were resolved with.
        let writer = Writer {
            local_registry: read.as_ref().map_or_else(
                || local.unwrap_or_default().to_string(),
                |lock| lock.header.writer.local_registry.clone(),
            ),
            ..writer.clone()
        };
        let lock = if let Some(lock) = read {
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
        Ok(Self {
            contract,
            lock,
            writer,
            package: package.clone(),
            bucket: bucket.clone(),
            catalog,
        })
    }

    fn store(&self, domain: &DomainRoot) -> Result<RegistryStore, ReattachError> {
        Ok(domain.registry(&self.bucket)?)
    }

    /// Whether `hash` is a top hash the bucket lacks and the lock names as
    /// none of the package's commits. A base is a revision the bucket holds,
    /// so it is not compared.
    async fn is_unattached(&self, remote: &impl Remote, hash: &str) -> Result<bool, ReattachError> {
        let named = self
            .lock
            .packages
            .get(&self.package)
            .is_some_and(|entry| entry.commits.iter().any(|commit| commit == hash));
        if !is_top_hash(hash) || named {
            return Ok(false);
        }
        Ok(!self.holds(remote, hash).await?)
    }

    /// Whether the bucket holds the manifest `hash`.
    async fn holds(&self, remote: &impl Remote, hash: &str) -> Result<bool, ReattachError> {
        let manifest = S3Uri {
            bucket: self.bucket.clone(),
            key: get_manifest_key(hash),
            version: None,
        };
        Ok(remote.exists(self.catalog.as_ref(), &manifest).await?)
    }
}

#[cfg(test)]
mod tests;
