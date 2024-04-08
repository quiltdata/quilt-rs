use std::io::ErrorKind;
use std::path::PathBuf;

use tracing::log;

use crate::quilt::lineage::PackageLineage;
use crate::quilt::storage::Storage;
use crate::Error;

fn not_found_error(path: &str) -> Error {
    Error::Uninstall(format!("path {} not found. Cannot uninstall.", path))
}

pub async fn uninstall_paths(
    mut lineage: PackageLineage,
    working_dir: PathBuf,
    storage: &mut impl Storage,
    paths: &Vec<String>,
) -> Result<PackageLineage, Error> {
    log::debug!("Uninstalling paths {:?}", paths);

    for path in paths {
        lineage.paths.remove(path).ok_or(not_found_error(path))?;

        let working_path = working_dir.join(path);
        if let Err(err) = storage.remove_file(working_path).await {
            if err.kind() != ErrorKind::NotFound {
                return Err(Error::Io(err));
            }
        }
    }

    // TODO: Remove unused files in OBJECTS_DIR?

    Ok(lineage)
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::BTreeMap;

    use crate::quilt::lineage::PathState;
    use crate::quilt::mocks;
    use crate::quilt::storage::mock_storage::MockStorage;

    #[tokio::test]
    async fn uninstall_not_installed_path() -> Result<(), Error> {
        let mut storage = MockStorage::default();
        let lineage = PackageLineage::default();
        let paths = vec!["test folde/r".to_string()];

        let modified_lineage = uninstall_paths(lineage, PathBuf::new(), &mut storage, &paths).await;
        assert_eq!(
            modified_lineage.unwrap_err().to_string(),
            "Uninstall error: path test folde/r not found. Cannot uninstall."
        );
        Ok(())
    }

    #[tokio::test]
    async fn uninstall_single_path() -> Result<(), Error> {
        let installed_paths = vec!["a/a", "test folde/r", "b/b"];
        let lineage = mocks::lineage::with_paths(&installed_paths);

        let mut storage = MockStorage::with_keys(&installed_paths);

        let paths_to_uninstall = vec!["test folde/r".to_string()];

        let key = PathBuf::from("test folde/r");
        assert!(storage.registry.get(&key).is_some(),);

        let modified_lineage =
            uninstall_paths(lineage, PathBuf::new(), &mut storage, &paths_to_uninstall).await?;

        // Check that the key was removed
        assert!(storage.registry.get(&key).is_none());

        assert_eq!(
            modified_lineage.paths,
            BTreeMap::from([
                ("a/a".to_string(), PathState::default()),
                ("b/b".to_string(), PathState::default()),
            ])
        );
        Ok(())
    }

    #[tokio::test]
    async fn uninstall_multiple_paths() -> Result<(), Error> {
        let lineage = mocks::lineage::with_paths(&vec!["a/a", "b/b"]);
        let paths = vec!["b/b".to_string(), "a/a".to_string()];
        let mut storage = MockStorage::default();
        let modified_lineage =
            uninstall_paths(lineage, PathBuf::new(), &mut storage, &paths).await?;
        assert!(modified_lineage.paths.is_empty());
        Ok(())
    }
}
