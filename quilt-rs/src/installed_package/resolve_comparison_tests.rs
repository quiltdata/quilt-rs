use tempfile::TempDir;
use test_log::test;

use super::*;

use super::sync_flow_tests::DeniedRemote;
use crate::error::LineageError;
use crate::io::remote::mocks::MockRemote;
use crate::lineage::DomainLineageIo;
use crate::lineage::Home;
use crate::lineage::PackageLineageIo;
use crate::paths::DomainPaths;
use crate::paths::tag_key;

const REMOTE: &str = r#"{
    "bucket": "bucket",
    "namespace": "test/resolve",
    "hash": "1111111111111111111111111111111111111111111111111111111111111111",
    "origin": "quilt.test"
}"#;

const REMOTE_WITHOUT_CATALOG: &str = r#"{
    "bucket": "bucket",
    "namespace": "test/resolve",
    "hash": "1111111111111111111111111111111111111111111111111111111111111111"
}"#;

/// The revision this copy is based on, and its remote's hash.
const BASE_REV: &str = "1111111111111111111111111111111111111111111111111111111111111111";

/// The published `latest`: a pointer holds a full top hash.
const PUB_REV: &str = "2222222222222222222222222222222222222222222222222222222222222222";
const EMPTY_HASH: &str = "47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU=";
const OTHER_HASH: &str = "ypeBEsobvcr6wjGzmiPcTaeG7/gUfE5yuYB3ha/uSLs=";

fn manifest(message: &str, rows: &[(&str, &str)]) -> String {
    let header = format!("{{\"version\":\"v0\",\"message\":\"{message}\",\"user_meta\":null}}\n");
    let rows: Vec<String> = rows
        .iter()
        .map(|(key, hash)| {
            format!(
                "{{\"logical_key\":\"{key}\",\"physical_keys\":[\"s3://bucket/{key}\"],\
                 \"hash\":{{\"type\":\"sha2-256-chunked\",\"value\":\"{hash}\"}},\
                 \"size\":0,\"meta\":null}}\n"
            )
        })
        .collect();
    header + &rows.concat()
}

/// An installed `test/resolve` with a two-revision pending chain
/// (`local-1` then `local-2`) over `remote_json`, whose on-disk
/// `latest_hash` is the stale `BASE_REV`. The temp dirs must outlive the
/// package.
async fn diverged_over<R: Remote>(
    remote: R,
    remote_json: &str,
) -> Res<(InstalledPackage<LocalStorage, R>, [TempDir; 2])> {
    let (home, home_dir) = Home::from_temp_dir()?;
    let (paths, paths_dir) = DomainPaths::from_temp_dir()?;
    let storage = LocalStorage::new();
    let namespace: Namespace = ("test", "resolve").into();

    paths
        .scaffold_for_installing(&storage, &home, &namespace)
        .await?;
    let lineage_json = format!(
        r#"{{
            "packages": {{
                "test/resolve": {{
                    "commit": {{
                        "timestamp": "2024-01-01T00:00:00Z",
                        "hash": "local-2",
                        "prev_hashes": ["local-1"]
                    }},
                    "remote": {remote_json},
                    "base_hash": "{BASE_REV}",
                    "latest_hash": "{BASE_REV}",
                    "paths": {{}}
                }}}},
            "home": "/tmp/working_dir"
            }}"#
    );
    storage
        .write_byte_stream(&paths.lineage(), lineage_json.as_bytes().to_vec().into())
        .await?;
    storage
        .write_byte_stream(
            paths.installed_manifest(&namespace, "local-2"),
            manifest("Mine", &[("same.txt", EMPTY_HASH), ("a.txt", EMPTY_HASH)])
                .into_bytes()
                .into(),
        )
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

/// A remote whose `latest` has moved to `PUB_REV` past the lineage's
/// `BASE_REV`, and whose registry lists `BASE_REV`, `PUB_REV` and `local-1`.
async fn published_remote() -> Res<MockRemote> {
    let remote = MockRemote::default();
    let namespace: Namespace = ("test", "resolve").into();
    remote
        .put_object(
            None,
            &format!("s3://bucket/{}", tag_key(&namespace, "latest")).parse()?,
            PUB_REV.as_bytes().to_vec(),
        )
        .await?;
    remote
        .put_object(
            None,
            &format!("s3://bucket/.quilt/packages/{PUB_REV}").parse()?,
            manifest("Sent", &[("same.txt", EMPTY_HASH), ("a.txt", OTHER_HASH)]).into_bytes(),
        )
        .await?;
    // Were the stale `latest_hash` read, the answer would be this one's.
    remote
        .put_object(
            None,
            &format!("s3://bucket/.quilt/packages/{BASE_REV}").parse()?,
            manifest("Stale", &[("same.txt", EMPTY_HASH), ("a.txt", EMPTY_HASH)]).into_bytes(),
        )
        .await?;
    for (timestamp, hash) in [
        ("1758500000", BASE_REV),
        ("1758500001", PUB_REV),
        ("1758500002", "local-1"),
    ] {
        remote
            .put_object(
                None,
                &format!("s3://bucket/{}", tag_key(&namespace, timestamp)).parse()?,
                hash.as_bytes().to_vec(),
            )
            .await?;
    }
    Ok(remote)
}

#[test(tokio::test)]
async fn the_comparison_reads_the_published_latest_and_the_registry() -> Res {
    let (package, _dirs) = diverged_over(published_remote().await?, REMOTE).await?;

    let lineage = package.lineage().await?;
    let comparison = package.resolve_comparison(&lineage).await?;

    assert_eq!(comparison.published_message.as_deref(), Some("Sent"));
    assert_eq!(comparison.differing, vec![PathBuf::from("a.txt")]);
    assert_eq!(comparison.unpublished, 1);
    Ok(())
}

/// With no catalog the bucket's pointers list `local-1`, as the registry
/// would, so it is subtracted.
#[test(tokio::test)]
async fn without_a_catalog_host_the_bucket_is_listed() -> Res {
    let (package, _dirs) = diverged_over(published_remote().await?, REMOTE_WITHOUT_CATALOG).await?;

    let lineage = package.lineage().await?;
    let comparison = package.resolve_comparison(&lineage).await?;

    assert_eq!(comparison.published_message.as_deref(), Some("Sent"));
    assert_eq!(comparison.unpublished, 1);
    Ok(())
}

#[test(tokio::test)]
async fn a_local_only_package_has_nothing_to_compare() -> Res {
    let (package, _dirs) = diverged_over(published_remote().await?, "null").await?;

    let lineage = package.lineage().await?;
    let result = package.resolve_comparison(&lineage).await;

    assert!(
        matches!(result, Err(Error::Lineage(LineageError::NoRemote))),
        "{result:?}"
    );
    Ok(())
}

/// A refusal is not swallowed into an empty answer: the backend draws any
/// `Err` as the refused comparison.
#[test(tokio::test)]
async fn a_refusing_remote_refuses_the_comparison() -> Res {
    let (package, _dirs) = diverged_over(DeniedRemote, REMOTE).await?;

    let lineage = package.lineage().await?;
    let err = package
        .resolve_comparison(&lineage)
        .await
        .expect_err("a denied remote refuses the comparison");

    assert!(err.is_access_denied(), "{err:?}");
    Ok(())
}
