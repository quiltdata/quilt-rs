//! What the dataspace commands share: the Domain, where one exists, whose
//! local registry they reach, and who they write or read the lock as.

use std::path::Path;

use quilt_rs::dataspace::contract::Contract;
use quilt_rs::dataspace::contract::Entry;
use quilt_rs::dataspace::contract::PackageKey;
use quilt_rs::dataspace::lock::Writer;
use quilt_rs::domain::DomainRoot;
use quilt_rs::domain::LocalRegistry;
use quilt_rs::io::storage::LocalStorage;
use quilt_uri::Namespace;

use crate::cli::Error;

/// How a dataspace command names a package: by its key, or by a name the
/// contract gives one package.
#[derive(Debug)]
pub enum Package {
    Key(PackageKey),
    Namespace(Namespace),
}

impl Package {
    /// The package a command's positional URI or `--namespace` names.
    pub(super) fn named(key: Option<PackageKey>, namespace: Option<String>) -> Result<Self, Error> {
        match (key, namespace) {
            (Some(key), _) => Ok(Self::Key(key)),
            (None, Some(namespace)) => Ok(Self::Namespace(namespace.try_into()?)),
            (None, None) => Err(Error::PackageRequired),
        }
    }

    /// [`Package::named`], the URI given as its text.
    pub(super) fn uri(uri: Option<String>, namespace: Option<String>) -> Result<Self, Error> {
        let key = uri.map(|uri| uri.parse::<PackageKey>()).transpose()?;
        Self::named(key, namespace)
    }

    /// The package's key in `contract`: a key as it is, and a name as the
    /// one package of the contract that has it.
    pub(super) fn key_in(self, contract: &Contract) -> Result<PackageKey, Error> {
        let namespace = match self {
            Self::Key(key) => return Ok(key),
            Self::Namespace(namespace) => namespace,
        };
        let mut keys: Vec<PackageKey> = contract
            .entries()
            .iter()
            .filter(|entry| entry.package == namespace)
            .map(Entry::key)
            .collect();
        keys.dedup();
        match keys.as_slice() {
            [key] => Ok(key.clone()),
            [] => Err(Error::NamespaceNotFound(namespace)),
            _ => Err(Error::AmbiguousPackage(namespace)),
        }
    }

    /// [`Package::key_in`] the contract of the dataspace in the folder
    /// `dataspace`, read as `writer` reads it.
    pub(super) async fn key(self, dataspace: &Path, writer: &Writer) -> Result<PackageKey, Error> {
        let storage = LocalStorage::new();
        let contract = Contract::read(&storage, dataspace, writer.local_registry_name()).await?;
        self.key_in(&contract)
    }
}

/// The Domain in `dir`, where one exists there. A dataspace command with no
/// Domain makes none: it reads and publishes bucket packages only.
pub(super) fn existing(dir: &Path) -> Option<DomainRoot> {
    dir.is_dir().then(|| DomainRoot::new(dir))
}

/// The writer of a lock on this machine, naming `local`'s name, or none
/// where there is no local registry.
pub(super) async fn writer(local: Option<&LocalRegistry>) -> Result<Writer, Error> {
    let name = match local {
        Some(local) => local.names(&LocalStorage::new()).await?.name,
        None => String::new(),
    };
    Ok(Writer::here(name)?)
}

/// Who a command that writes nothing compares the lock's header with: this
/// machine's host and user, and the name `domain`'s local registry keeps,
/// read but never given, so a registry not named yet stays unnamed.
pub(super) async fn reader(domain: Option<&DomainRoot>) -> Result<Writer, Error> {
    let name = match domain {
        Some(domain) => domain
            .local()
            .read_names(&LocalStorage::new())
            .await?
            .map_or_else(String::new, |names| names.name),
        None => String::new(),
    };
    Ok(Writer::here(name)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    use test_log::test;

    #[test]
    fn a_folder_that_is_not_there_is_no_domain() -> Result<(), Error> {
        let dir = tempfile::TempDir::new()?;

        assert!(existing(&dir.path().join("bio.quilt.dataspace")).is_none());
        assert_eq!(
            existing(dir.path()).map(|domain| domain.root().to_path_buf()),
            Some(dir.path().to_path_buf())
        );
        Ok(())
    }

    #[test(tokio::test)]
    async fn the_writer_names_the_local_registry_given() -> Result<(), Error> {
        let dir = tempfile::TempDir::new()?;
        let local = DomainRoot::new(dir.path()).local();
        local.rename(&LocalStorage::new(), "lab-mac").await?;

        assert_eq!(writer(Some(&local)).await?.local_registry, "lab-mac");
        assert_eq!(writer(None).await?.local_registry_name(), None);
        Ok(())
    }

    /// The reader names the local registry by the name it keeps, and does
    /// not name one never named.
    #[test(tokio::test)]
    async fn the_reader_reads_the_local_registry_name_and_gives_none() -> Result<(), Error> {
        let dir = tempfile::TempDir::new()?;
        let domain = DomainRoot::new(dir.path());

        assert_eq!(reader(Some(&domain)).await?.local_registry_name(), None);
        assert!(!dir.path().join("registries").exists());

        domain
            .local()
            .rename(&LocalStorage::new(), "lab-mac")
            .await?;
        assert_eq!(reader(Some(&domain)).await?.local_registry, "lab-mac");
        assert_eq!(reader(None).await?.local_registry_name(), None);
        Ok(())
    }
}
