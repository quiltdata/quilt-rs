use super::*;

use std::path::Path;
use std::path::PathBuf;

use bytes::Bytes;
use test_log::test;

use crate::LogicalKey;
use crate::dataspace::store::tests::publish_local;
use crate::flow::PackageStore;
use crate::flow::WorkflowCheck;
use crate::flow::record;
use crate::io::manifest::RowContent;
use crate::io::remote::mocks::MockRemote;
use crate::io::storage::LocalStorage;
use crate::lineage::PackageLineage;
use crate::manifest::Manifest;
use crate::manifest::ManifestRow;
use quilt_uri::S3Uri;
use quilt_uri::paths::get_manifest_key;

type Res = Result<(), Box<dyn std::error::Error>>;

/// Records `files` as a revision of `acme/demo` in `store`; returns its top
/// hash and its rows' object paths, in key order.
async fn commit(
    store: &impl PackageStore,
    files: &[(&str, &'static [u8])],
) -> Result<(String, Vec<PathBuf>), Box<dyn std::error::Error>> {
    let storage = LocalStorage::new();
    let namespace = Namespace::from(("acme", "demo"));
    let mut rows = Vec::new();
    let mut content = std::collections::BTreeMap::new();
    for (name, body) in files {
        let key = LogicalKey::new(*name)?;
        rows.push(ManifestRow {
            logical_key: key.clone(),
            ..ManifestRow::default()
        });
        content.insert(key, RowContent::Bytes(Bytes::from_static(body)));
    }
    let manifest = Manifest {
        rows,
        ..Manifest::default()
    };
    let (_, recorded) = record(
        store,
        &storage,
        &MockRemote::default(),
        WorkflowCheck::Warn { host: None },
        &namespace,
        PackageLineage::default(),
        manifest,
        &content,
    )
    .await?;
    let hash = recorded.hash().ok_or("no revision")?.to_string();
    let written =
        Manifest::from_path(&storage, &store.manifests_dir(&namespace).join(&hash)).await?;
    let objects = written
        .rows
        .iter()
        .map(|row| store.object_path(&row.hash))
        .collect();
    Ok((hash, objects))
}

fn manifest_uri(bucket: &str, hash: &str) -> S3Uri {
    S3Uri {
        bucket: bucket.to_string(),
        key: get_manifest_key(hash),
        version: None,
    }
}

/// Marks the manifest `hash` as held by `bucket`.
async fn publish(remote: &MockRemote, bucket: &str, hash: &str) -> Res {
    Ok(remote
        .put_object(None, &manifest_uri(bucket, hash), b"{}".to_vec())
        .await?)
}

/// Every file under `dir`, with its bytes, but the package locks, which a
/// gc takes.
fn files(dir: &Path) -> std::io::Result<Vec<(PathBuf, Vec<u8>)>> {
    let mut found = Vec::new();
    let mut dirs = vec![dir.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(dir)? {
            let path = entry?.path();
            if path.is_dir() {
                if path.file_name().is_some_and(|name| name == "locks") {
                    continue;
                }
                dirs.push(path);
            } else {
                found.push((path.clone(), std::fs::read(&path)?));
            }
        }
    }
    found.sort();
    Ok(found)
}

fn freed(collected: &Collected, bucket: &str) -> Option<GcReport> {
    match collected.stores.get(bucket) {
        Some(StoreGc::Freed(report)) => Some(*report),
        _ => None,
    }
}

/// gc clears in each bucket's store what the bucket holds, and keeps an
/// unpushed commit with every object it names; it never touches the local
/// registry, and clears nothing in a store whose bucket cannot answer, as
/// offline, while still clearing the others.
#[test(tokio::test)]
async fn gc_never_clears_unpublished_work_the_local_registry_or_anything_offline() -> Res {
    let dir = tempfile::tempdir()?;
    let domain = DomainRoot::new(dir.path());
    let storage = LocalStorage::new();
    let remote = MockRemote::default();
    let one = domain.registry("one")?;
    let (pushed, pushed_objects) =
        commit(&one, &[("only.txt", b"pushed"), ("shared.txt", b"both")]).await?;
    let (unpushed, unpushed_objects) =
        commit(&one, &[("mine.txt", b"local"), ("shared.txt", b"both")]).await?;
    publish(&remote, "one", &pushed).await?;
    let offline = domain.registry("two")?;
    let (unreachable, _) = commit(&offline, &[("a.txt", b"a")]).await?;
    publish(&remote, "two", &unreachable).await?;
    remote.deny_reads(&manifest_uri("two", &unreachable).to_string());
    let local = domain.local();
    publish_local(&storage, &local, "me/analysis", &[("a.txt", b"local")]).await?;
    commit(&local, &[("draft.txt", b"not published")]).await?;
    let local_before = files(local.dir())?;
    let offline_before = files(offline.dir())?;

    let collected = gc(&storage, &remote, &domain, |_| None).await?;

    let namespace = Namespace::from(("acme", "demo"));
    assert!(!one.installed_manifest(&namespace, &pushed).exists());
    assert!(!pushed_objects[0].exists(), "used only by what was pushed");
    assert!(one.installed_manifest(&namespace, &unpushed).exists());
    for object in &unpushed_objects {
        assert!(object.exists(), "{} is the commit's", object.display());
    }
    assert!(matches!(
        collected.stores.get("two"),
        Some(StoreGc::Failed(_))
    ));
    assert_eq!(files(offline.dir())?, offline_before);
    assert_eq!(files(local.dir())?, local_before);
    assert!(!collected.stores.contains_key("@local"));
    assert!(collected.incomplete());
    assert_eq!(collected.total(), freed(&collected, "one").ok_or("one")?);
    Ok(())
}

/// The estimate deletes nothing, and says what gc then frees, store by
/// store; once gc ran, it says nothing is left.
#[test(tokio::test)]
async fn the_estimate_matches_what_gc_frees() -> Res {
    let dir = tempfile::tempdir()?;
    let domain = DomainRoot::new(dir.path());
    let storage = LocalStorage::new();
    let remote = MockRemote::default();
    for bucket in ["one", "two"] {
        let store = domain.registry(bucket)?;
        let (pushed, _) = commit(&store, &[("a.txt", b"alpha"), ("b.txt", b"bravo")]).await?;
        commit(&store, &[("a.txt", b"alpha"), ("c.txt", b"charlie")]).await?;
        publish(&remote, bucket, &pushed).await?;
        // A copy fetched from the bucket, as a read caches one.
        let cached = store.cached_manifest(&pushed);
        std::fs::create_dir_all(cached.parent().ok_or("a parent")?)?;
        std::fs::copy(
            store.installed_manifest(&Namespace::from(("acme", "demo")), &pushed),
            &cached,
        )?;
    }
    let before = files(dir.path())?;

    let estimate = gc_estimate(&storage, &remote, &domain, |_| None).await?;
    assert_eq!(files(dir.path())?, before, "the estimate deletes nothing");
    let collected = gc(&storage, &remote, &domain, |_| None).await?;

    for bucket in ["one", "two"] {
        let estimated = freed(&estimate, bucket).ok_or("an estimate")?;
        assert_eq!(Some(estimated), freed(&collected, bucket), "{bucket}");
        assert_eq!((estimated.cached_manifests, estimated.objects), (2, 1));
    }
    assert_eq!(estimate.total(), collected.total());
    let after = gc_estimate(&storage, &remote, &domain, |_| None).await?;
    assert!(after.total().is_empty(), "{after:?}");
    Ok(())
}
