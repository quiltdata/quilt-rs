//! `uninstall`: a package taken out of a dataspace, its declarations removed
//! from `data.toml` and its entry from the lock, and the working folder a
//! read filled removed, as an uninstall removes it today.

use std::path::Path;
use std::path::PathBuf;

use thiserror::Error;
use tracing::info;

use crate::dataspace::contract::Access;
use crate::dataspace::contract::CONTRACT_FILE;
use crate::dataspace::contract::Contract;
use crate::dataspace::contract::ContractError;
use crate::dataspace::contract::Entry;
use crate::dataspace::contract::PackageKey;
use crate::dataspace::contract::Registry;
use crate::dataspace::edit::ContractEdit;
use crate::dataspace::install::write_lock;
use crate::dataspace::lock::LOCK_FILE;
use crate::dataspace::lock::LockEntry;
use crate::dataspace::lock::LockError;
use crate::dataspace::lock::Mismatch;
use crate::dataspace::lock::Writer;
use crate::dataspace::read_all::usable_lock;
use crate::dataspace::store::name_local;
use crate::domain::LocalRegistry;
use crate::io::storage::Storage;

/// What [`uninstall`] did.
#[derive(Debug)]
#[non_exhaustive]
pub struct Uninstalled {
    /// The package's entries removed from the contract, as it expanded them.
    pub entries: Vec<Entry>,
    /// The working folder, where it was removed.
    pub removed: Option<PathBuf>,
    /// The working folder, where it was left: outside the dataspace, or
    /// under `write` or `replace`, whose files are the user's own.
    pub kept: Option<PathBuf>,
    /// The package's lock entry, removed. Its unpushed commits stay in the
    /// Domain's store; [`reattach`](crate::dataspace::reattach::reattach)
    /// puts one back once the package is declared again.
    pub locked: Option<LockEntry>,
}

impl Uninstalled {
    /// The local commits the lock named that were not pushed, which the
    /// Domain still holds.
    #[must_use]
    pub fn unpushed(&self) -> &[String] {
        self.locked
            .as_ref()
            .map_or(&[], |locked| locked.commits.as_slice())
    }
}

/// Why nothing was uninstalled.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum UninstallError {
    #[error(transparent)]
    Contract(Box<ContractError>),

    #[error(transparent)]
    Lock(Box<LockError>),

    #[error(
        "{LOCK_FILE} was written for another {}; remove the lock, or fix it, to uninstall",
        .0.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ")
    )]
    Mismatch(Vec<Mismatch>),

    #[error("neither {CONTRACT_FILE} nor {LOCK_FILE} names {0}")]
    NotInstalled(Box<PackageKey>),

    #[error(transparent)]
    Failed(Box<crate::Error>),
}

impl From<ContractError> for UninstallError {
    fn from(err: ContractError) -> Self {
        Self::Contract(Box::new(err))
    }
}

impl From<LockError> for UninstallError {
    fn from(err: LockError) -> Self {
        Self::Lock(Box::new(err))
    }
}

impl From<crate::Error> for UninstallError {
    fn from(err: crate::Error) -> Self {
        Self::Failed(Box::new(err))
    }
}

/// Uninstalls `package` from the dataspace in the folder `dataspace`: every
/// declaration of it is removed from `data.toml`, which the user asked for
/// (25.3), then the working folder is removed where a read filled it, then
/// its lock entry is dropped and the lock written, last.
///
/// The working folder of a `read` or `sync` entry inside the dataspace is
/// removed with what it holds, as an uninstall removes the package's folder
/// today. One outside the dataspace, or of a `write` or `replace` entry, is
/// left: its files are the user's.
///
/// Nothing is removed from the Domain. Commits the lock named that were not
/// pushed stay in the store, recoverable by
/// [`reattach`](crate::dataspace::reattach::reattach) once the package is
/// declared again, and [`Uninstalled::unpushed`] names them; the local
/// registry is never touched. Clearing what the store holds of the package
/// is [`prune`](crate::dataspace::prune::prune)'s.
///
/// A package of the local registry `local` is matched under its name or one
/// it had. The lock is used as
/// [`read_all`](crate::dataspace::read_all::read_all) uses it.
pub async fn uninstall(
    storage: &(impl Storage + Sync),
    local: Option<&LocalRegistry>,
    dataspace: &Path,
    writer: &Writer,
    package: &PackageKey,
) -> Result<Uninstalled, UninstallError> {
    // Read whole first: a missing `data.toml` is refused, not made.
    Contract::read(storage, dataspace, writer.local_registry_name()).await?;
    let mut edit = ContractEdit::open(storage, dataspace).await?;
    let mut contract = edit.original(writer.local_registry_name())?;
    let (mut lock, mut writer) = usable_lock(storage, &contract, writer)
        .await?
        .map_err(UninstallError::Mismatch)?;
    let matched = lock.header.data_toml_hash == contract.data_toml_hash();
    name_local(storage, local, &mut contract, &mut lock, &mut writer).await?;
    let key = current(storage, local, package).await?;

    let places: Vec<(Access, usize)> = contract
        .declared()
        .iter()
        .filter(|(_, _, declared)| *declared == key)
        .map(|(access, index, _)| (*access, *index))
        .collect();
    let entries: Vec<Entry> = contract
        .entries()
        .iter()
        .filter(|entry| entry.key() == key)
        .cloned()
        .collect();
    let locked = lock.packages.remove(&key);
    if places.is_empty() && locked.is_none() {
        return Err(UninstallError::NotInstalled(Box::new(key)));
    }

    edit.remove(&places);
    let mut edited = edit.edited(writer.local_registry_name())?;
    name_local(storage, local, &mut edited, &mut lock, &mut writer).await?;
    if edit.changed() {
        edit.write(storage).await?;
    }

    let (mut removed, mut kept) = (None, None);
    if let Some(folder) = entries.first().map(|entry| entry.folder.clone()) {
        let read = entries
            .iter()
            .all(|entry| matches!(entry.access, Access::Read | Access::Sync));
        if read && folder.starts_with(contract.dataspace()) {
            match storage.remove_dir_all(&folder).await {
                Ok(()) => {}
                Err(err) if err.is_not_found() => {}
                Err(err) => return Err(err.into()),
            }
            removed = Some(folder);
        } else {
            kept = Some(folder);
        }
    }

    write_lock(storage, &mut lock, &edited, &writer, matched).await?;
    info!(
        "✔️ Uninstalled {key} from {}",
        contract.dataspace().display()
    );
    Ok(Uninstalled {
        entries,
        removed,
        kept,
        locked,
    })
}

/// `package` under the local registry's current name, where `local`
/// answers to the name it gives.
async fn current(
    storage: &(impl Storage + Sync),
    local: Option<&LocalRegistry>,
    package: &PackageKey,
) -> Result<PackageKey, UninstallError> {
    if let (Registry::Local(name), Some(local)) = (&package.registry, local) {
        let names = local.names(storage).await?;
        if names.answers_to(name) {
            return Ok(PackageKey {
                registry: Registry::Local(names.name),
                package: package.package.clone(),
            });
        }
    }
    Ok(package.clone())
}

#[cfg(test)]
mod tests;
