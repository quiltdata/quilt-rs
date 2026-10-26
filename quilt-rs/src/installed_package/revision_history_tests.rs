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
    let (package, _) = domain
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
    let (package, _) = domain
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
    "origin": "quilt.test"
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
        state_store: None,
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

/// Two pushed revisions and one kept here, as a catalog-less push leaves
/// them: a timestamped pointer for each pushed one, and `latest`.
async fn seed_pointers(remote: &MockRemote, pointers: &[(&str, &str)]) -> Res {
    let namespace: Namespace = ("test", "history").into();
    for (tag, hash) in pointers {
        remote
            .put_object(
                None,
                &format!("s3://bucket/{}", tag_key(&namespace, tag)).parse()?,
                hash.as_bytes().to_vec(),
            )
            .await?;
    }
    Ok(())
}

/// With no catalog the bucket's pointers say what is published, as the
/// registry would. `latest` names a revision, but is not one.
#[test(tokio::test)]
async fn a_catalog_less_package_reads_its_pointers_from_the_bucket() -> Res {
    let remote = MockRemote::default();
    seed_pointers(
        &remote,
        &[
            ("1758400000", "first-rev"),
            ("1758500000", "published-rev"),
            ("latest", "published-rev"),
        ],
    )
    .await?;
    let (package, _dirs) = package_over(remote, REMOTE_WITHOUT_CATALOG).await?;
    for (hash, message) in [
        ("first-rev", "First"),
        ("published-rev", "Sent"),
        ("local-rev", "Kept here"),
    ] {
        install_manifest(
            &package,
            hash,
            &format!(r#"{{"version":"v0","message":"{message}"}}"#),
        )
        .await?;
    }

    let lineage = package.lineage().await?;
    let history = package.revision_history(&lineage).await?;
    let mut marks = messages_published(&history);
    marks.sort_unstable();

    assert_eq!(
        marks,
        vec![
            (Some("First"), true),
            (Some("Kept here"), false),
            (Some("Sent"), true),
        ]
    );
    Ok(())
}

/// The listing goes to the catalog's registry when there is a catalog, and
/// to the bucket when there is none.
#[test(tokio::test)]
async fn the_listing_asks_the_registry_only_with_a_catalog() -> Res {
    for (remote_json, asked) in [
        (REMOTE, Some("quilt.test".parse::<Host>()?)),
        (REMOTE_WITHOUT_CATALOG, None),
    ] {
        let remote = ListingRemote::default();
        let (package, _dirs) = package_over(remote, remote_json).await?;
        install_manifest(&package, "published-rev", r#"{"version":"v0"}"#).await?;

        let lineage = package.lineage().await?;
        package.revision_history(&lineage).await?;

        assert_eq!(*package.remote.asked.lock().unwrap(), vec![asked]);
    }
    Ok(())
}

/// Records the host each listing was asked for, and lists nothing.
#[derive(Default)]
struct ListingRemote {
    asked: std::sync::Mutex<Vec<Option<Host>>>,
}

#[allow(
    clippy::unused_async_trait_impl,
    reason = "all but the listing panic on call, which only `async fn` can write; see `set_remote_tests`."
)]
impl Remote for ListingRemote {
    async fn exists(&self, _host: Option<&Host>, _s3_uri: &S3Uri) -> Res<bool> {
        unreachable!("only the listing is asked")
    }
    async fn get_object_stream(
        &self,
        _host: Option<&Host>,
        _s3_uri: &S3Uri,
    ) -> Res<crate::io::remote::RemoteObjectStream> {
        unreachable!("only the listing is asked")
    }
    async fn resolve_url(&self, _host: Option<&Host>, _s3_uri: &S3Uri) -> Res<S3Uri> {
        unreachable!("only the listing is asked")
    }
    async fn put_object(
        &self,
        _host: Option<&Host>,
        _s3_uri: &S3Uri,
        _contents: impl Into<aws_sdk_s3::primitives::ByteStream>,
    ) -> Res {
        unreachable!("only the listing is asked")
    }
    async fn upload_file(
        &self,
        _host_config: &crate::io::remote::HostConfig,
        _source_path: impl AsRef<Path>,
        _dest_uri: &S3Uri,
        _size: u64,
    ) -> Res<(S3Uri, crate::object_hash::ObjectHash)> {
        unreachable!("only the listing is asked")
    }
    async fn host_config(&self, _host: Option<&Host>) -> Res<crate::io::remote::HostConfig> {
        unreachable!("only the listing is asked")
    }
    async fn verify_bucket(&self, _bucket: &str) -> Res {
        unreachable!("only the listing is asked")
    }
    async fn published_revisions(
        &self,
        host: Option<&Host>,
        _bucket: &str,
        _namespace: &Namespace,
    ) -> Res<Vec<String>> {
        self.asked.lock().unwrap().push(host.cloned());
        Ok(Vec::new())
    }
}

/// Unusable AWS credentials on this computer fail a catalog-less listing
/// the way a lost session fails the registry's.
#[test(tokio::test)]
async fn unusable_ambient_credentials_fail_a_catalog_less_history() -> Res {
    let (package, _dirs) = package_over(LoggedOutRemote, REMOTE_WITHOUT_CATALOG).await?;
    install_manifest(&package, "published-rev", r#"{"version":"v0"}"#).await?;

    let lineage = package.lineage().await?;
    let err = package
        .revision_history(&lineage)
        .await
        .expect_err("the bucket could not be listed");

    assert!(err.is_ambient_session_absent(), "{err:?}");
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
async fn package_with_old_revisions()
-> Res<(InstalledPackage<LocalStorage, MockRemote>, [TempDir; 2])> {
    package_with_old_revisions_on(REMOTE).await
}

/// [`package_with_old_revisions`] with `remote_json` as its remote.
async fn package_with_old_revisions_on(
    remote_json: &str,
) -> Res<(InstalledPackage<LocalStorage, MockRemote>, [TempDir; 2])> {
    let remote = MockRemote::default();
    seed_pointers(
        &remote,
        &[("1758500000", "published-rev"), ("1758400000", "old-rev")],
    )
    .await?;
    let (package, dirs) = package_over(remote, remote_json).await?;
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

#[test(tokio::test)]
async fn an_old_revision_of_a_catalog_less_package_is_removed() -> Res {
    let (package, _dirs) = package_with_old_revisions_on(REMOTE_WITHOUT_CATALOG).await?;

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
        (
            "published-rev",
            "it is kept (current \u{b7} latest \u{b7} base)",
        ),
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

    assert!(
        matches!(&err, Error::PackageBusy(ns) if *ns == package.namespace),
        "{err:?}"
    );
    assert_eq!(held(&package).await?.len(), 3);
    Ok(())
}

/// Without a listing, an unpublished revision cannot be told apart.
#[test(tokio::test)]
async fn a_refused_listing_refuses_the_removal() -> Res {
    let (package, _dirs) = package_over(DeniedRemote, REMOTE).await?;
    install_manifest(&package, "old-rev", r#"{"version":"v0"}"#).await?;

    assert!(
        package
            .remove_revisions(&hashes(&["old-rev"]))
            .await
            .is_err()
    );
    assert_eq!(held(&package).await?, vec!["old-rev"]);
    Ok(())
}
