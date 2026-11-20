//! What the dataspace commands share: the Domain, where one exists, whose
//! local registry they reach, and the writer they write the lock as.

use std::path::Path;

use quilt_rs::dataspace::lock::Writer;
use quilt_rs::domain::DomainRoot;
use quilt_rs::domain::LocalRegistry;
use quilt_rs::io::storage::LocalStorage;

use crate::cli::Error;

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
}
