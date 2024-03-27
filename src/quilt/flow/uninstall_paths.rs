use std::io::ErrorKind;
use std::path::PathBuf;

use crate::quilt::lineage::PackageLineage;
use crate::quilt::storage::fs::RemoveFile;
use crate::Error;

fn not_found_error(path: &str) -> Error {
    Error::Uninstall(format!("path {} not found. Cannot uninstall.", path))
}

pub async fn uninstall_paths(
    mut lineage: PackageLineage,
    working_dir: PathBuf,
    fs_impl: impl RemoveFile,
    paths: &Vec<String>,
) -> Result<PackageLineage, Error> {
    tracing::debug!("Uninstalling paths {:?}", paths);

    for path in paths {
        lineage.paths.remove(path).ok_or(not_found_error(path))?;

        let working_path = working_dir.join(path);
        if let Err(err) = fs_impl.remove_file(working_path).await {
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

    use crate::quilt::lineage::PathState;
    use std::collections::BTreeMap;

    struct MockFs {}
    impl RemoveFile for MockFs {
        async fn remove_file(&self, _path: PathBuf) -> Result<(), std::io::Error> {
            // TODO: detect it was removed
            Ok(())
        }
    }

    #[tokio::test]
    async fn uninstall_not_installed_path() -> Result<(), Error> {
        let file_ops = MockFs {};
        let lineage = PackageLineage::default();
        let paths = vec!["test folde/r".to_string()];
        let modified_lineage = uninstall_paths(lineage, PathBuf::new(), file_ops, &paths).await;
        assert_eq!(
            modified_lineage.unwrap_err().to_string(),
            "Uninstall error: path test folde/r not found. Cannot uninstall."
        );
        Ok(())
    }

    #[tokio::test]
    async fn uninstall_single_path() -> Result<(), Error> {
        let lineage = PackageLineage {
            paths: BTreeMap::from([
                ("a/a".to_string(), PathState::default()),
                ("test folde/r".to_string(), PathState::default()),
                ("b/b".to_string(), PathState::default()),
            ]),
            ..PackageLineage::default()
        };
        let s = MockFs {};
        let paths = vec!["test folde/r".to_string()];
        let modified_lineage = uninstall_paths(lineage, PathBuf::new(), s, &paths).await?;
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
        let lineage = PackageLineage {
            paths: BTreeMap::from([
                ("a/a".to_string(), PathState::default()),
                ("b/b".to_string(), PathState::default()),
            ]),
            ..PackageLineage::default()
        };
        let paths = vec!["b/b".to_string(), "a/a".to_string()];
        let s = MockFs {};
        let modified_lineage = uninstall_paths(lineage, PathBuf::new(), s, &paths).await?;
        assert!(modified_lineage.paths.is_empty());
        Ok(())
    }
}
