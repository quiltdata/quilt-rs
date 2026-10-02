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
use crate::paths::tag_key;

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
    "origin": "test.quilt.dev"
}"#;

/// `ManifestUri` spells the catalog host `origin`; leaving it out is the
/// bucket-only remote a no-catalog push records.
const REMOTE_WITHOUT_CATALOG: &str = r#"{
    "bucket": "bucket",
    "namespace": "test/history",
    "hash": "published-rev"
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

/// Published is the registry listing the revision, not its manifest object
/// existing: `orphan-rev`'s manifest is in the bucket with no pointer to it.
#[test(tokio::test)]
async fn only_the_revisions_the_registry_lists_are_published() -> Res {
    let remote = MockRemote::default();
    let namespace: Namespace = ("test", "history").into();
    remote
        .put_object(
            None,
            &format!("s3://bucket/{}", tag_key(&namespace, "1758500000")).parse()?,
            b"published-rev".to_vec(),
        )
        .await?;
    remote
        .put_object(
            None,
            &"s3://bucket/.quilt/packages/orphan-rev".parse()?,
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
        "orphan-rev",
        r#"{"version":"v0","message":"Uploaded, never tagged"}"#,
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
        vec![
            (Some("Kept here"), false),
            (Some("Sent"), true),
            (Some("Uploaded, never tagged"), false),
        ]
    );
    Ok(())
}

/// `LoggedOutRemote` fails every call, so asking the registry would fail
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

/// A remote with no catalog host has no registry to ask, so nothing is
/// published and nothing is asked — `LoggedOutRemote` would fail the ask.
#[test(tokio::test)]
async fn a_remote_without_a_catalog_host_asks_nothing() -> Res {
    let (package, _dirs) = package_over(LoggedOutRemote, REMOTE_WITHOUT_CATALOG).await?;
    install_manifest(
        &package,
        "published-rev",
        r#"{"version":"v0","message":"Sent"}"#,
    )
    .await?;

    let lineage = package.lineage().await?;
    assert_eq!(
        lineage.remote_uri.as_ref().map(|uri| uri.origin.is_none()),
        Some(true)
    );
    let history = package.revision_history(&lineage).await?;

    assert_eq!(messages_published(&history), vec![(Some("Sent"), false)]);
    Ok(())
}

/// The refusal the popover draws: a refused listing refuses the whole list.
#[test(tokio::test)]
async fn a_refused_listing_fails_the_history() -> Res {
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
        .expect_err("a denied listing refuses the history");

    assert!(err.is_access_denied(), "{err:?}");
    Ok(())
}

/// `published-rev` is current, base and latest; `old-rev` is published
/// and old; `local-rev` was never published.
async fn package_with_old_revisions() -> Res<(InstalledPackage<LocalStorage, MockRemote>, [TempDir; 2])>
{
    let remote = MockRemote::default();
    let namespace: Namespace = ("test", "history").into();
    for (tag, hash) in [("1758500000", "published-rev"), ("1758400000", "old-rev")] {
        remote
            .put_object(
                None,
                &format!("s3://bucket/{}", tag_key(&namespace, tag)).parse()?,
                hash.as_bytes().to_vec(),
            )
            .await?;
    }
    let (package, dirs) = package_over(remote, REMOTE).await?;
    for hash in ["published-rev", "old-rev", "local-rev"] {
        install_manifest(&package, hash, r#"{"version":"v0"}"#).await?;
    }
    Ok((package, dirs))
}

fn hashes(hashes: &[&str]) -> std::collections::BTreeSet<String> {
    hashes.iter().map(|h| (*h).to_string()).collect()
}

async fn held<R: Remote>(package: &InstalledPackage<LocalStorage, R>) -> Res<Vec<String>> {
    let mut held: Vec<String> = package
        .revisions()
        .await?
        .into_iter()
        .map(|revision| revision.hash)
        .collect();
    held.sort();
    Ok(held)
}

#[test(tokio::test)]
async fn an_old_published_revision_is_removed() -> Res {
    let (package, _dirs) = package_with_old_revisions().await?;

    let report = package.remove_revisions(&hashes(&["old-rev"])).await?;

    assert_eq!(report.revisions, 1);
    assert_eq!(held(&package).await?, vec!["local-rev", "published-rev"]);
    Ok(())
}

/// A protected or absent hash refuses the whole request: `old-rev`, asked
/// for beside it, stays too.
#[test(tokio::test)]
async fn a_kept_or_absent_revision_refuses_the_whole_removal() -> Res {
    let (package, _dirs) = package_with_old_revisions().await?;

    for (refused, why) in [
        ("published-rev", "it is kept (current \u{b7} latest \u{b7} base)"),
        ("local-rev", "it is kept (unpublished)"),
        ("gone-rev", "this copy does not hold it"),
    ] {
        let err = package
            .remove_revisions(&hashes(&["old-rev", refused]))
            .await
            .expect_err("refused");
        assert!(
            matches!(&err, Error::RevisionNotRemovable { hash, why: said, .. }
                if hash == refused && said == why),
            "{err:?}"
        );
    }
    assert_eq!(
        held(&package).await?,
        vec!["local-rev", "old-rev", "published-rev"]
    );
    Ok(())
}

/// A sync holding the package's lock refuses it before anything is read.
#[test(tokio::test)]
async fn a_busy_package_refuses_the_removal() -> Res {
    let (package, _dirs) = package_with_old_revisions().await?;
    let _syncing = package.lock().await?;

    let err = package
        .remove_revisions(&hashes(&["old-rev"]))
        .await
        .expect_err("busy");

    assert!(matches!(&err, Error::PackageBusy(ns) if *ns == package.namespace), "{err:?}");
    assert_eq!(held(&package).await?.len(), 3);
    Ok(())
}

/// Without a listing, an unpublished revision cannot be told apart.
#[test(tokio::test)]
async fn a_refused_listing_refuses_the_removal() -> Res {
    let (package, _dirs) = package_over(DeniedRemote, REMOTE).await?;
    install_manifest(&package, "old-rev", r#"{"version":"v0"}"#).await?;

    assert!(package.remove_revisions(&hashes(&["old-rev"])).await.is_err());
    assert_eq!(held(&package).await?, vec!["old-rev"]);
    Ok(())
}
