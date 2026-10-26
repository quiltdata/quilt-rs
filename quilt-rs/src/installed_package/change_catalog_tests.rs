//! Tests for replacing the catalog of a package's remote via `change_catalog`.

use super::*;
use quilt_uri::fixtures;

use test_log::test;

use aws_sdk_s3::primitives::ByteStream;

use crate::io::remote::WorkflowIntent;
use crate::io::remote::mocks::MockRemote;
use crate::lineage::DomainLineageIo;
use crate::lineage::Home;
use crate::lineage::PackageLineageIo;
use crate::lineage::UpstreamState;
use crate::paths::DomainPaths;

const HASH: &str = "abcdef";

/// A package published to `bkt` at `HASH`, which is also the remote's
/// `latest`, holding a pending commit on top. `catalog` is the remote's
/// catalog, if any.
async fn published(
    catalog: Option<&str>,
) -> Res<(
    InstalledPackage<LocalStorage, MockRemote>,
    tempfile::TempDir,
    tempfile::TempDir,
)> {
    let (home, home_dir) = Home::from_temp_dir()?;
    let (paths, paths_dir) = DomainPaths::from_temp_dir()?;
    let storage = LocalStorage::new();
    let remote = MockRemote::default();
    let namespace: Namespace = ("test", "survey").into();

    paths
        .scaffold_for_installing(&storage, &home, &namespace)
        .await?;
    let catalog = catalog.map_or(String::new(), |c| format!(r#", "catalog": "{c}""#));
    let lineage_json = format!(
        r#"{{
            "packages": {{
                "test/survey": {{
                    "commit": {{
                        "timestamp": "2024-01-01T00:00:00Z",
                        "hash": "{HASH}",
                        "prev_hashes": ["older"]
                    }},
                    "remote": {{
                        "bucket": "bkt",
                        "namespace": "test/survey",
                        "hash": "{HASH}"{catalog}
                    }},
                    "base_hash": "{HASH}",
                    "latest_hash": "{HASH}",
                    "paths": {{}}
                }}
            }},
            "home": "{}"
        }}"#,
        home.as_ref().display(),
    );
    storage
        .write_byte_stream(&paths.lineage(), lineage_json.as_bytes().to_vec().into())
        .await?;
    storage
        .write_byte_stream(
            paths.installed_manifest(&namespace, HASH),
            ByteStream::from_static(br#"{"version": "v0"}"#),
        )
        .await?;
    remote
        .put_object(
            None,
            &S3Uri::try_from("s3://bkt/.quilt/named_packages/test/survey/latest")?,
            HASH.as_bytes().to_vec(),
        )
        .await?;

    let package = InstalledPackage {
        lineage: PackageLineageIo::new(DomainLineageIo::new(paths.lineage()), namespace.clone()),
        paths,
        remote: std::sync::Arc::new(remote),
        storage,
        namespace,
        state_store: None,
    };
    Ok((package, home_dir, paths_dir))
}

/// Everything but the catalog, as the lineage holds it.
async fn kept(
    package: &InstalledPackage<LocalStorage, MockRemote>,
) -> Res<crate::lineage::PackageLineage> {
    let mut lineage = package.lineage().await?;
    if let Some(remote) = lineage.remote_uri.as_mut() {
        remote.origin = None;
    }
    Ok(lineage)
}

/// Adding a catalog to a package published without one keeps the bucket, the
/// remote hash and the commits, so the package is still at the remote's
/// latest.
#[test(tokio::test)]
async fn adding_a_catalog_keeps_the_published_package() -> Res {
    let (package, _home, _paths) = published(None).await?;
    let before = kept(&package).await?;

    let remote = package.change_catalog(Some(fixtures::host())).await?;

    assert_eq!(remote.origin, Some(fixtures::host()));
    assert_eq!(remote.bucket, "bkt");
    assert_eq!(remote.hash, HASH);
    assert_eq!(kept(&package).await?, before, "only the catalog changed");
    assert_eq!(
        package.lineage().await?.remote()?.origin,
        Some(fixtures::host()),
        "saved"
    );
    let status = package.status(None).await?;
    assert_eq!(status.upstream_state, UpstreamState::UpToDate);
    Ok(())
}

/// Removing the catalog is the same narrow change the other way.
#[test(tokio::test)]
async fn removing_the_catalog_keeps_the_published_package() -> Res {
    let (package, _home, _paths) = published(Some("quilt.test")).await?;
    let before = kept(&package).await?;

    let remote = package.change_catalog(None).await?;

    assert_eq!(remote.origin, None);
    assert_eq!(kept(&package).await?, before, "only the catalog changed");
    let status = package.status(None).await?;
    assert_eq!(status.upstream_state, UpstreamState::UpToDate);
    Ok(())
}

/// One catalog for another.
#[test(tokio::test)]
async fn changing_the_catalog_keeps_the_published_package() -> Res {
    let (package, _home, _paths) = published(Some("quilt.test")).await?;
    let before = kept(&package).await?;

    let remote = package
        .change_catalog(Some(fixtures::another_host()))
        .await?;

    assert_eq!(remote.origin, Some(fixtures::another_host()));
    assert_eq!(kept(&package).await?, before, "only the catalog changed");
    Ok(())
}

/// A local-only package has no remote to change the catalog of: set remote
/// chooses its bucket first.
#[test(tokio::test)]
async fn a_package_without_a_remote_is_refused() -> Res {
    let (home, _home_dir) = Home::from_temp_dir()?;
    let (paths, _paths_dir) = DomainPaths::from_temp_dir()?;
    let storage = LocalStorage::new();
    let namespace: Namespace = ("test", "local").into();
    paths
        .scaffold_for_installing(&storage, &home, &namespace)
        .await?;
    let lineage_json = r#"{
        "packages": {
            "test/local": {
                "commit": null,
                "remote": null,
                "base_hash": "",
                "latest_hash": "",
                "paths": {}
            }
        },
        "home": "/tmp/working_dir"
    }"#;
    storage
        .write_byte_stream(&paths.lineage(), lineage_json.as_bytes().to_vec().into())
        .await?;
    let package = InstalledPackage {
        lineage: PackageLineageIo::new(DomainLineageIo::new(paths.lineage()), namespace.clone()),
        paths,
        remote: std::sync::Arc::new(MockRemote::default()),
        storage,
        namespace,
        state_store: None,
    };

    let result = package.change_catalog(Some(fixtures::host())).await;

    assert!(result.is_err(), "no remote to change");
    assert!(
        package.lineage().await?.remote_uri.is_none(),
        "nothing saved"
    );
    Ok(())
}

/// Set remote is not loosened: on a published package it still refuses a
/// new bucket, even with the same catalog.
#[test(tokio::test)]
async fn set_remote_still_refuses_a_new_bucket_once_published() -> Res {
    let (package, _home, _paths) = published(Some("quilt.test")).await?;

    let result = package
        .set_remote(
            "other-bucket".to_string(),
            Some(fixtures::host()),
            WorkflowIntent::BucketDefault,
        )
        .await;

    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Cannot change remote"),
    );
    assert_eq!(package.lineage().await?.remote()?.bucket, "bkt");
    Ok(())
}
