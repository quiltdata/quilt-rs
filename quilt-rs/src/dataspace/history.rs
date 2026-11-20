//! `history`: a package's revisions, newest first, from its registry: a
//! bucket's listing, or the local registry's revisions, with the local
//! commits its store holds that the registry does not, marked as local.
//! Nothing is written.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::Path;
use std::path::PathBuf;

use jiff::Timestamp;
use thiserror::Error;

use crate::dataspace::EntryError;
use crate::dataspace::contract::Contract;
use crate::dataspace::contract::ContractError;
use crate::dataspace::contract::PackageKey;
use crate::dataspace::contract::Registry;
use crate::dataspace::lock::Lock;
use crate::dataspace::lock::LockEntry;
use crate::dataspace::lock::LockError;
use crate::dataspace::lock::Writer;
use crate::domain::DomainRoot;
use crate::io::remote::Remote;
use crate::io::storage::Storage;
use crate::manifest::Manifest;
use quilt_uri::Host;
use quilt_uri::is_top_hash;

/// One revision of a package.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Revision {
    /// Top hash of the revision's manifest.
    pub hash: String,
    /// Its commit message, where its manifest or the registry's listing is
    /// at hand.
    pub message: Option<String>,
    /// When it was published, as the registry's pointer to it says; for a
    /// local commit, or a revision of the local registry, when this machine
    /// stored it.
    pub time: Option<Timestamp>,
    /// The revision it was made on, where known: a manifest does not record
    /// it, so only the lock's commits, each made on the one before it and the
    /// first on the base, have one.
    pub parent: Option<String>,
    /// A local commit the registry does not have yet.
    pub local: bool,
}

/// Why a history could not be read.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum HistoryError {
    #[error(transparent)]
    Contract(Box<ContractError>),

    #[error(transparent)]
    Lock(Box<LockError>),

    #[error(transparent)]
    Entry(#[from] EntryError),
}

impl From<ContractError> for HistoryError {
    fn from(err: ContractError) -> Self {
        Self::Contract(Box::new(err))
    }
}

impl From<LockError> for HistoryError {
    fn from(err: LockError) -> Self {
        Self::Lock(Box::new(err))
    }
}

/// The history of `package` as the dataspace in the folder `dataspace` has
/// it: [`revisions`], under the catalog the contract gives the package and
/// with the commits its lock names. A package the contract does not
/// declare is asked of its registry with no catalog. The lock is read
/// whatever its header says, and nothing is written.
pub async fn history(
    storage: &(impl Storage + Sync),
    remote: &(impl Remote + Sync),
    domain: Option<&DomainRoot>,
    dataspace: &Path,
    writer: &Writer,
    package: &PackageKey,
) -> Result<Vec<Revision>, HistoryError> {
    let contract = Contract::read(storage, dataspace, writer.local_registry_name()).await?;
    let lock = Lock::read(storage, &contract).await?;
    let names = match domain {
        Some(domain) => domain.local().read_names(storage).await.ok().flatten(),
        None => None,
    };
    // The package and the lock's keys by the local registry's current name.
    let current = |key: &PackageKey| match (&key.registry, &names) {
        (Registry::Local(name), Some(names)) if names.answers_to(name) => PackageKey {
            registry: Registry::Local(names.name.clone()),
            package: key.package.clone(),
        },
        _ => key.clone(),
    };
    let wanted = current(package);
    let catalog = contract
        .entries()
        .iter()
        .find(|entry| current(&entry.key()) == wanted)
        .and_then(|entry| entry.catalog.clone());
    let locked = lock.as_ref().and_then(|lock| {
        lock.packages
            .iter()
            .find(|(key, _)| current(key) == wanted)
            .map(|(_, locked)| locked)
    });
    Ok(revisions(storage, remote, domain, package, catalog.as_ref(), locked).await?)
}

/// `package`'s revisions, newest first, those with no time last.
///
/// A bucket's are those its registry lists, asked of `catalog` where there
/// is one, with each time its listing gives; the local commits `domain`'s
/// store for the bucket holds that it does not list follow them in.
/// The local registry's are those it published, with those committed to it
/// and not published marked local; it is `domain`'s, so `package` must name
/// it, by any name. A message is read from the manifest where the Domain
/// holds it; a bucket's revision the Domain does not hold is not fetched.
///
/// `locked`, the lock's entry for the package, gives each of its commits a
/// parent, and its commits the store no longer holds are listed too, with
/// what the lock knows of them.
pub async fn revisions(
    storage: &(impl Storage + Sync),
    remote: &(impl Remote + Sync),
    domain: Option<&DomainRoot>,
    package: &PackageKey,
    catalog: Option<&Host>,
    locked: Option<&LockEntry>,
) -> Result<Vec<Revision>, EntryError> {
    let namespace = &package.package;
    let mut listed = Vec::new();
    // Each manifest the Domain holds for the package, by hash, where it is.
    let mut held: BTreeMap<String, PathBuf> = BTreeMap::new();
    let installed = match (&package.registry, domain) {
        (Registry::S3(bucket), domain) => {
            let published = remote
                .published_revision_history(catalog, bucket, namespace)
                .await?;
            if let Some(domain) = domain {
                let store = domain.registry(bucket)?;
                for revision in &published {
                    let cached = store.cached_manifest(&revision.hash);
                    if storage.exists(&cached).await {
                        held.insert(revision.hash.clone(), cached);
                    }
                }
            }
            for revision in published {
                listed.push(Revision {
                    hash: revision.hash,
                    message: revision.message,
                    time: revision.modified,
                    parent: None,
                    local: false,
                });
            }
            match domain {
                Some(domain) => Some(domain.registry(bucket)?.installed_manifests_dir(namespace)),
                None => None,
            }
        }
        (Registry::Local(_), None) => return Err(EntryError::LocalRegistry),
        (Registry::Local(name), Some(domain)) => {
            let local = domain.local();
            let names = local.read_names(storage).await.ok().flatten();
            if !names.is_some_and(|names| names.answers_to(name)) {
                return Err(EntryError::LocalRegistry);
            }
            let dir = local.installed_manifests_dir(namespace);
            for (hash, path) in manifests_in(storage, &dir).await? {
                let revision = local.revision(&hash);
                let published = storage.exists(&revision).await;
                let stored = if published { &revision } else { &path };
                listed.push(Revision {
                    hash: hash.clone(),
                    message: None,
                    time: modified(storage, stored).await,
                    parent: None,
                    local: !published,
                });
                held.insert(hash, path);
            }
            None
        }
    };

    let known: BTreeSet<String> = listed
        .iter()
        .map(|revision| revision.hash.clone())
        .collect();
    if let Some(dir) = installed {
        for (hash, path) in manifests_in(storage, &dir).await? {
            if !known.contains(&hash) {
                listed.push(Revision {
                    hash: hash.clone(),
                    message: None,
                    time: modified(storage, &path).await,
                    parent: None,
                    local: true,
                });
            }
            held.insert(hash, path);
        }
    }

    if let Some(locked) = locked {
        follow_lock(&mut listed, locked);
    }
    for revision in &mut listed {
        if revision.message.is_some() {
            continue;
        }
        if let Some(path) = held.get(&revision.hash)
            && let Ok(manifest) = Manifest::from_path(storage, path).await
        {
            revision.message = manifest.header.message().map(str::to_string);
        }
    }
    // A commit only the lock names has no time and is newer than any listed.
    listed.sort_by_key(|revision| {
        std::cmp::Reverse(
            revision
                .time
                .or_else(|| revision.local.then_some(Timestamp::MAX)),
        )
    });
    Ok(listed)
}

/// Gives each commit `locked` names its parent: the commit before it, or
/// the base for the first, which has none before a first publication. A
/// commit not `listed` is added, local.
fn follow_lock(listed: &mut Vec<Revision>, locked: &LockEntry) {
    let parents = std::iter::once(locked.base.as_ref()).chain(locked.commits.iter().map(Some));
    let mut added = Vec::new();
    for (commit, parent) in locked.commits.iter().zip(parents) {
        let parent = parent.cloned();
        match listed.iter_mut().find(|revision| revision.hash == *commit) {
            Some(revision) => revision.parent = parent,
            None => added.push(Revision {
                hash: commit.clone(),
                message: None,
                time: None,
                parent,
                local: true,
            }),
        }
    }
    // Newest first, as the sort keeps them.
    listed.extend(added.into_iter().rev());
}

/// The manifests in `dir`, by hash; none where it does not exist.
async fn manifests_in(
    storage: &(impl Storage + Sync),
    dir: &Path,
) -> Result<Vec<(String, PathBuf)>, EntryError> {
    let mut entries = match storage.read_dir(dir).await {
        Ok(entries) => entries,
        Err(err) if err.is_not_found() => return Ok(Vec::new()),
        Err(err) => return Err(err.into()),
    };
    let mut manifests = Vec::new();
    while let Some(entry) = entries.next_entry().await.map_err(crate::Error::from)? {
        let name = entry.file_name().to_string_lossy().into_owned();
        if is_top_hash(&name) {
            manifests.push((name, entry.path()));
        }
    }
    Ok(manifests)
}

/// When the file at `path` was last written, where that can be read.
async fn modified(storage: &(impl Storage + Sync), path: &Path) -> Option<Timestamp> {
    storage.modified_timestamp(path).await.ok()
}

#[cfg(test)]
mod tests;
