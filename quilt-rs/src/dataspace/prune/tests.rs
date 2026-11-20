use super::*;

use std::collections::BTreeMap;

use bytes::Bytes;
use test_log::test;

use crate::LogicalKey;
use crate::dataspace::store::tests::local_bytes;
use crate::dataspace::store::tests::publish_local;
use crate::flow::PackageStore;
use crate::flow::WorkflowCheck;
use crate::flow::record;
use crate::io::manifest::RowContent;
use crate::io::remote::mocks::MockRemote;
use crate::io::storage::mocks::MockStorage;
use crate::lineage::PackageLineage;
use crate::manifest::Manifest;
use crate::manifest::ManifestRow;
use quilt_uri::S3Uri;
use quilt_uri::paths::get_manifest_key;

type Res<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn key(registry: Registry) -> PackageKey {
    PackageKey {
        registry,
        package: Namespace::from(("team", "raw")),
    }
}

/// Commits `body` as `a.txt` of `team/raw` into `domain`'s store for `b`;
/// returns its top hash.
async fn commit(storage: &MockStorage, domain: &DomainRoot, body: &'static [u8]) -> Res<String> {
    let logical_key = LogicalKey::new("a.txt")?;
    let manifest = Manifest {
        rows: vec![ManifestRow {
            logical_key: logical_key.clone(),
            ..ManifestRow::default()
        }],
        ..Manifest::default()
    };
    let files = BTreeMap::from([(logical_key, RowContent::Bytes(Bytes::from_static(body)))]);
    let (_, recorded) = record(
        &domain.registry("b")?,
        storage,
        &MockRemote::default(),
        WorkflowCheck::Warn { host: None },
        &Namespace::from(("team", "raw")),
        PackageLineage::default(),
        manifest,
        &files,
    )
    .await?;
    Ok(recorded.hash().expect("a revision").to_string())
}

/// Puts the manifest `hash` in bucket `b`, so it is published.
async fn publish(remote: &MockRemote, hash: &str) -> Res {
    let uri = S3Uri {
        bucket: "b".to_string(),
        key: get_manifest_key(hash),
        version: None,
    };
    remote.put_object(None, &uri, b"{}".to_vec()).await?;
    Ok(())
}

/// Published content goes; the unpushed commit stays, with its object.
#[test(tokio::test)]
async fn prune_clears_published_content_and_keeps_unpublished_work() -> Res {
    let (storage, remote) = (MockStorage::default(), MockRemote::default());
    let domain = DomainRoot::new(storage.temp_dir.path().join("domain"));
    let pushed = commit(&storage, &domain, b"pushed").await?;
    let unpushed = commit(&storage, &domain, b"local").await?;
    publish(&remote, &pushed).await?;
    let store = domain.registry("b")?;
    let package = key(Registry::S3("b".to_string()));
    let unpushed_rows = Manifest::from_path(
        &storage,
        &store.installed_manifest(&package.package, &unpushed),
    )
    .await?
    .rows;

    let pruned = prune(&storage, &remote, &domain, &package, None).await?;

    assert!(
        matches!(&pruned, Pruned::Freed(report) if report.cached_manifests == 1),
        "{pruned:?}"
    );
    assert!(!store.installed_manifest(&package.package, &pushed).exists());
    assert!(
        store
            .installed_manifest(&package.package, &unpushed)
            .exists()
    );
    assert!(store.object_path(&unpushed_rows[0].hash).exists());
    Ok(())
}

/// Offline, the bucket cannot be asked, so nothing is cleared.
#[test(tokio::test)]
async fn prune_clears_nothing_when_the_bucket_cannot_answer() -> Res {
    let (storage, remote) = (MockStorage::default(), MockRemote::default());
    let domain = DomainRoot::new(storage.temp_dir.path().join("domain"));
    let pushed = commit(&storage, &domain, b"pushed").await?;
    publish(&remote, &pushed).await?;
    remote.deny_reads(&format!("s3://b/{}", get_manifest_key(&pushed)));
    let package = key(Registry::S3("b".to_string()));

    let pruned = prune(&storage, &remote, &domain, &package, None).await;

    assert!(pruned.is_err(), "{pruned:?}");
    let store = domain.registry("b")?;
    assert!(store.installed_manifest(&package.package, &pushed).exists());
    Ok(())
}

/// The local registry holds the only copy, and is never cleared.
#[test(tokio::test)]
async fn prune_never_clears_the_local_registry() -> Res {
    let (storage, remote) = (MockStorage::default(), MockRemote::default());
    let domain = DomainRoot::new(storage.temp_dir.path().join("domain"));
    let local = domain.local();
    local.rename(&storage, "lab-mac").await?;
    let hash = publish_local(&storage, &local, "team/raw", &[("a.txt", b"alpha")]).await?;

    let pruned = prune(
        &storage,
        &remote,
        &domain,
        &key(Registry::Local("lab-mac".to_string())),
        None,
    )
    .await?;

    assert_eq!(pruned, Pruned::LocalRegistry);
    assert_eq!(
        local_bytes(&storage, &local, &hash, "a.txt").await?,
        b"alpha"
    );
    Ok(())
}
