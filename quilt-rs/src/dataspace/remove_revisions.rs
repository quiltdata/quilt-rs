//! `remove_revisions`: revisions of a package taken out of its registry,
//! their tags and manifests, never the data, as quilt3's `delete_package`
//! with a registry takes them out (25.14).

use std::collections::BTreeSet;
use std::path::Path;

use thiserror::Error;
use tracing::info;

use crate::dataspace::EntryError;
use crate::dataspace::contract::CONTRACT_FILE;
use crate::dataspace::contract::Contract;
use crate::dataspace::contract::ContractError;
use crate::dataspace::contract::Entry;
use crate::dataspace::contract::PackageKey;
use crate::dataspace::contract::Registry;
use crate::dataspace::lock::LOCK_FILE;
use crate::dataspace::lock::LockError;
use crate::dataspace::lock::Mismatch;
use crate::dataspace::lock::Writer;
use crate::dataspace::read_all::usable_lock;
use crate::dataspace::store::current_key;
use crate::dataspace::store::local_registry;
use crate::dataspace::store::name_local;
use crate::domain::DomainRoot;
use crate::domain::LocalRegistry;
use crate::io::remote::Remote;
use crate::io::storage::Storage;
use quilt_uri::S3Uri;
use quilt_uri::Tag;
use quilt_uri::paths::tag_key;

/// What became of `latest` when the revisions it might name were removed.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum LatestAfter {
    /// `latest` named none of them, and was left.
    Kept,
    /// `latest` named one of them, and now names the newest revision left.
    Moved(String),
    /// `latest` named one of them, and no revision is left for it to name.
    Removed,
}

/// What [`remove_revisions`] did.
#[derive(Debug)]
#[non_exhaustive]
pub struct Removed {
    /// The revisions removed, by whole hash, in the order asked for.
    pub revisions: Vec<String>,
    pub latest: LatestAfter,
    /// The removed revisions that the dataspace's lock names for the
    /// package, as its base or a commit. The lock is left as it is (25.12):
    /// a bucket still holds the manifest by its hash, and a local registry
    /// does not, so a read of it there fails until the package is reset.
    pub locked: Vec<String>,
}

/// Why nothing was removed.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum RemoveError {
    #[error(transparent)]
    Contract(Box<ContractError>),

    #[error(transparent)]
    Lock(Box<LockError>),

    #[error(
        "{LOCK_FILE} was written for another {}; remove the lock, or fix it, to remove revisions",
        .0.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ")
    )]
    Mismatch(Vec<Mismatch>),

    #[error("{CONTRACT_FILE} does not declare {0}")]
    NotDeclared(Box<PackageKey>),

    #[error("no revision to remove was named")]
    NoneNamed,

    #[error("the registry holds no revision of the package that {0:?} names")]
    NotFound(String),

    #[error("{0:?} names more than one revision of the package; give more of the hash")]
    Ambiguous(String),

    #[error("another process is changing {0} in the Domain; try again")]
    Busy(Box<PackageKey>),

    #[error(transparent)]
    Entry(#[from] EntryError),
}

impl From<ContractError> for RemoveError {
    fn from(err: ContractError) -> Self {
        Self::Contract(Box::new(err))
    }
}

impl From<LockError> for RemoveError {
    fn from(err: LockError) -> Self {
        Self::Lock(Box::new(err))
    }
}

impl From<crate::Error> for RemoveError {
    fn from(err: crate::Error) -> Self {
        Self::Entry(err.into())
    }
}

/// Removes `revisions`, each a whole top hash or a prefix of one hash only,
/// of `package` from its registry, as the dataspace in the folder
/// `dataspace` declares it: in a bucket, every tag that names one, and in
/// the local registry of `domain`, its manifest. No object is deleted: the
/// data stays (25.14). No revision is protected, as none is in quilt3.
///
/// Where `latest` names a removed revision, it is moved to the newest
/// revision left, by when it was published, or removed with none left, as
/// quilt3 does. A bucket keeps the manifests under `.quilt/packages/` by
/// hash alone, shared by every package that holds one, so they are left, as
/// quilt3 leaves them; the domain's latest index follows the move. The local
/// registry keeps its revisions by hash too, so the registry's copy is
/// removed only where no other of its packages holds that revision, and
/// the package's own copy always, under the package's lock.
///
/// `data.toml` and the lock are not changed: a lock entry naming a removed
/// revision is left (25.12), and [`Removed::locked`] names it. The catalog
/// the contract gives the package signs the bucket's requests. The lock is
/// used as [`read_all`](crate::dataspace::read_all::read_all) uses it.
pub async fn remove_revisions(
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    domain: Option<&DomainRoot>,
    dataspace: &Path,
    writer: &Writer,
    package: &PackageKey,
    revisions: &[String],
) -> Result<Removed, RemoveError> {
    if revisions.is_empty() {
        return Err(RemoveError::NoneNamed);
    }
    let mut contract = Contract::read(storage, dataspace, writer.local_registry_name()).await?;
    let (mut lock, mut writer) = usable_lock(storage, &contract, writer)
        .await?
        .map_err(RemoveError::Mismatch)?;
    let local = domain.map(DomainRoot::local);
    name_local(
        storage,
        local.as_ref(),
        &mut contract,
        &mut lock,
        &mut writer,
    )
    .await?;
    let key = current_key(storage, local.as_ref(), package).await?;
    let entry = contract
        .entries()
        .iter()
        .find(|entry| entry.key() == key)
        .cloned()
        .ok_or_else(|| RemoveError::NotDeclared(Box::new(key.clone())))?;

    let (removed, latest) = match local_registry(storage, local.as_ref(), &entry).await? {
        Some(registry) => {
            let Some(_held) = storage
                .try_lock_exclusive(registry.package_lock(&entry.package))
                .await?
            else {
                return Err(RemoveError::Busy(Box::new(key)));
            };
            from_local(storage, registry, &entry, revisions).await?
        }
        None => from_bucket(storage, remote, domain, &entry, revisions).await?,
    };

    let locked = lock.packages.get(&key).map_or_else(Vec::new, |locked| {
        locked
            .base
            .iter()
            .chain(&locked.commits)
            .filter(|hash| removed.contains(hash))
            .cloned()
            .collect()
    });
    info!("✔️ Removed {} revisions of {key}", removed.len());
    Ok(Removed {
        revisions: removed,
        latest,
        locked,
    })
}

/// Each of `wanted` as one of `held`, a whole hash or the only one it
/// starts, in `wanted`'s order and once each.
fn resolve(held: &BTreeSet<String>, wanted: &[String]) -> Result<Vec<String>, RemoveError> {
    let mut resolved = Vec::new();
    for prefix in wanted {
        let mut matching = held
            .iter()
            .filter(|hash| !prefix.is_empty() && hash.starts_with(prefix.as_str()));
        let hash = match (matching.next(), matching.next()) {
            (Some(hash), None) => hash,
            (Some(_), Some(_)) => return Err(RemoveError::Ambiguous(prefix.clone())),
            (None, _) => return Err(RemoveError::NotFound(prefix.clone())),
        };
        if !resolved.contains(hash) {
            resolved.push(hash.clone());
        }
    }
    Ok(resolved)
}

/// [`remove_revisions`] in `entry`'s bucket.
async fn from_bucket(
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    domain: Option<&DomainRoot>,
    entry: &Entry,
    revisions: &[String],
) -> Result<(Vec<String>, LatestAfter), RemoveError> {
    let Registry::S3(bucket) = &entry.registry else {
        return Err(EntryError::LocalRegistry.into());
    };
    let host = entry.catalog.as_ref();
    let tags = remote.package_tags(host, bucket, &entry.package).await?;
    let held = tags.iter().map(|(_, hash)| hash.clone()).collect();
    let removed = resolve(&held, revisions)?;
    let latest_tag = Tag::Latest.to_string();
    let tag_uri = |name: &str| S3Uri {
        bucket: bucket.clone(),
        key: tag_key(&entry.package, name),
        version: None,
    };

    let named = |name: &str| {
        tags.iter()
            .find(|(tag, _)| tag == name)
            .map(|(_, hash)| hash.clone())
    };
    let latest = match named(&latest_tag) {
        Some(hash) if removed.contains(&hash) => {
            // The newest timestamp left: quilt3 compares their names.
            let newest = tags
                .iter()
                .filter_map(|(name, hash)| match name.parse() {
                    Ok(Tag::Timestamp(at)) if !removed.contains(hash) => Some((at.0, hash)),
                    _ => None,
                })
                .max()
                .map(|(_, hash)| hash.clone());
            if let Some(hash) = newest {
                remote
                    .put_object(host, &tag_uri(&latest_tag), hash.as_bytes().to_vec())
                    .await?;
                LatestAfter::Moved(hash)
            } else {
                remote.delete_object(host, &tag_uri(&latest_tag)).await?;
                LatestAfter::Removed
            }
        }
        _ => LatestAfter::Kept,
    };
    for (name, hash) in &tags {
        if *name != latest_tag && removed.contains(hash) {
            remote.delete_object(host, &tag_uri(name)).await?;
        }
    }

    // The latest index this machine keeps follows the move.
    if let (Some(domain), LatestAfter::Moved(hash)) = (domain, &latest) {
        domain
            .registry(bucket)?
            .write_latest(storage, &entry.package, hash)
            .await?;
    }
    Ok((removed, latest))
}

/// [`remove_revisions`] in the local registry `registry`, by a caller that
/// holds the package's lock.
async fn from_local(
    storage: &(impl Storage + Sync),
    registry: &LocalRegistry,
    entry: &Entry,
    revisions: &[String],
) -> Result<(Vec<String>, LatestAfter), RemoveError> {
    let namespace = &entry.package;
    let listed = registry.revisions(storage, namespace).await?;
    let held = listed.iter().map(|(hash, _)| hash.clone()).collect();
    let removed = resolve(&held, revisions)?;
    let latest = match registry.read_latest(storage, namespace).await? {
        Some(hash) if removed.contains(&hash) => {
            let newest = listed
                .iter()
                .filter(|(hash, _)| !removed.contains(hash))
                .max_by_key(|(_, published)| *published)
                .map(|(hash, _)| hash.clone());
            registry
                .set_latest_held(storage, namespace, newest.as_deref())
                .await?;
            newest.map_or(LatestAfter::Removed, LatestAfter::Moved)
        }
        _ => LatestAfter::Kept,
    };
    for hash in &removed {
        registry
            .remove_revision_held(storage, namespace, hash)
            .await?;
    }
    Ok((removed, latest))
}

#[cfg(test)]
mod tests;
