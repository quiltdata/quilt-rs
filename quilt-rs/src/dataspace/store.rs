//! Where an entry's package is kept on this machine: its bucket's Package
//! Store, or the local registry, which is a registry in its own right and
//! its own store.

use std::path::PathBuf;

use crate::dataspace::EntryError;
use crate::dataspace::contract::Contract;
use crate::dataspace::contract::ContractError;
use crate::dataspace::contract::Entry;
use crate::dataspace::contract::PackageKey;
use crate::dataspace::contract::Registry;
use crate::dataspace::lock::Lock;
use crate::dataspace::lock::Writer;
use crate::domain::DomainRoot;
use crate::domain::LocalNames;
use crate::domain::LocalRegistry;
use crate::domain::RegistryStore;
use crate::flow::PackageStore;
use crate::io::storage::Storage;
use quilt_uri::Namespace;

/// The local registry `entry` names, where `local` answers to that name or
/// to one it had; `None` for a bucket's entry. An entry that names another
/// local registry, or one with no local registry given, is refused.
pub(crate) async fn local_registry<'a>(
    storage: &(impl Storage + Sync),
    local: Option<&'a LocalRegistry>,
    entry: &Entry,
) -> Result<Option<&'a LocalRegistry>, EntryError> {
    let Registry::Local(name) = &entry.registry else {
        return Ok(None);
    };
    let Some(local) = local else {
        return Err(EntryError::LocalRegistry);
    };
    if local.names(storage).await?.answers_to(name) {
        Ok(Some(local))
    } else {
        Err(EntryError::LocalRegistry)
    }
}

/// Names `local` by its current name wherever `contract`, `lock` or the
/// lock's `writer` names it by a name it had: each entry's key, as
/// [`Contract::name_local`] does, each lock key, as [`Lock::name_local`]
/// does, and the local registry the lock's header names, so that the lock
/// is written with the current name. Nothing is changed where none of them
/// names a local registry, or the names cannot be read: a local entry is
/// then refused when it is reached.
pub(crate) async fn name_local(
    storage: &(impl Storage + Sync),
    local: Option<&LocalRegistry>,
    contract: &mut Contract,
    lock: &mut Lock,
    writer: &mut Writer,
) -> Result<(), ContractError> {
    let Some(names) = local_names(storage, local, contract, Some(lock)).await else {
        return Ok(());
    };
    contract.name_local(&names)?;
    lock.name_local(&names);
    if names.answers_to(&writer.local_registry) {
        writer.local_registry.clone_from(&names.name);
    }
    Ok(())
}

/// `package` under the local registry's current name, where `local`
/// answers to the name it gives.
pub(crate) async fn current_key(
    storage: &(impl Storage + Sync),
    local: Option<&LocalRegistry>,
    package: &PackageKey,
) -> crate::Res<PackageKey> {
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

/// The names of `local`, where `contract` or `lock` names a local registry
/// and they can be read; see [`name_local`].
pub(crate) async fn local_names(
    storage: &(impl Storage + Sync),
    local: Option<&LocalRegistry>,
    contract: &Contract,
    lock: Option<&Lock>,
) -> Option<LocalNames> {
    let local = local?;
    let is_local = |registry: &Registry| matches!(registry, Registry::Local(_));
    let names_one = contract
        .entries()
        .iter()
        .any(|entry| is_local(&entry.registry))
        || lock.is_some_and(|lock| lock.packages.keys().any(|key| is_local(&key.registry)));
    if !names_one {
        return None;
    }
    local.names(storage).await.ok()
}

/// The Package Store an entry's commits go to in the Domain.
#[derive(Clone, Debug)]
pub(crate) enum Store {
    Bucket(RegistryStore),
    Local(LocalRegistry),
}

impl Store {
    /// `entry`'s store in `domain`: its bucket's, or the local registry it
    /// names, which must be the Domain's.
    pub(crate) async fn of(
        storage: &(impl Storage + Sync),
        domain: &DomainRoot,
        entry: &Entry,
    ) -> Result<Self, EntryError> {
        match &entry.registry {
            Registry::S3(bucket) => Ok(Self::Bucket(domain.registry(bucket)?)),
            Registry::Local(_) => {
                let local = domain.local();
                local_registry(storage, Some(&local), entry).await?;
                Ok(Self::Local(local))
            }
        }
    }

    /// The local registry, where this is it.
    pub(crate) fn local(&self) -> Option<&LocalRegistry> {
        match self {
            Self::Bucket(_) => None,
            Self::Local(local) => Some(local),
        }
    }

    pub(crate) fn package_lock(&self, namespace: &Namespace) -> PathBuf {
        match self {
            Self::Bucket(store) => store.package_lock(namespace),
            Self::Local(local) => local.package_lock(namespace),
        }
    }

    pub(crate) fn installed_manifests_dir(&self, namespace: &Namespace) -> PathBuf {
        match self {
            Self::Bucket(store) => store.installed_manifests_dir(namespace),
            Self::Local(local) => local.installed_manifests_dir(namespace),
        }
    }

    pub(crate) fn installed_manifest(&self, namespace: &Namespace, hash: &str) -> PathBuf {
        self.installed_manifests_dir(namespace).join(hash)
    }
}

impl PackageStore for Store {
    fn objects_dir(&self) -> PathBuf {
        match self {
            Self::Bucket(store) => store.objects_dir(),
            Self::Local(local) => local.objects_dir(),
        }
    }

    fn manifests_dir(&self, namespace: &Namespace) -> PathBuf {
        self.installed_manifests_dir(namespace)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::collections::BTreeMap;

    use super::*;

    use crate::LogicalKey;
    use crate::flow::WorkflowCheck;
    use crate::flow::record;
    use crate::io::manifest::RowContent;
    use crate::io::remote::mocks::MockRemote;
    use crate::io::storage::StorageExt;
    use crate::lineage::PackageLineage;
    use crate::manifest::Manifest;
    use crate::manifest::ManifestRow;

    /// Publishes `files`, and only them, as a revision of `package` in
    /// `local` on its `latest`, as another dataspace's publish would, and
    /// returns its hash.
    pub(crate) async fn publish_local(
        storage: &(impl Storage + Sync),
        local: &LocalRegistry,
        package: &str,
        files: &[(&str, &[u8])],
    ) -> Result<String, Box<dyn std::error::Error>> {
        let namespace = Namespace::try_from(package)?;
        let latest = local.read_latest(storage, &namespace).await?;
        let mut rows = Vec::new();
        let mut content = BTreeMap::new();
        for (key, body) in files {
            let key = LogicalKey::new(*key)?;
            rows.push(ManifestRow {
                logical_key: key.clone(),
                ..ManifestRow::default()
            });
            content.insert(key, RowContent::Bytes(body.to_vec().into()));
        }
        let lineage = PackageLineage {
            base_hash: latest.clone().unwrap_or_default(),
            ..PackageLineage::default()
        };
        let manifest = Manifest {
            rows,
            ..Manifest::default()
        };
        let (_, recorded) = record(
            local,
            storage,
            &MockRemote::default(),
            WorkflowCheck::Warn { host: None },
            &namespace,
            lineage,
            manifest,
            &content,
        )
        .await?;
        let hash = recorded.hash().ok_or("nothing recorded")?.to_string();
        local
            .publish(storage, &namespace, latest.as_deref(), &hash)
            .await?;
        Ok(hash)
    }

    /// The bytes of `key` in the revision `hash` of the local registry.
    pub(crate) async fn local_bytes(
        storage: &(impl Storage + Sync),
        local: &LocalRegistry,
        hash: &str,
        key: &str,
    ) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        let manifest = Manifest::from_path(storage, &local.revision(hash)).await?;
        let row = manifest
            .rows
            .iter()
            .find(|row| row.logical_key.to_string_lossy() == key)
            .ok_or("no such key")?;
        Ok(storage.read_bytes(local.object_path(&row.hash)).await?)
    }
}
