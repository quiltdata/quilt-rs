use super::*;

use std::collections::BTreeMap;
use std::path::PathBuf;

use tempfile::TempDir;

use crate::Result;
use crate::quilt;

pub fn create() -> MockQuiltModel {
    MockQuiltModel::new()
}

pub fn create_remote_manifest() -> quilt::manifest::Manifest {
    quilt::manifest::Manifest {
        header: quilt::manifest::ManifestHeader {
            version: "v0".to_string(),
            message: None,
            user_meta: None,
            workflow: None,
        },
        rows: Vec::new(),
    }
}

pub fn mock_installed_package(model: &mut MockQuiltModel) -> &MockQuiltModel {
    let remote_manifest = quilt_uri::ManifestUri {
        bucket: "quilt-example".to_string(),
        namespace: ("foo", "bar").into(),
        hash: "6c3758a4d2bf8fe730be5d12f5e095950dc123c373f55f66ca4b3ced74772b22".to_string(),
        origin: Some("test.quilt.dev".parse().unwrap()),
    };
    model.expect_get_installed_package().returning(move |_| {
        Ok(Some(
            quilt::LocalDomain::new(PathBuf::new())
                .create_installed_package(("foo", "bar").into())
                .expect("Failed to create installed package"),
        ))
    });
    model
        .expect_get_installed_package_lineage()
        .returning(move |_| {
            Ok(quilt::lineage::PackageLineage::from_remote(
                remote_manifest.clone(),
                remote_manifest.hash.clone(),
            ))
        });
    let status = Ok(quilt::lineage::InstalledPackageStatus::default());
    model
        .expect_get_installed_package_status()
        .return_once(move |_, _| status);
    model.expect_get_installed_package_records().returning(|_| {
        Ok(BTreeMap::from([(
            PathBuf::from("NAME"),
            quilt::manifest::ManifestRow::default(),
        )]))
    });
    model
        .expect_browse_remote_manifest()
        .returning(|_| Ok(create_remote_manifest()));
    model.expect_get_workflows_config().returning(|_| Ok(None));
    model
}

/// Mock for the case where the package is already installed with a different hash.
pub fn mock_remote_package_different_version(model: &mut MockQuiltModel) -> &MockQuiltModel {
    model
        .expect_resolve_manifest_uri()
        .returning(|uri| Ok(quilt_uri::ManifestUri::try_from(uri.clone()).unwrap()));
    model
        .expect_is_package_installed()
        .returning(|_| Ok(InstallCheck::DifferentVersion("aaaa1111".to_string())));

    // These are needed for ViewInstalledPackage::create after the error is caught
    model.expect_get_installed_package().returning(|_| {
        Ok(Some(
            quilt::LocalDomain::new(PathBuf::new())
                .create_installed_package(("foo", "bar").into())
                .expect("Failed to create installed package"),
        ))
    });

    let remote_manifest = quilt_uri::ManifestUri {
        bucket: "quilt-example".to_string(),
        namespace: ("foo", "bar").into(),
        hash: "aaaa1111".to_string(),
        origin: None,
    };

    model
        .expect_get_installed_package_lineage()
        .returning(move |_| {
            Ok(quilt::lineage::PackageLineage::from_remote(
                remote_manifest.clone(),
                remote_manifest.hash.clone(),
            ))
        });
    let status = Ok(quilt::lineage::InstalledPackageStatus::default());
    model
        .expect_get_installed_package_status()
        .return_once(move |_, _| status);
    model.expect_get_installed_package_records().returning(|_| {
        Ok(BTreeMap::from([(
            PathBuf::from("NAME"),
            quilt::manifest::ManifestRow::default(),
        )]))
    });

    model
}

pub fn mock_remote_package_local_only(model: &mut MockQuiltModel) -> &MockQuiltModel {
    model
        .expect_resolve_manifest_uri()
        .returning(|uri| Ok(quilt_uri::ManifestUri::try_from(uri.clone()).unwrap()));
    model
        .expect_is_package_installed()
        .returning(|_| Ok(InstallCheck::LocalOnly));
    model.expect_get_installed_package().returning(|_| {
        Ok(Some(
            quilt::LocalDomain::new(PathBuf::new())
                .create_installed_package(("foo", "bar").into())
                .expect("Failed to create installed package"),
        ))
    });

    model
        .expect_get_installed_package_lineage()
        .returning(move |_| Ok(quilt::lineage::PackageLineage::default()));
    let status = Ok(quilt::lineage::InstalledPackageStatus::default());
    model
        .expect_get_installed_package_status()
        .return_once(move |_, _| status);
    model.expect_get_installed_package_records().returning(|_| {
        Ok(BTreeMap::from([(
            PathBuf::from("NAME"),
            quilt::manifest::ManifestRow::default(),
        )]))
    });

    model
}

pub fn mock_installed_packages_list(model: &mut MockQuiltModel) -> &MockQuiltModel {
    model
        .expect_get_installed_packages_list()
        .returning(|| Ok(Vec::new()));
    model
}

#[tokio::test]
async fn test_install_package_only_with_timestamp_tag() -> Result {
    crate::env::init();

    let temp_dir = TempDir::new()?;
    let model = super::Model::create(temp_dir.path());
    model.set_home(temp_dir.path()).await?;

    // Use timestamp tag instead of "latest" for stable testing
    // Timestamp 1740761585 represents a specific tagged revision
    let uri = quilt_uri::S3PackageUri::try_from(
        "quilt+s3://data-yaml-spec-tests#package=reference/quilt-rs:1740761585",
    )?;

    assert_eq!(
        install_package_only(&model, &uri).await?,
        InstallOutcome::Installed,
    );

    let namespace: quilt_uri::Namespace = ("reference", "quilt-rs").into();
    let installed_package = model
        .get_installed_package(&namespace)
        .await?
        .ok_or_else(|| Error::from(quilt::InstallPackageError::NotInstalled(namespace)))?;
    let lineage = model
        .get_installed_package_lineage(&installed_package)
        .await?;
    assert_eq!(
        lineage.remote()?.hash,
        "a4aed21f807f0474d2761ed924a5875cc10fd0cd84617ef8f7307e4b9daebcc7"
    );

    Ok(())
}

#[tokio::test]
async fn test_install_package_only_with_hash() -> Result {
    crate::env::init();

    let temp_dir = TempDir::new()?;
    let model = super::Model::create(temp_dir.path());
    model.set_home(temp_dir.path()).await?;

    let uri = quilt_uri::S3PackageUri::try_from(
        "quilt+s3://data-yaml-spec-tests#package=reference/quilt-rs@a4aed21f807f0474d2761ed924a5875cc10fd0cd84617ef8f7307e4b9daebcc7",
    )?;

    assert_eq!(
        install_package_only(&model, &uri).await?,
        InstallOutcome::Installed,
    );

    let namespace: quilt_uri::Namespace = ("reference", "quilt-rs").into();
    let installed_package = model
        .get_installed_package(&namespace)
        .await?
        .ok_or_else(|| Error::from(quilt::InstallPackageError::NotInstalled(namespace)))?;

    let first_hash = model
        .get_installed_package_lineage(&installed_package)
        .await?
        .remote()?
        .hash
        .clone();

    // TODO: make sure there was no double installation
    assert_eq!(
        install_package_only(&model, &uri).await?,
        InstallOutcome::Installed,
    );

    let second_hash = model
        .get_installed_package_lineage(&installed_package)
        .await?
        .remote()?
        .hash
        .clone();

    assert_eq!(first_hash, second_hash);

    Ok(())
}

#[tokio::test]
async fn test_install_package_only_resolution_failure() -> Result {
    crate::env::init();

    let temp_dir = TempDir::new()?;
    let model = super::Model::create(temp_dir.path());
    // Set up home directory (required for Model to work properly)
    model.set_home(temp_dir.path()).await?;

    let uri = quilt_uri::S3PackageUri::try_from(
        "quilt+s3://nonexisting-bucket#package=two/files:latest",
    )?;

    let result = install_package_only(&model, &uri).await;
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("nonexisting-bucket") && msg.contains("not reachable"),
        "error should name the bucket and say it's unreachable, got: {msg}"
    );

    Ok(())
}

#[tokio::test]
async fn test_install_package_only_local_only() -> Result {
    let mut model = create();
    mock_remote_package_local_only(&mut model);

    let uri =
        quilt_uri::S3PackageUri::try_from("quilt+s3://quilt-example#package=foo/bar@some_hash")?;

    let result = install_package_only(&model, &uri).await?;
    assert!(
        matches!(result, InstallOutcome::LocalOnly),
        "expected LocalOnly, got {result:?}",
    );

    Ok(())
}

#[tokio::test]
async fn test_install_package_only_different_version() -> Result {
    let mut model = create();
    mock_remote_package_different_version(&mut model);

    let uri =
        quilt_uri::S3PackageUri::try_from("quilt+s3://quilt-example#package=foo/bar@bbbb2222")?;

    let result = install_package_only(&model, &uri).await?;
    match result {
        InstallOutcome::DifferentVersion {
            requested_hash,
            installed_hash,
        } => {
            assert_eq!(requested_hash, "bbbb2222");
            assert_eq!(installed_hash, "aaaa1111");
        }
        other => panic!("expected DifferentVersion, got {other:?}"),
    }

    Ok(())
}
