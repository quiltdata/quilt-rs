use std::io::ErrorKind;
use std::path::PathBuf;

use tracing::log;

use crate::io::storage::Storage;
use crate::lineage::PackageLineage;
use crate::Error;

fn not_found_error(path: &PathBuf) -> Error {
    Error::Uninstall(format!("path {:?} not found. Cannot uninstall.", path))
}

pub async fn uninstall_paths(
    mut lineage: PackageLineage,
    working_dir: PathBuf,
    storage: &impl Storage,
    paths: &Vec<PathBuf>,
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

    use crate::io::storage::mocks::MockStorage;
    use crate::quilt::mocks;

    #[tokio::test]
    async fn uninstall_not_installed_path() -> Result<(), Error> {
        let storage = MockStorage::default();
        let lineage = PackageLineage::default();
        let paths = vec![PathBuf::from("test folde/r")];

        let modified_lineage = uninstall_paths(lineage, PathBuf::new(), &storage, &paths).await;
        assert_eq!(
            modified_lineage.unwrap_err().to_string(),
            r#"Uninstall error: path "test folde/r" not found. Cannot uninstall."#
        );
        Ok(())
    }

    #[tokio::test]
    async fn uninstall_single_path() -> Result<(), Error> {
        let installed_paths = vec![
            PathBuf::from("a/a"),
            PathBuf::from("test folde/r"),
            PathBuf::from("b/b"),
        ];
        let lineage = mocks::lineage::with_paths(installed_paths);

        let storage = MockStorage::default();
        storage
            .write_file(PathBuf::from("a/a"), &Vec::new())
            .await?;
        storage
            .write_file(PathBuf::from("test folde/r"), &Vec::new())
            .await?;
        storage
            .write_file(PathBuf::from("b/b"), &Vec::new())
            .await?;

        let key = PathBuf::from("test folde/r");
        assert!(storage.exists(&key).await);

        let modified_lineage =
            uninstall_paths(lineage, PathBuf::new(), &storage, &vec![key.clone()]).await?;

        // Check that the key was removed
        assert!(!storage.exists(&key).await);

        assert_eq!(
            modified_lineage.paths,
            BTreeMap::from([
                (PathBuf::from("a/a"), mocks::lineage::path_state()),
                (PathBuf::from("b/b"), mocks::lineage::path_state()),
            ])
        );
        Ok(())
    }

    #[tokio::test]
    async fn uninstall_multiple_paths() -> Result<(), Error> {
        let lineage = mocks::lineage::with_paths(vec![PathBuf::from("a/a"), PathBuf::from("b/b")]);
        let paths = vec![PathBuf::from("b/b"), PathBuf::from("a/a")];
        let storage = MockStorage::default();
        let modified_lineage = uninstall_paths(lineage, PathBuf::new(), &storage, &paths).await?;
        assert!(modified_lineage.paths.is_empty());
        Ok(())
    }
}
