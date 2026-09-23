use tempfile::TempDir;
use test_log::test;

use super::*;

use super::sync_flow_tests::DeniedRemote;
use super::sync_flow_tests::LoggedOutRemote;
use crate::io::remote::mocks::MockRemote;
use crate::lineage::DomainLineageIo;
use crate::lineage::Home;
use crate::lineage::PackageLineageIo;
use crate::local_domain::LocalDomain;
use crate::paths::DomainPaths;

/// The trigger's count reads the directory and parses nothing: a damaged
/// historical manifest is still a revision this copy holds, and must not
/// fail the page read that carries the count.
#[test(tokio::test)]
async fn the_count_is_the_manifest_files_and_parses_none_of_them() -> Res {
    let temp_dir = TempDir::new()?;
    let domain = LocalDomain::new(temp_dir.path());
    domain.set_home(temp_dir.path()).await?;

    let namespace: Namespace = ("demo", "sales").into();
    let package = domain
        .create_package(namespace.clone(), None, Some("initial import".to_string()))
        .await?;

    package
        .storage
        .write_byte_stream(
            package
                .paths
                .installed_manifest(&namespace, "damaged-old-revision"),
            b"not a manifest".to_vec().into(),
        )
        .await?;
    // A directory beside the manifests is not a revision.
    package
        .storage
        .create_dir_all(
            package
                .paths
                .installed_manifests_dir(&namespace)
                .join("not-a-revision"),
        )
        .await?;

    assert_eq!(package.revision_count().await?, 2);
    Ok(())
}

/// A write's temp file (`LocalStorage` writes `.tmp-<uuid>` beside its
/// target, and a crash can strand one) is not a revision: counting it would
/// make N disagree with the list, and parsing it would refuse every opening.
#[test(tokio::test)]
async fn a_stranded_temp_file_is_not_a_revision() -> Res {
    let temp_dir = TempDir::new()?;
    let domain = LocalDomain::new(temp_dir.path());
    domain.set_home(temp_dir.path()).await?;

    let namespace: Namespace = ("demo", "sales").into();
    let package = domain
        .create_package(namespace.clone(), None, Some("initial import".to_string()))
        .await?;

    let manifests = package.paths.installed_manifests_dir(&namespace);
    for stray in [".tmp-3f2a9c1e-0000-4000-8000-000000000000", ".DS_Store"] {
        package
            .storage
            .write_byte_stream(manifests.join(stray), b"half a manifest".to_vec().into())
            .await?;
    }

    assert_eq!(package.revision_count().await?, 1);
    assert_eq!(package.revisions().await?.len(), 1);
    Ok(())
}

const REMOTE: &str = r#"{
    "bucket": "bucket",
    "namespace": "test/history",
    "hash": "published-rev",
    "catalog": "test.quilt.dev"
}"#;

/// An installed `test/history` whose lineage has `remote_json` as its remote,
/// over `remote`. The temp dirs must outlive the package.
async fn package_over<R: Remote>(
    remote: R,
    remote_json: &str,
) -> Res<(InstalledPackage<LocalStorage, R>, [TempDir; 2])> {
    let (home, home_dir) = Home::from_temp_dir()?;
    let (paths, paths_dir) = DomainPaths::from_temp_dir()?;
    let storage = LocalStorage::new();
    let namespace: Namespace = ("test", "history").into();

    paths
        .scaffold_for_installing(&storage, &home, &namespace)
        .await?;
    let lineage_json = format!(
        r#"{{
            "packages": {{
                "test/history": {{
                    "commit": null,
                    "remote": {remote_json},
                    "base_hash": "published-rev",
                    "latest_hash": "published-rev",
                    "paths": {{}}
                }}}},
            "home": "/tmp/working_dir"
            }}"#
    );
    storage
        .write_byte_stream(&paths.lineage(), lineage_json.as_bytes().to_vec().into())
        .await?;

    let package = InstalledPackage {
        lineage: PackageLineageIo::new(DomainLineageIo::new(paths.lineage()), namespace.clone()),
        paths,
        remote: Arc::new(remote),
        storage,
        namespace,
    };
    Ok((package, [home_dir, paths_dir]))
}

async fn install_manifest<R: Remote>(
    package: &InstalledPackage<LocalStorage, R>,
    hash: &str,
    manifest: &str,
) -> Res {
    package
        .storage
        .write_byte_stream(
            package.paths.installed_manifest(&package.namespace, hash),
            manifest.as_bytes().to_vec().into(),
        )
        .await
}

fn messages_published(entries: &[flow::HistoryEntry]) -> Vec<(Option<&str>, bool)> {
    entries
        .iter()
        .map(|entry| (entry.revision.message.as_deref(), entry.published))
        .collect()
}

#[test(tokio::test)]
async fn only_the_manifests_the_remote_holds_are_published() -> Res {
    let remote = MockRemote::default();
    remote
        .put_object(
            None,
            &"s3://bucket/.quilt/packages/published-rev".parse()?,
            b"{}".to_vec(),
        )
        .await?;
    let (package, _dirs) = package_over(remote, REMOTE).await?;
    install_manifest(
        &package,
        "published-rev",
        r#"{"version":"v0","message":"Sent"}"#,
    )
    .await?;
    install_manifest(
        &package,
        "local-rev",
        r#"{"version":"v0","message":"Kept here"}"#,
    )
    .await?;

    let lineage = package.lineage().await?;
    let history = package.revision_history(&lineage).await?;

    // By message, not index: two writes in one test can share an mtime.
    let mut marks = messages_published(&history);
    marks.sort_unstable();
    assert_eq!(
        marks,
        vec![(Some("Kept here"), false), (Some("Sent"), true)]
    );
    Ok(())
}

/// `LoggedOutRemote` fails every call, so any existence check would fail
/// the history.
#[test(tokio::test)]
async fn a_local_only_package_asks_the_remote_nothing() -> Res {
    let (package, _dirs) = package_over(LoggedOutRemote, "null").await?;
    install_manifest(
        &package,
        "local-rev",
        r#"{"version":"v0","message":"Kept here"}"#,
    )
    .await?;

    let lineage = package.lineage().await?;
    let history = package.revision_history(&lineage).await?;

    assert_eq!(
        messages_published(&history),
        vec![(Some("Kept here"), false)]
    );
    Ok(())
}

/// The refusal the popover draws: one refused check refuses the whole list.
#[test(tokio::test)]
async fn a_refused_existence_check_fails_the_history() -> Res {
    let (package, _dirs) = package_over(DeniedRemote, REMOTE).await?;
    install_manifest(
        &package,
        "published-rev",
        r#"{"version":"v0","message":"Sent"}"#,
    )
    .await?;

    let lineage = package.lineage().await?;
    let err = package
        .revision_history(&lineage)
        .await
        .expect_err("a denied existence check refuses the history");

    assert!(err.is_access_denied(), "{err:?}");
    Ok(())
}
