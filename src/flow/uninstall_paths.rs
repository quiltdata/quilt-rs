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

/// Uninstalls paths: remote files from home directory and stop tracking in `.quilt/lineage.json`.
pub async fn uninstall_paths(
    mut lineage: PackageLineage,
    package_home: PathBuf,
    storage: &impl Storage,
    paths: &Vec<PathBuf>,
) -> Res<PackageLineage> {
    info!("⏳ Uninstalling {} paths", paths.len());

    for path in paths {
        debug!("⏳ Processing path: {}", path.display());

        debug!("⏳ Removing path from lineage");
        lineage.paths.remove(path).ok_or(not_found_error(path))?;
        debug!("✔️ Path removed from lineage");

        let object_home_path = package_home.join(path);
        debug!("⏳ Removing file from {}", object_home_path.display());
        if let Err(err) = storage.remove_file(object_home_path).await {
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
    use crate::io::storage::mocks::MockStorage;
    use crate::lineage::PathState;

    #[tokio::test]
    async fn uninstall_not_installed_path() -> Res {
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
    async fn uninstall_single_path() -> Res {
        let logical_key_zero = PathBuf::from("0mb.bin");
        let logical_key_less_than_8mb = PathBuf::from("less-then-8mb.txt");
        let logical_key_nested = PathBuf::from("one/two two/three three three/READ ME.md");

        let lineage = PackageLineage {
            paths: BTreeMap::from([
                (logical_key_zero.clone(), PathState::default()),
                (logical_key_less_than_8mb.clone(), PathState::default()),
                (logical_key_nested.clone(), PathState::default()),
            ]),
            ..PackageLineage::default()
        };

        let storage = MockStorage::default();
        storage
            .write_file(&logical_key_zero, fixtures::objects::zero_bytes())
            .await?;
        storage
            .write_file(
                &logical_key_less_than_8mb,
                fixtures::objects::less_than_8mb(),
            )
            .await?;
        storage
            .write_file(&logical_key_nested, fixtures::objects::nested())
            .await?;

        assert!(storage.exists(&logical_key_nested).await);

        let modified_lineage = uninstall_paths(
            lineage,
            PathBuf::new(),
            &storage,
            &vec![logical_key_nested.clone()],
        )
        .await?;

        // Check that the key was removed
        assert!(!storage.exists(&logical_key_nested).await);

        assert_eq!(
            modified_lineage.paths,
            BTreeMap::from([
                (logical_key_zero.clone(), PathState::default()),
                (logical_key_less_than_8mb.clone(), PathState::default()),
            ])
        );
        Ok(())
    }

    #[tokio::test]
    async fn uninstall_multiple_paths() -> Res {
        let logical_key_zero = PathBuf::from("0mb.bin");
        let logical_key_nested = PathBuf::from("one/two two/three three three/READ ME.md");
        let lineage = PackageLineage {
            paths: BTreeMap::from([
                (logical_key_zero.clone(), PathState::default()),
                (logical_key_nested.clone(), PathState::default()),
            ]),
            ..PackageLineage::default()
        };

        let paths = vec![logical_key_zero, logical_key_nested];
        let storage = MockStorage::default();
        let modified_lineage = uninstall_paths(lineage, PathBuf::new(), &storage, &paths).await?;
        assert!(modified_lineage.paths.is_empty());
        Ok(())
    }
}
