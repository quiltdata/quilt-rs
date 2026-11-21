//! A package a dataspace declares, opened at the revision its lock holds:
//! quilt3's `Package.browse(name)` with no registry (quilt-specs#297's
//! browse-revision: the last local commit, else the base).

use std::path::Path;

use thiserror::Error;

use crate::dataspace::EntryError;
use crate::dataspace::contract::Contract;
use crate::dataspace::contract::ContractError;
use crate::dataspace::contract::Entry;
use crate::dataspace::contract::PackageKey;
use crate::dataspace::contract::Registry;
use crate::dataspace::lock::Lock;
use crate::dataspace::lock::LockError;
use crate::dataspace::lock::Writer;
use crate::dataspace::store::Store;
use crate::dataspace::sync::manifest_at;
use crate::dataspace::sync::pending_move;
use crate::domain::DomainRoot;
use crate::io::remote::Remote;
use crate::io::storage::Storage;
use crate::manifest::Manifest;

/// What [`browse`] opened.
#[derive(Debug)]
#[non_exhaustive]
pub struct Browsed {
    /// The package's entry, as the contract expands it. Where its move to
    /// its bucket has not landed, the revision opened is still the local
    /// registry's.
    pub entry: Entry,
    /// The revision opened: the lock's last commit of the package, else its
    /// base.
    pub hash: String,
    /// The lock's base: the registry revision the package was last read or
    /// published at, which a push onto that registry builds on.
    pub base: Option<String>,
    pub manifest: Manifest,
}

/// Why nothing was opened.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum BrowseError {
    #[error(transparent)]
    Contract(Box<ContractError>),

    #[error(transparent)]
    Lock(Box<LockError>),

    #[error("{0} is not a package of this dataspace")]
    NotDeclared(Box<PackageKey>),

    #[error("{0} has no revision in the lock yet; sync or install it first")]
    NotRead(Box<PackageKey>),

    #[error(transparent)]
    Entry(#[from] EntryError),
}

impl From<ContractError> for BrowseError {
    fn from(err: ContractError) -> Self {
        Self::Contract(Box::new(err))
    }
}

impl From<LockError> for BrowseError {
    fn from(err: LockError) -> Self {
        Self::Lock(Box::new(err))
    }
}

/// Opens `package` as the dataspace in the folder `dataspace` has it: the
/// manifest of the last commit its lock names, else of its base, from the
/// package's store in `domain`, else fetched from its bucket. A local
/// registry renamed since the contract or the lock named it is still
/// found, and a package moved to its bucket whose move has not landed is
/// opened where its commits are, in the local registry. Nothing in the
/// dataspace is written.
///
/// # Errors
///
/// A contract or lock that cannot be read, a package the contract does not
/// declare or the lock holds no revision of, and a manifest that cannot be
/// read.
pub async fn browse(
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    domain: &DomainRoot,
    dataspace: &Path,
    writer: &Writer,
    package: &PackageKey,
) -> Result<Browsed, BrowseError> {
    let mut contract = Contract::read(storage, dataspace, writer.local_registry_name()).await?;
    let mut lock = Lock::read(storage, &contract).await?;
    let mut wanted = package.clone();
    let mut writer = writer.clone();
    if let Some(names) = domain.local().read_names(storage).await.ok().flatten() {
        // The local registry a pending move left the package's commits in
        // is this Domain's, as `commit` follows it.
        writer.local_registry.clone_from(&names.name);
        contract.name_local(&names)?;
        if let Some(lock) = &mut lock {
            lock.name_local(&names);
        }
        if let Registry::Local(name) = &wanted.registry
            && names.answers_to(name)
        {
            wanted.registry = Registry::Local(names.name.clone());
        }
    }
    let entry = contract
        .entries()
        .iter()
        .find(|entry| entry.key() == wanted)
        .cloned()
        .ok_or_else(|| BrowseError::NotDeclared(Box::new(wanted.clone())))?;
    // A package `data.toml` moved to its bucket, whose move has not landed,
    // is read where its commits are: under the local registry's key.
    let held = lock.as_ref().map_or_else(
        || entry.clone(),
        |lock| pending_move(lock, &contract, &writer, entry.clone()),
    );
    let locked = lock
        .as_ref()
        .and_then(|lock| lock.packages.get(&held.key()));
    let Some((hash, base, commits)) = locked.and_then(|locked| {
        let hash = locked.commits.last().or(locked.base.as_ref())?;
        Some((hash.clone(), locked.base.clone(), locked.commits.clone()))
    }) else {
        return Err(BrowseError::NotRead(Box::new(wanted)));
    };
    let store = Store::of(storage, domain, &held).await?;
    let manifest = manifest_at(storage, remote, &store, &held, &hash, &commits).await?;
    Ok(Browsed {
        entry,
        hash,
        base,
        manifest,
    })
}

#[cfg(test)]
mod tests;
