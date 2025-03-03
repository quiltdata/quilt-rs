use std::io::ErrorKind;
use std::path::PathBuf;

use tracing::debug;
use tracing::error;
use tracing::info;

use crate::io::storage::Storage;
use crate::lineage::PackageLineage;
use crate::Error;
use crate::Res;

fn not_found_error(path: &PathBuf) -> Error {
    Error::Uninstall(format!("path {:?} not found. Cannot uninstall.", path))
}

/// Uninstalls paths: remote files from working directory and stop tracking in `.quilt/lineage.json`.
pub async fn uninstall_paths(
    mut lineage: PackageLineage,
    working_dir: PathBuf,
    storage: &impl Storage,
    paths: &Vec<PathBuf>,
) -> Res<PackageLineage> {
    info!("⏳ Uninstalling {} paths", paths.len());

    for path in paths {
        debug!("⏳ Processing path: {}", path.display());

        debug!("⏳ Removing path from lineage");
        lineage.paths.remove(path).ok_or(not_found_error(path))?;
        debug!("✔️ Path removed from lineage");

        let working_path = working_dir.join(path);
        debug!("⏳ Removing file from {}", working_path.display());
        if let Err(err) = storage.remove_file(working_path).await {
            if err.kind() != ErrorKind::NotFound {
                return Err(Error::Io(err));
            }
            error!("❌ Failed to remove: {:?}", err);
        } else {
            debug!("✔️ File removed successfully");
        }
    }

    // TODO: Remove unused files in OBJECTS_DIR?

    info!("✔️ Successfully uninstalled {} paths", paths.len());
    Ok(lineage)
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::BTreeMap;

    use crate::fixtures;
    use crate::fixtures::sample_file_1;

    #[tokio::test]
    async fn uninstall_not_installed_path() -> Res {
        let storage = fixtures::storage::MockStorage::default();
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
    async fn uninstall_single_path() -> Res {
        let lineage = PackageLineage {
            paths: BTreeMap::from([
                (PathBuf::from("a/a"), sample_file_1::path_state()),
                (PathBuf::from("test folde/r"), sample_file_1::path_state()),
                (PathBuf::from("b/b"), sample_file_1::path_state()),
            ]),
            ..PackageLineage::default()
        };

        let storage = fixtures::storage::MockStorage::default();
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
                (PathBuf::from("a/a"), sample_file_1::path_state()),
                (PathBuf::from("b/b"), sample_file_1::path_state()),
            ])
        );
        Ok(())
    }

    #[tokio::test]
    async fn uninstall_multiple_paths() -> Res {
        let lineage = PackageLineage {
            paths: BTreeMap::from([
                (PathBuf::from("a/a"), sample_file_1::path_state()),
                (PathBuf::from("b/b"), sample_file_1::path_state()),
            ]),
            ..PackageLineage::default()
        };

        let paths = vec![PathBuf::from("b/b"), PathBuf::from("a/a")];
        let storage = fixtures::storage::MockStorage::default();
        let modified_lineage = uninstall_paths(lineage, PathBuf::new(), &storage, &paths).await?;
        assert!(modified_lineage.paths.is_empty());
        Ok(())
    }
}
