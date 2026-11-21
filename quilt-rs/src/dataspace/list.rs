//! `list`: the packages a dataspace declares, each with a summary of its
//! [`status`]; the packages the Domain's stores hold, which is what
//! quilt3's `list_packages` lists with no registry (25.14); and a
//! registry's packages, from its latest index.

use std::collections::BTreeMap;
use std::path::Path;

use crate::Res;
use crate::dataspace::EntryError;
use crate::dataspace::contract::Entry;
use crate::dataspace::contract::PackageKey;
use crate::dataspace::contract::Registry;
use crate::dataspace::lock::Writer;
use crate::dataspace::status::LatestSource;
use crate::dataspace::status::StatusError;
use crate::dataspace::status::status;
use crate::domain::DomainRoot;
use crate::io::remote::Remote;
use crate::io::storage::Storage;
use crate::lineage::PackageState;
use crate::lineage::UpstreamState;
use crate::queue::KeptReason;
use crate::queue::Queue;
use quilt_uri::Host;
use quilt_uri::Namespace;

/// One package a dataspace declares, under one access.
#[derive(Debug)]
#[non_exhaustive]
pub struct DeclaredPackage {
    pub entry: Entry,
    /// What a front end shows: [`EntryStatus::state`](crate::dataspace::status::EntryStatus::state).
    pub state: PackageState,
    pub upstream_state: UpstreamState,
    pub latest_source: LatestSource,
    /// How many local commits the lock names that are not pushed.
    pub unpushed: usize,
    /// How many of its keys `conflict.json` holds.
    pub conflicts: usize,
    /// Why `queue.db` kept a revision of it, where it did.
    pub kept: Option<KeptReason>,
    /// How many things could not be read for it.
    pub problems: usize,
}

/// The packages the dataspace in the folder `dataspace` declares, in the
/// contract's order, each summarised from its [`status`], which takes the
/// same arguments and, like it, writes nothing.
#[allow(clippy::too_many_arguments)]
pub async fn list(
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    domain: Option<&DomainRoot>,
    queue: Option<&Queue>,
    dataspace: &Path,
    writer: &Writer,
    user_rules: &Path,
    fetch: bool,
) -> Result<Vec<DeclaredPackage>, StatusError> {
    let found = Box::pin(status(
        storage, remote, domain, queue, dataspace, writer, user_rules, fetch,
    ))
    .await?;
    Ok(found
        .entries
        .into_iter()
        .map(|status| DeclaredPackage {
            state: status.state(),
            upstream_state: status.upstream_state,
            latest_source: status.latest_source,
            unpushed: status.unpushed.len(),
            conflicts: status.conflicts.len(),
            kept: status.kept.as_ref().map(|kept| kept.reason),
            problems: status.problems.len(),
            entry: status.entry,
        })
        .collect())
}

/// A package one of the Domain's stores holds.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct StoredPackage {
    pub key: PackageKey,
    /// What `latest` names: the local registry's own tag, or what the
    /// bucket's latest index last knew; `None` where nothing is known.
    pub latest: Option<String>,
}

/// Every package `domain` holds, in every store: those installed or
/// committed in a bucket's Package Store, and those the local registry
/// holds, under its name. A local registry never named holds nothing, as
/// every write to it names it first. Nothing is written.
pub async fn stored(
    storage: &(impl Storage + Sync),
    domain: &DomainRoot,
) -> Res<Vec<StoredPackage>> {
    let mut packages = Vec::new();
    for store in domain.stores(storage).await? {
        for namespace in store.installed(storage).await? {
            let latest = store.read_latest(storage, &namespace).await?;
            packages.push(StoredPackage {
                key: PackageKey {
                    registry: Registry::S3(store.bucket().to_string()),
                    package: namespace,
                },
                latest,
            });
        }
    }
    let local = domain.local();
    if let Some(names) = local.read_names(storage).await? {
        for namespace in local.packages(storage).await? {
            let latest = local.read_latest(storage, &namespace).await?;
            packages.push(StoredPackage {
                key: PackageKey {
                    registry: Registry::Local(names.name.clone()),
                    package: namespace,
                },
                latest,
            });
        }
    }
    Ok(packages)
}

/// The packages `registry` holds, with the hash each one's `latest` names.
///
/// A bucket's come from its latest index in `domain`, refreshed first from
/// the bucket's registry, asked of `catalog` where there is one, where
/// `fetch`; that refresh is the only write. The local registry's come
/// from its own tags; `registry` must name it, by any name it has had.
pub async fn registry_packages(
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    domain: &DomainRoot,
    registry: &Registry,
    catalog: Option<&Host>,
    fetch: bool,
) -> Result<BTreeMap<Namespace, String>, EntryError> {
    match registry {
        Registry::S3(bucket) => {
            let store = domain.registry(bucket)?;
            if fetch {
                Ok(store.refresh_latest_index(storage, remote, catalog).await?)
            } else {
                Ok(store.latest_index(storage).await?)
            }
        }
        Registry::Local(name) => {
            let local = domain.local();
            let names = local.read_names(storage).await?;
            if !names.is_some_and(|names| names.answers_to(name)) {
                return Err(EntryError::LocalRegistry);
            }
            let mut packages = BTreeMap::new();
            for namespace in local.packages(storage).await? {
                if let Some(latest) = local.read_latest(storage, &namespace).await? {
                    packages.insert(namespace, latest);
                }
            }
            Ok(packages)
        }
    }
}

#[cfg(test)]
mod tests;
