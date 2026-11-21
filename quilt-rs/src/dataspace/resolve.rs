//! A short top hash, resolved among the revisions a store of the Domain
//! holds for one package: quilt3's `Package.resolve_hash` on a dataspace
//! package, with [`flow::resolve_local_hash`]'s rules. Only the package's
//! own installed manifests count, never the bucket's fetched ones in
//! `packages/` (quilt-specs#297's unit-local-hash, entry 27d).

use crate::Res;
use crate::dataspace::contract::PackageKey;
use crate::dataspace::contract::Registry;
use crate::domain::DomainRoot;
use crate::flow;
use crate::io::storage::Storage;

/// The one revision of `package` in its store in `domain` whose top hash
/// starts with `prefix`: the bucket's Package Store, or the local registry
/// by any name it has had. A local registry this Domain's does not answer
/// to holds no revisions. Reads only.
///
/// # Errors
///
/// [`flow::resolve_local_hash`]'s: a prefix that is too short or not hex,
/// none or more than one revision starting with it, and a store that
/// cannot be listed.
pub async fn resolve_hash(
    storage: &(impl Storage + Sync),
    domain: &DomainRoot,
    package: &PackageKey,
    prefix: &str,
) -> Res<String> {
    let manifests = match &package.registry {
        Registry::S3(bucket) => domain
            .registry(bucket)?
            .installed_manifests_dir(&package.package),
        Registry::Local(name) => {
            let local = domain.local();
            let held = local
                .read_names(storage)
                .await?
                .is_some_and(|names| names.answers_to(name));
            if held {
                local.installed_manifests_dir(&package.package)
            } else {
                // Another registry's revisions are not here: a folder that
                // is not there holds none, and the prefix is still checked.
                domain.root().join("no such registry")
            }
        }
    };
    flow::resolve_local_hash(storage, manifests, prefix).await
}

#[cfg(test)]
mod tests;
