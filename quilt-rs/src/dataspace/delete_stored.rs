//! A package deleted from every local Package Store of a Domain: quilt3's
//! `delete_package` with no registry (quilt-specs#297's delete ruling).
//!
//! Each bucket's store loses the package's installed manifests, unpushed
//! commits included, and the local registry its revisions and `latest`.
//! Nothing a dataspace holds is touched: `data.toml`, the lock and the
//! working files stay. No object is deleted; `gc` clears what no manifest
//! names any more. The buckets are not asked or changed.

use std::collections::BTreeSet;
use std::path::PathBuf;

use quilt_uri::Namespace;
use quilt_uri::is_top_hash;
use thiserror::Error;
use tracing::info;

use crate::dataspace::contract::PackageKey;
use crate::dataspace::contract::Registry;
use crate::dataspace::remove_revisions::LatestAfter;
use crate::domain::DomainRoot;
use crate::domain::LocalRegistry;
use crate::domain::RegistryStore;
use crate::error::HashPrefixError;
use crate::flow;
use crate::flow::list_files;
use crate::io::storage::Storage;

/// What [`delete_stored`] removed from one store.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct StoreDeleted {
    /// The package, keyed by the store's registry.
    pub key: PackageKey,
    /// The revisions removed, by whole hash, sorted.
    pub revisions: Vec<String>,
    /// What became of the local registry's `latest`. A bucket's store
    /// keeps only a cache of the bucket's, so for it this is always
    /// [`LatestAfter::Kept`].
    pub latest: LatestAfter,
}

/// Why nothing was deleted.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum DeleteError {
    #[error("no local Package Store holds {0}")]
    NotFound(Box<Namespace>),

    #[error(transparent)]
    HashPrefix(#[from] HashPrefixError),

    #[error("another process is changing {0} in the Domain; try again")]
    Busy(Box<PackageKey>),

    #[error(transparent)]
    Store(Box<crate::Error>),
}

impl From<crate::Error> for DeleteError {
    fn from(err: crate::Error) -> Self {
        match err {
            crate::Error::HashPrefix(err) => Self::HashPrefix(err),
            err => Self::Store(Box::new(err)),
        }
    }
}

impl From<std::io::Error> for DeleteError {
    fn from(err: std::io::Error) -> Self {
        crate::Error::from(err).into()
    }
}

/// One store holding the package.
enum Target {
    Bucket(RegistryStore),
    Local(LocalRegistry, String),
}

impl Target {
    fn key(&self, namespace: &Namespace) -> PackageKey {
        let registry = match self {
            Self::Bucket(store) => Registry::S3(store.bucket().to_string()),
            Self::Local(_, name) => Registry::Local(name.clone()),
        };
        PackageKey {
            registry,
            package: namespace.clone(),
        }
    }

    fn installed(&self, namespace: &Namespace) -> PathBuf {
        match self {
            Self::Bucket(store) => store.installed_manifests_dir(namespace),
            Self::Local(local, _) => local.installed_manifests_dir(namespace),
        }
    }

    fn package_lock(&self, namespace: &Namespace) -> PathBuf {
        match self {
            Self::Bucket(store) => store.package_lock(namespace),
            Self::Local(local, _) => local.package_lock(namespace),
        }
    }
}

/// Deletes `namespace` from every local Package Store of `domain`: each
/// bucket's store, and the local registry. With no `top_hash`, the whole
/// package goes, unpushed commits included. With one, a whole top hash or a
/// prefix that only one of the package's revisions here starts, only that
/// revision goes, from every store holding it, with no revision protected.
/// Where the local registry's `latest` named it, `latest` moves to the
/// newest revision left, by when it was published, or is removed with none
/// left; a package with no revision left is gone.
///
/// A prefix is matched as [`flow::resolve_local_hash`] matches it, among the
/// package's own installed manifests only (entry 27d). Every store's lock of
/// the package is held before anything is removed.
///
/// # Errors
///
/// A package no store holds, a prefix that is malformed, matches none of
/// its revisions or more than one, a package another process holds, and a
/// store that cannot be read or written. Nothing is removed on any of
/// these but the last.
pub async fn delete_stored(
    storage: &(impl Storage + Sync),
    domain: &DomainRoot,
    namespace: &Namespace,
    top_hash: Option<&str>,
) -> Result<Vec<StoreDeleted>, DeleteError> {
    let mut targets = Vec::new();
    for store in domain.stores(storage).await? {
        if store.installed(storage).await?.contains(namespace) {
            targets.push(Target::Bucket(store));
        }
    }
    let local = domain.local();
    if local.packages(storage).await?.contains(namespace) {
        let name = local
            .read_names(storage)
            .await?
            .map_or_default(|names| names.name);
        targets.push(Target::Local(local, name));
    }
    if targets.is_empty() {
        return Err(DeleteError::NotFound(Box::new(namespace.clone())));
    }

    let mut held = Vec::new();
    for target in &targets {
        let Some(guard) = storage
            .try_lock_exclusive(target.package_lock(namespace))
            .await?
        else {
            return Err(DeleteError::Busy(Box::new(target.key(namespace))));
        };
        held.push(guard);
    }

    let wanted = match top_hash {
        Some(prefix) => Some(resolve(storage, &targets, namespace, prefix).await?),
        None => None,
    };
    let mut deleted = Vec::new();
    for target in &targets {
        let revisions = revisions_in(storage, &target.installed(namespace)).await?;
        let removing: Vec<String> = match &wanted {
            Some(hash) if revisions.contains(hash) => vec![hash.clone()],
            Some(_) => continue,
            None => revisions.iter().cloned().collect(),
        };
        let latest = match target {
            Target::Bucket(_) => {
                for hash in &removing {
                    remove_if_there(storage, &target.installed(namespace).join(hash)).await?;
                }
                LatestAfter::Kept
            }
            Target::Local(local, _) => {
                from_local(storage, local, namespace, &removing, wanted.is_none()).await?
            }
        };
        // A package with no revision left here is not listed as held.
        if revisions_in(storage, &target.installed(namespace))
            .await?
            .is_empty()
        {
            match storage.remove_dir_all(target.installed(namespace)).await {
                Err(err) if !err.is_not_found() => return Err(err.into()),
                _ => {}
            }
        }
        deleted.push(StoreDeleted {
            key: target.key(namespace),
            revisions: removing,
            latest,
        });
    }
    drop(held);
    info!(
        "✔️ Deleted {namespace} from {} local Package Stores",
        deleted.len()
    );
    Ok(deleted)
}

/// The one revision `prefix` names among every target's: a prefix two
/// stores resolve to one hash is no ambiguity.
async fn resolve(
    storage: &(impl Storage + Sync),
    targets: &[Target],
    namespace: &Namespace,
    prefix: &str,
) -> Result<String, DeleteError> {
    let mut found = BTreeSet::new();
    for target in targets {
        let installed = target.installed(namespace);
        match flow::resolve_local_hash(storage, &installed, prefix).await {
            // A whole hash resolves as itself, held or not.
            Ok(hash) if storage.is_file(installed.join(&hash)).await => {
                found.insert(hash);
            }
            Ok(_) | Err(crate::Error::HashPrefix(HashPrefixError::NoMatch(_))) => {}
            Err(err) => return Err(err.into()),
        }
    }
    let mut found = found.into_iter();
    match (found.next(), found.next()) {
        (Some(hash), None) => Ok(hash),
        (None, _) => Err(HashPrefixError::NoMatch(prefix.to_string()).into()),
        (Some(first), Some(second)) => {
            let mut matches = vec![first, second];
            matches.extend(found);
            let more = matches.len().saturating_sub(flow::MAX_LISTED_MATCHES);
            matches.truncate(flow::MAX_LISTED_MATCHES);
            Err(HashPrefixError::Ambiguous {
                prefix: prefix.to_string(),
                matches,
                more,
            }
            .into())
        }
    }
}

/// The top hashes of the manifests in `dir`.
async fn revisions_in(
    storage: &(impl Storage + Sync),
    dir: &std::path::Path,
) -> Result<BTreeSet<String>, DeleteError> {
    Ok(list_files(storage, dir)
        .await?
        .into_iter()
        .map(|(name, _, _)| name)
        .filter(|name| is_top_hash(name))
        .collect())
}

async fn remove_if_there(
    storage: &(impl Storage + Sync),
    path: &std::path::Path,
) -> Result<(), DeleteError> {
    match storage.remove_file(path).await {
        Err(err) if err.kind() != std::io::ErrorKind::NotFound => Err(err.into()),
        _ => Ok(()),
    }
}

/// `removing` taken out of the local registry `local`, by a caller that
/// holds the package's lock: with `whole`, `latest` is removed, else moved
/// as
/// [`remove_revisions`](crate::dataspace::remove_revisions::remove_revisions)
/// moves it.
async fn from_local(
    storage: &(impl Storage + Sync),
    local: &LocalRegistry,
    namespace: &Namespace,
    removing: &[String],
    whole: bool,
) -> Result<LatestAfter, DeleteError> {
    let published = local.revisions(storage, namespace).await?;
    let latest = match local.read_latest(storage, namespace).await? {
        Some(_) if whole => {
            local.set_latest_held(storage, namespace, None).await?;
            LatestAfter::Removed
        }
        Some(hash) if removing.contains(&hash) => {
            let newest = published
                .iter()
                .filter(|(hash, _)| !removing.contains(hash))
                .max_by_key(|(_, at)| *at)
                .map(|(hash, _)| hash.clone());
            local
                .set_latest_held(storage, namespace, newest.as_deref())
                .await?;
            newest.map_or(LatestAfter::Removed, LatestAfter::Moved)
        }
        _ => LatestAfter::Kept,
    };
    for hash in removing {
        local.remove_revision_held(storage, namespace, hash).await?;
    }
    Ok(latest)
}

#[cfg(test)]
mod tests;
