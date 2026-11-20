//! `gc`: what is safe to clear in the Domain, cleared (25.12): in each
//! bucket's Package Store, the content its bucket holds too. Unpublished
//! manifests and their objects are never cleared, the local registry is never
//! touched, and nothing is cleared in a store whose bucket cannot be asked,
//! as offline. [`gc_estimate`] counts the same, clearing nothing.

use std::collections::BTreeMap;

use tracing::info;

use crate::Error;
use crate::Res;
use crate::domain::DomainRoot;
use crate::domain::RegistryStore;
use crate::flow::GcReport;
use crate::io::remote::Remote;
use crate::io::storage::Storage;
use quilt_uri::Host;
use quilt_uri::Namespace;

/// What a gc did in one bucket's Package Store.
#[derive(Debug)]
#[non_exhaustive]
pub enum StoreGc {
    /// What was cleared, or with [`gc_estimate`] what would be.
    Freed(GcReport),
    /// Another writer held this package's lock, so nothing was cleared.
    Busy(Namespace),
    /// The bucket could not be asked about every manifest, as when offline,
    /// or the store could not be read; nothing was cleared.
    Failed(Error),
}

/// What [`gc`] or [`gc_estimate`] found, per bucket.
#[derive(Debug, Default)]
#[non_exhaustive]
pub struct Collected {
    pub stores: BTreeMap<String, StoreGc>,
}

impl Collected {
    /// What was freed in every store, summed.
    #[must_use]
    pub fn total(&self) -> GcReport {
        let mut total = GcReport::default();
        for store in self.stores.values() {
            if let StoreGc::Freed(report) = store {
                total += *report;
            }
        }
        total
    }

    /// Whether some store was left as it was because it was busy or could
    /// not be read.
    #[must_use]
    pub fn incomplete(&self) -> bool {
        self.stores
            .values()
            .any(|store| !matches!(store, StoreGc::Freed(_)))
    }
}

/// Clears, in each bucket's Package Store in `domain`, what its bucket holds
/// too: the manifests the bucket has, installed and fetched, and the objects
/// no manifest the store keeps names. A manifest the bucket lacks, such as
/// an unpushed commit, is kept with its objects, and the local registry is
/// never touched (25.12).
///
/// Each store is cleared on its own, holding the lock of every package it
/// has, as [`RegistryStore::clear_published`] clears one: a store whose
/// bucket cannot be asked about every manifest, as when offline, or one
/// with a busy package, is left as it is and reported, and the others are
/// still cleared. `host` gives the catalog that signs the reads of a bucket,
/// or `None` for this machine's own AWS credentials.
pub async fn gc(
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    domain: &DomainRoot,
    host: impl Fn(&str) -> Option<Host>,
) -> Res<Collected> {
    info!(
        "⏳ Clearing what the buckets hold in {}",
        domain.root().display()
    );
    let collected = each(storage, domain, async |store| {
        store
            .clear_published(storage, remote, host(store.bucket()).as_ref())
            .await
    })
    .await?;
    info!("✔️ Cleared {}", collected.total());
    Ok(collected)
}

/// What [`gc`] would clear now, clearing nothing: the same walk and the same
/// questions to each bucket, so it needs the network as gc does. It takes no
/// package lock, so a busy package is not reported, and a writer meanwhile
/// can change the answer; gc's own report is the exact one.
pub async fn gc_estimate(
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    domain: &DomainRoot,
    host: impl Fn(&str) -> Option<Host>,
) -> Res<Collected> {
    each(storage, domain, async |store| {
        store
            .estimate_published(storage, remote, host(store.bucket()).as_ref())
            .await
    })
    .await
}

/// Runs `clear` on each bucket's store in `domain`, keeping each outcome.
async fn each(
    storage: &(impl Storage + Sync),
    domain: &DomainRoot,
    clear: impl AsyncFn(&RegistryStore) -> Res<GcReport>,
) -> Res<Collected> {
    let mut collected = Collected::default();
    for store in domain.registries(storage).await? {
        let outcome = match clear(&store).await {
            Ok(report) => StoreGc::Freed(report),
            Err(Error::PackageBusy(namespace)) => StoreGc::Busy(namespace),
            Err(err) => StoreGc::Failed(err),
        };
        collected.stores.insert(store.bucket().to_string(), outcome);
    }
    Ok(collected)
}

#[cfg(test)]
mod tests;
