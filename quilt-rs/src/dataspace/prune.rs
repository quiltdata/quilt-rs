//! `prune`: what the Domain's store holds of one package that its registry
//! holds too, cleared (25.12). Today's pruning uninstall deletes the
//! package's objects that no other installed package uses; here only a
//! bucket's Package Store is a cache, so only published content is cleared.

use crate::Error;
use crate::Res;
use crate::dataspace::contract::PackageKey;
use crate::dataspace::contract::Registry;
use crate::domain::DomainRoot;
use crate::flow::GcReport;
use crate::io::remote::Remote;
use crate::io::storage::Storage;
use quilt_uri::Host;
use quilt_uri::Namespace;

/// What [`prune`] did.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Pruned {
    /// What was cleared from the bucket's store.
    Freed(GcReport),
    /// Another writer held this package's lock, so nothing was cleared.
    Busy(Namespace),
    /// The package is in the local registry, which holds the only copy and
    /// is never cleared.
    LocalRegistry,
}

impl std::fmt::Display for Pruned {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Freed(report) => write!(f, "{report}"),
            Self::Busy(namespace) => write!(
                f,
                "Nothing was cleared: {namespace} is busy in another quilt process"
            ),
            Self::LocalRegistry => {
                f.write_str("Nothing was cleared: the local registry holds the only copy")
            }
        }
    }
}

/// Clears from `domain` what it holds of `package` that the package's
/// registry holds too: in its bucket's Package Store, the revisions installed
/// or committed here that the bucket has, and the objects only they name.
///
/// Unpublished manifests, such as unpushed commits, are kept with their
/// objects, and nothing in the local registry is cleared. Nothing at all is
/// cleared when the bucket cannot be asked about every revision, as when
/// offline; that is an error. `host` is the catalog that signs the reads.
pub async fn prune(
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    domain: &DomainRoot,
    package: &PackageKey,
    host: Option<&Host>,
) -> Res<Pruned> {
    let Registry::S3(bucket) = &package.registry else {
        return Ok(Pruned::LocalRegistry);
    };
    let store = domain.registry(bucket)?;
    match store
        .clear_package(storage, remote, host, &package.package)
        .await
    {
        Ok(report) => Ok(Pruned::Freed(report)),
        Err(Error::PackageBusy(namespace)) => Ok(Pruned::Busy(namespace)),
        Err(err) => Err(err),
    }
}

#[cfg(test)]
mod tests;
