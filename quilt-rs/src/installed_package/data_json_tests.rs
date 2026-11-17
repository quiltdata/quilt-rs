//! What a package's flows write to `data.json` itself.

use test_log::test;

use super::on_data_json::sync_flow_tests::committed_data_txt;
use crate::Res;

/// Builds that require `paths[].hash` cannot load an entry without it.
#[test(tokio::test)]
async fn a_committed_file_is_stored_with_its_hash() -> Res {
    let (package, _temp_dir) = committed_data_txt().await?;
    let record: serde_json::Value =
        serde_json::from_slice(&std::fs::read(package.paths.lineage())?)?;
    let entry = &record["packages"]["demo/tracked"]["paths"]["data.txt"];
    assert!(entry["hash"].is_string(), "{entry}");
    Ok(())
}

/// `data.json` would read a package with no working copy back as one at the
/// home with every file gone, so no write of it holds one.
#[test(tokio::test)]
async fn data_json_refuses_a_package_with_no_working_copy() -> Res {
    use crate::LineageError;
    use crate::lineage::DomainLineage;
    use crate::lineage::DomainLineageIo;
    use crate::lineage::PackageLineage;
    use crate::lineage::WorkingCopyLocation;

    let temp_dir = tempfile::TempDir::new()?;
    let path = temp_dir.path().join("data.json");
    let mut lineage = DomainLineage::new(temp_dir.path());
    lineage.packages.insert(
        ("demo", "none").into(),
        PackageLineage {
            working_copy: WorkingCopyLocation::None,
            ..PackageLineage::default()
        },
    );
    let written = DomainLineageIo::data_json(path.clone())
        .write(&crate::io::storage::LocalStorage::new(), lineage)
        .await;
    assert!(
        matches!(
            written,
            Err(crate::Error::Lineage(
                LineageError::UnsupportedCopies { .. }
            ))
        ),
        "{written:?}"
    );
    assert!(!path.exists());
    Ok(())
}
