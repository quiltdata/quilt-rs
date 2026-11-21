//! `measure_storage`: how much the Domain holds, as today's measure says
//! how much `.quilt/` holds: every byte under `bio.quilt.dataspace/`, and
//! each registry's store's share of it.

use std::collections::BTreeMap;

use crate::Res;
use crate::domain::DomainRoot;
use crate::flow::tree_bytes;
use crate::io::storage::Storage;

/// How much the Domain holds.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct DomainSize {
    /// Every file's length under the Domain's folder, summed: the stores,
    /// and `queue.db`, `state.db` and the rest beside them. A symlink counts
    /// as itself, not what it points at.
    pub total_bytes: u64,
    /// Each bucket's Package Store, by bucket.
    pub registries: BTreeMap<String, u64>,
    /// The local registry, which holds the only copy of its packages and is
    /// never cleared.
    pub local_bytes: u64,
}

/// Measures `domain`: its total size, each bucket's store and the local
/// registry. Takes no lock, reads no bucket and deletes nothing; what
/// [`gc`](crate::dataspace::gc::gc) would free is
/// [`gc_estimate`](crate::dataspace::gc::gc_estimate)'s to say, since it
/// asks the buckets.
pub async fn measure_storage(
    storage: &(impl Storage + Sync),
    domain: &DomainRoot,
) -> Res<DomainSize> {
    let mut registries = BTreeMap::new();
    for store in domain.registries(storage).await? {
        let bytes = tree_bytes(storage, store.dir()).await?;
        registries.insert(store.bucket().to_string(), bytes);
    }
    Ok(DomainSize {
        total_bytes: tree_bytes(storage, domain.root()).await?,
        registries,
        local_bytes: tree_bytes(storage, domain.local().dir()).await?,
    })
}

#[cfg(test)]
mod tests;
