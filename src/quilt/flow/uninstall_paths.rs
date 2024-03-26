use std::io::ErrorKind;
use std::path::PathBuf;
use tokio::fs;

use crate::quilt::lineage::PackageLineage;
use crate::Error;

fn not_found_error(path: &str) -> Error {
    Error::Uninstall(format!("path {} not found. Cannot uninstall.", path))
}

pub async fn uninstall_paths(
    mut lineage: PackageLineage,
    working_dir: PathBuf,
    paths: &Vec<String>,
) -> Result<PackageLineage, Error> {
    tracing::debug!("Uninstalling paths {:?}", paths);

    for path in paths {
        lineage.paths.remove(path).ok_or(not_found_error(path))?;

        let working_path = working_dir.join(path);
        if let Err(err) = fs::remove_file(working_path).await {
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

    #[tokio::test]
    async fn uninstall_not_installed_path() -> Result<(), Error> {
        let lineage = PackageLineage::default();
        let paths = vec!["test folde/r".to_string()];
        let modified_lineage = uninstall_paths(lineage, PathBuf::new(), &paths).await;
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
        let paths = vec!["test folde/r".to_string()];
        let modified_lineage = uninstall_paths(lineage, PathBuf::new(), &paths).await?;
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
        let modified_lineage = uninstall_paths(lineage, PathBuf::new(), &paths).await?;
        assert!(modified_lineage.paths.is_empty());
        Ok(())
    }
}
