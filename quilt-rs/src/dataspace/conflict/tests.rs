use super::*;

use std::path::PathBuf;

use test_log::test;

use crate::dataspace::METADATA_FILE;
use crate::dataspace::contract::Contract;
use crate::dataspace::contract::FolderCase;
use crate::io::remote::mocks::MockRemote;
use crate::io::storage::StorageExt;
use crate::io::storage::mocks::MockStorage;
use crate::manifest::Manifest;
use crate::manifest::ManifestHeader;
use crate::manifest::ManifestRow;
use crate::manifest::RawNumbers;
use crate::paths::file_url;
use quilt_uri::Namespace;
use quilt_uri::S3Uri;

type Res = Result<(), Box<dyn std::error::Error>>;

const DATASPACE: &str = "/work/rnaseq";
const FIRST: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const SECOND: &str = "2222222222222222222222222222222222222222222222222222222222222222";
const PACKAGE: &str = "quilt+s3://b#package=team/raw";

fn dataspace() -> PathBuf {
    PathBuf::from(DATASPACE)
}

fn entry() -> Entry {
    let text = "registry = \"s3://b\"\n[packages]\nread = [\"team/raw\"]";
    let contract = Contract::parse(text.as_bytes(), &dataspace(), FolderCase::Sensitive, None)
        .expect("contract");
    contract.entries()[0].clone()
}

fn folder() -> PathBuf {
    dataspace().join("team/raw")
}

fn key(name: &str) -> LogicalKey {
    LogicalKey::new(name).expect("key")
}

fn row(name: &str, body: &[u8]) -> ManifestRow {
    use sha2::Digest;
    let hash =
        multihash::Multihash::<256>::wrap(0x12, &sha2::Sha256::digest(body)).expect("multihash");
    ManifestRow {
        logical_key: key(name),
        physical_key: format!("s3://b/team/raw/{name}?versionId={}", hex::encode(body)),
        hash: hash.try_into().expect("hash"),
        size: body.len() as u64,
        meta: None,
        meta_numbers: RawNumbers::default(),
    }
}

/// Stores `files` as revision `hash` under `header`, tagged `latest`.
async fn publish(
    remote: &MockRemote,
    hash: &str,
    files: &[(&str, &[u8])],
    header: ManifestHeader,
) -> Result<Manifest, Box<dyn std::error::Error>> {
    let mut rows = Vec::new();
    for (name, body) in files {
        let row = row(name, body);
        let uri = S3Uri::try_from(row.physical_key.as_str())?;
        remote.put_object(None, &uri, body.to_vec()).await?;
        rows.push(row);
    }
    let manifest = Manifest {
        header,
        rows,
        ..Manifest::default()
    };
    let at = format!("s3://b/.quilt/packages/{hash}");
    let latest = "s3://b/.quilt/named_packages/team/raw/latest";
    remote
        .put_object(
            None,
            &S3Uri::try_from(at.as_str())?,
            manifest.to_jsonlines().into_bytes(),
        )
        .await?;
    remote
        .put_object(None, &S3Uri::try_from(latest)?, hash.as_bytes().to_vec())
        .await?;
    Ok(manifest)
}

async fn write(storage: &MockStorage, path: &Path, body: &[u8]) -> Result<(), crate::Error> {
    storage
        .write_byte_stream(path, ByteStream::from(body.to_vec()))
        .await
}

async fn text(storage: &MockStorage, name: &str) -> Option<String> {
    let bytes = storage.read_bytes(folder().join(name)).await.ok()?;
    Some(String::from_utf8(bytes).expect("utf-8"))
}

async fn read(
    storage: &MockStorage,
    remote: &MockRemote,
    base: Option<&str>,
) -> Result<ReadOutcome, EntryError> {
    read_resolving(storage, remote, None, &dataspace(), &entry(), base).await
}

async fn map(storage: &MockStorage) -> Result<ConflictMap, EntryError> {
    ConflictMap::read(storage, &dataspace()).await
}

/// The package's rows in the map.
async fn rows(storage: &MockStorage) -> Result<BTreeMap<LogicalKey, Row>, EntryError> {
    Ok(map(storage)
        .await?
        .packages
        .remove(&entry().key())
        .unwrap_or_default())
}

/// Sets `name`'s decision in the map to `resolved`.
async fn decide(storage: &MockStorage, name: &str, resolved: &str) -> Res {
    let mut map = map(storage).await?;
    let rows = map.packages.get_mut(&entry().key()).ok_or("no rows")?;
    rows.get_mut(&key(name)).ok_or("no row")?.resolved = Some(resolved.to_string());
    map.write(storage, &dataspace()).await?;
    Ok(())
}

/// A folder read at [`FIRST`], `a.txt` and `b.txt` both changed here, and
/// [`SECOND`] changing both too: the read of it blocked.
async fn collided() -> Result<(MockStorage, MockRemote), Box<dyn std::error::Error>> {
    let remote = MockRemote::default();
    let storage = MockStorage::default();
    let files: &[(&str, &[u8])] = &[("a.txt", b"v1"), ("b.txt", b"v1"), ("c.txt", b"v1")];
    publish(&remote, FIRST, files, ManifestHeader::default()).await?;
    read(&storage, &remote, None).await?;
    write(&storage, &folder().join("a.txt"), b"mine a").await?;
    write(&storage, &folder().join("b.txt"), b"mine b").await?;
    let files: &[(&str, &[u8])] = &[("a.txt", b"v2"), ("b.txt", b"v2"), ("c.txt", b"v2")];
    publish(&remote, SECOND, files, ManifestHeader::default()).await?;
    let outcome = read(&storage, &remote, Some(FIRST)).await?;
    assert!(
        matches!(outcome, ReadOutcome::Blocked { .. }),
        "{outcome:?}"
    );
    Ok((storage, remote))
}

fn completed(outcome: &ReadOutcome) -> Vec<PathBuf> {
    assert_eq!(outcome.lock_base(), Some(SECOND), "{outcome:?}");
    match outcome {
        ReadOutcome::Read(report) => report.resolved.clone(),
        ReadOutcome::Blocked { .. } => unreachable!(),
    }
}

#[test(tokio::test)]
async fn a_blocked_read_writes_each_colliding_key_by_package_with_both_sides() -> Res {
    let (storage, _) = collided().await?;

    let bytes = storage.read_bytes(dataspace().join(CONFLICT_FILE)).await;

    let written: serde_json::Value = match bytes {
        Ok(bytes) => serde_json::from_slice(&bytes)?,
        Err(err) => panic!("no {CONFLICT_FILE}: {err}"),
    };
    let incoming = row("a.txt", b"v2");
    let mine = row("a.txt", b"mine a");
    assert_eq!(
        written[PACKAGE]["a.txt"],
        serde_json::json!({
            "incoming": {
                "physical_key": incoming.physical_key,
                "hash": incoming.hash,
                "size": 2,
            },
            "current": {
                "physical_key": file_url(&folder().join("a.txt"))?,
                "hash": mine.hash,
                "size": 6,
            },
        })
    );
    assert!(written[PACKAGE]["b.txt"].is_object());
    assert!(written[PACKAGE]["c.txt"].is_null(), "not changed here");
    Ok(())
}

#[test(tokio::test)]
async fn keys_decided_are_set_as_decided_and_the_read_completes() -> Res {
    let (storage, remote) = collided().await?;
    decide(&storage, "a.txt", "").await?;
    decide(&storage, "b.txt", &row("b.txt", b"v2").physical_key).await?;

    let outcome = read(&storage, &remote, Some(FIRST)).await?;

    assert_eq!(
        completed(&outcome),
        [PathBuf::from("a.txt"), PathBuf::from("b.txt")]
    );
    assert_eq!(text(&storage, "a.txt").await, None);
    assert_eq!(text(&storage, "b.txt").await.as_deref(), Some("v2"));
    assert_eq!(text(&storage, "c.txt").await.as_deref(), Some("v2"));
    assert!(
        !storage.exists(dataspace().join(CONFLICT_FILE)).await,
        "no package has rows left"
    );
    Ok(())
}

#[test(tokio::test)]
async fn a_key_decided_as_a_local_file_or_an_s3_version_takes_its_bytes() -> Res {
    let (storage, remote) = collided().await?;
    let merged = dataspace().join("merged.txt");
    write(&storage, &merged, b"merged by hand").await?;
    decide(&storage, "a.txt", &file_url(&merged)?).await?;
    let object = S3Uri::try_from("s3://b/elsewhere.txt?versionId=v7")?;
    remote
        .put_object(None, &object, b"from S3".to_vec())
        .await?;
    decide(&storage, "b.txt", &object.to_string()).await?;

    let outcome = read(&storage, &remote, Some(FIRST)).await?;

    assert_eq!(completed(&outcome).len(), 2);
    assert_eq!(
        text(&storage, "a.txt").await.as_deref(),
        Some("merged by hand")
    );
    assert_eq!(text(&storage, "b.txt").await.as_deref(), Some("from S3"));
    Ok(())
}

#[test(tokio::test)]
async fn an_s3_object_without_a_version_is_refused_and_nothing_changes() -> Res {
    let (storage, remote) = collided().await?;
    decide(&storage, "a.txt", "").await?;
    decide(&storage, "b.txt", "s3://b/elsewhere.txt").await?;
    let before = map(&storage).await?;

    let outcome = read(&storage, &remote, Some(FIRST)).await;

    assert!(
        matches!(&outcome, Err(EntryError::ConflictMap(why)) if why.contains("versionId")),
        "{outcome:?}"
    );
    assert_eq!(text(&storage, "a.txt").await.as_deref(), Some("mine a"));
    assert_eq!(map(&storage).await?, before);
    Ok(())
}

#[test(tokio::test)]
async fn a_decision_on_a_key_changed_since_its_row_is_not_applied() -> Res {
    let (storage, remote) = collided().await?;
    decide(&storage, "a.txt", "").await?;
    decide(&storage, "b.txt", "").await?;
    write(&storage, &folder().join("a.txt"), b"mine again").await?;

    let outcome = read(&storage, &remote, Some(FIRST)).await?;

    assert_eq!(
        outcome,
        ReadOutcome::Blocked {
            revision: SECOND.to_string(),
            conflicts: vec![PathBuf::from("a.txt")],
        }
    );
    assert_eq!(text(&storage, "a.txt").await.as_deref(), Some("mine again"));
    assert_eq!(text(&storage, "b.txt").await.as_deref(), Some("mine b"));
    let rows = rows(&storage).await?;
    assert_eq!(rows[&key("a.txt")].resolved, None);
    assert_eq!(rows[&key("b.txt")].resolved.as_deref(), Some(""), "kept");
    Ok(())
}

#[test(tokio::test)]
async fn the_metadata_file_decided_as_a_local_file_takes_its_text() -> Res {
    let remote = MockRemote::default();
    let storage = MockStorage::default();
    let header = |run: i64| {
        let mut header = ManifestHeader::default();
        header.set_meta(serde_json::json!({ "run": run }), RawNumbers::default());
        header
    };
    publish(&remote, FIRST, &[("a.txt", b"v1")], header(1)).await?;
    read(&storage, &remote, None).await?;
    let metadata = folder().join(METADATA_FILE);
    write(&storage, &metadata, b"message = \"mine\"\n").await?;
    publish(&remote, SECOND, &[("a.txt", b"v1")], header(2)).await?;
    read(&storage, &remote, Some(FIRST)).await?;
    let chosen = dataspace().join("chosen.toml");
    write(&storage, &chosen, b"message = \"chosen\"\n").await?;
    decide(&storage, METADATA_FILE, &file_url(&chosen)?).await?;

    let outcome = read(&storage, &remote, Some(FIRST)).await?;

    assert_eq!(completed(&outcome), [PathBuf::from(METADATA_FILE)]);
    assert_eq!(
        text(&storage, METADATA_FILE).await.as_deref(),
        Some("message = \"chosen\"\n")
    );
    Ok(())
}

#[test(tokio::test)]
async fn a_completed_read_keeps_other_packages_rows() -> Res {
    let (storage, remote) = collided().await?;
    decide(&storage, "a.txt", "").await?;
    decide(&storage, "b.txt", "").await?;
    let mut map = map(&storage).await?;
    let other: PackageKey = "quilt+s3://b#package=team/other".parse()?;
    let rows = map.packages[&entry().key()].clone();
    map.packages.insert(other.clone(), rows.clone());
    map.write(&storage, &dataspace()).await?;

    read(&storage, &remote, Some(FIRST)).await?;

    let left = ConflictMap::read(&storage, &dataspace()).await?;
    assert_eq!(left.packages, BTreeMap::from([(other, rows)]));
    Ok(())
}

#[test(tokio::test)]
async fn a_map_that_cannot_be_read_fails_the_read_and_is_left_as_it_is() -> Res {
    let (storage, remote) = collided().await?;
    let path = dataspace().join(CONFLICT_FILE);
    write(&storage, &path, b"{ not json").await?;

    let outcome = read(&storage, &remote, Some(FIRST)).await;

    assert!(
        matches!(outcome, Err(EntryError::ConflictMap(_))),
        "{outcome:?}"
    );
    assert_eq!(storage.read_bytes(&path).await?, b"{ not json");
    Ok(())
}

/// A link is not a file to set.
#[cfg(unix)]
#[test(tokio::test)]
async fn a_decision_on_a_link_is_refused() -> Res {
    let (storage, remote) = collided().await?;
    let link = storage.temp_dir.path().join("work/rnaseq/team/raw/a.txt");
    std::fs::remove_file(&link)?;
    std::os::unix::fs::symlink(storage.temp_dir.path().join("nowhere"), &link)?;
    read(&storage, &remote, Some(FIRST)).await?;
    decide(&storage, "a.txt", "").await?;
    decide(&storage, "b.txt", "").await?;

    let outcome = read(&storage, &remote, Some(FIRST)).await;

    assert!(
        matches!(outcome, Err(EntryError::ConflictMap(_))),
        "{outcome:?}"
    );
    assert!(storage.is_symlink(folder().join("a.txt")).await);
    Ok(())
}

#[test(tokio::test)]
async fn a_decision_holds_however_the_folders_path_was_spelled() -> Res {
    let (storage, remote) = collided().await?;
    let mut map = map(&storage).await?;
    for (name, row) in map.packages.values_mut().flat_map(BTreeMap::iter_mut) {
        let elsewhere = format!("file:///another/spelling/{name}");
        let current = row.current.as_mut().ok_or("no current")?;
        current.physical_key = Some(elsewhere.clone());
        // `b.txt` is kept as the folder holds it, by the path the row named.
        row.resolved = Some(if name == &key("b.txt") {
            elsewhere
        } else {
            String::new()
        });
    }
    map.write(&storage, &dataspace()).await?;

    let outcome = read(&storage, &remote, Some(FIRST)).await;

    let outcome = outcome.unwrap_or_else(|err| panic!("the decisions were refused: {err}"));
    assert_eq!(completed(&outcome).len(), 2);
    assert_eq!(text(&storage, "a.txt").await, None);
    assert_eq!(text(&storage, "b.txt").await.as_deref(), Some("mine b"));
    Ok(())
}

#[test(tokio::test)]
async fn an_s3_version_that_cannot_be_fetched_names_its_key() -> Res {
    let (storage, remote) = collided().await?;
    decide(&storage, "a.txt", "").await?;
    decide(&storage, "b.txt", "s3://b/nowhere.txt?versionId=v1").await?;

    let outcome = read(&storage, &remote, Some(FIRST)).await;

    assert!(
        matches!(&outcome, Err(EntryError::ConflictMap(why)) if why.starts_with("b.txt cannot be set")),
        "{outcome:?}"
    );
    assert_eq!(text(&storage, "a.txt").await.as_deref(), Some("mine a"));
    Ok(())
}

#[test(tokio::test)]
async fn decisions_saved_or_changed_while_the_read_runs_are_kept() -> Res {
    let (storage, remote) = collided().await?;
    decide(&storage, "b.txt", "").await?;
    let incoming = row("b.txt", b"v2").physical_key;
    let manifest = remote.park(&format!("s3://b/.quilt/packages/{SECOND}"));
    let deciding = async {
        manifest.arrived().await;
        decide(&storage, "a.txt", "").await?;
        decide(&storage, "b.txt", &incoming).await?;
        manifest.release();
        Res::Ok(())
    };

    let (outcome, decided) = tokio::join!(read(&storage, &remote, Some(FIRST)), deciding);

    decided?;
    assert!(matches!(outcome?, ReadOutcome::Blocked { .. }));
    let rows = rows(&storage).await?;
    assert_eq!(rows[&key("a.txt")].resolved.as_deref(), Some(""));
    assert_eq!(
        rows[&key("b.txt")].resolved.as_deref(),
        Some(incoming.as_str())
    );
    Ok(())
}

#[test(tokio::test)]
async fn a_decision_on_an_incoming_object_since_replaced_is_not_applied() -> Res {
    let (storage, remote) = collided().await?;
    decide(&storage, "a.txt", &row("a.txt", b"v2").physical_key).await?;
    decide(&storage, "b.txt", "").await?;
    let mut rows = vec![
        row("a.txt", b"v2"),
        row("b.txt", b"v2"),
        row("c.txt", b"v2"),
    ];
    // The same bytes, as another object.
    rows[0].physical_key = "s3://b/team/raw/a.txt?versionId=again".to_string();
    let again = S3Uri::try_from(rows[0].physical_key.as_str())?;
    remote.put_object(None, &again, b"v2".to_vec()).await?;
    let at = S3Uri::try_from(format!("s3://b/.quilt/packages/{SECOND}").as_str())?;
    let manifest = Manifest {
        rows,
        ..Manifest::default()
    };
    remote
        .put_object(None, &at, manifest.to_jsonlines().into_bytes())
        .await?;

    let outcome = read(&storage, &remote, Some(FIRST)).await?;

    assert_eq!(
        outcome,
        ReadOutcome::Blocked {
            revision: SECOND.to_string(),
            conflicts: vec![PathBuf::from("a.txt")],
        }
    );
    Ok(())
}

#[test(tokio::test)]
async fn a_read_that_cannot_place_every_file_sets_no_key() -> Res {
    let (storage, remote) = collided().await?;
    let mut rows = vec![
        row("a.txt", b"v2"),
        row("b.txt", b"v2"),
        row("c.txt", b"v2"),
    ];
    // An unversioned key whose bytes a later write replaced.
    rows[2].physical_key = "s3://b/team/raw/c.txt".to_string();
    remote
        .put_object(
            None,
            &S3Uri::try_from("s3://b/team/raw/c.txt")?,
            b"newer".to_vec(),
        )
        .await?;
    let manifest = Manifest {
        rows,
        ..Manifest::default()
    };
    let at = S3Uri::try_from(format!("s3://b/.quilt/packages/{SECOND}").as_str())?;
    remote
        .put_object(None, &at, manifest.to_jsonlines().into_bytes())
        .await?;
    decide(&storage, "a.txt", "").await?;
    decide(&storage, "b.txt", "").await?;

    let outcome = read(&storage, &remote, Some(FIRST)).await?;

    let ReadOutcome::Read(report) = outcome else {
        panic!("expected a read, got {outcome:?}");
    };
    assert_eq!(report.skipped, [PathBuf::from("c.txt")]);
    assert_eq!(report.resolved, Vec::<PathBuf>::new());
    assert_eq!(text(&storage, "a.txt").await.as_deref(), Some("mine a"));
    Ok(())
}

/// The error for a key masks a user and password in it.
#[test(tokio::test)]
async fn a_key_error_masks_a_user_and_password() -> Res {
    let storage = MockStorage::default();
    let body = r#"{"quilt+s3://KEY:secret@b#package=team/raw": {}}"#;
    write(&storage, &dataspace().join(CONFLICT_FILE), body.as_bytes()).await?;
    let Err(err) = map(&storage).await else {
        panic!("the key should be refused");
    };
    let text = err.to_string();
    assert!(!text.contains("secret") && !text.contains("KEY"), "{text}");
    assert!(text.contains("***@b"), "{text}");
    Ok(())
}

/// Rows kept under a name the local registry had move to its current name,
/// and rows already there stay.
#[test]
fn rows_under_an_old_name_of_the_local_registry_move_to_its_current_name() -> Res {
    let key_of = |name: &str| PackageKey {
        registry: Registry::Local(name.to_string()),
        package: Namespace::try_from("me/analysis").unwrap(),
    };
    let row_of = |resolved: &str| Row {
        other: Other::Incoming(None),
        current: None,
        resolved: Some(resolved.to_string()),
    };
    let mut map = ConflictMap::default();
    map.packages.insert(
        key_of("old-mac"),
        BTreeMap::from([(key("a.txt"), row_of("")), (key("b.txt"), row_of("old"))]),
    );
    map.packages.insert(
        key_of("lab-mac"),
        BTreeMap::from([(key("b.txt"), row_of("new"))]),
    );
    let names = LocalNames {
        name: "lab-mac".to_string(),
        aliases: vec!["old-mac".to_string()],
    };

    map.name_local(&names);

    assert_eq!(map.packages.len(), 1, "{map:?}");
    let rows = &map.packages[&key_of("lab-mac")];
    assert_eq!(rows[&key("a.txt")].resolved.as_deref(), Some(""));
    assert_eq!(rows[&key("b.txt")].resolved.as_deref(), Some("new"));
    Ok(())
}

/// An entry that names the local registry by an alias has its rows under the
/// registry's current name.
#[test]
fn an_entry_naming_the_local_registry_by_an_alias_is_keyed_by_its_current_name() -> Res {
    let mut entry = entry();
    entry.registry = Registry::Local("old-mac".to_string());
    let names = LocalNames {
        name: "lab-mac".to_string(),
        aliases: vec!["old-mac".to_string()],
    };

    let key = current_key(Some(&names), &entry);

    assert_eq!(key.registry, Registry::Local("lab-mac".to_string()));
    Ok(())
}

/// A publish's row holds `current` and `destination`, a read's `incoming`
/// and `current`; either side is `null` where it holds nothing, and a row
/// holding both or neither is refused.
#[test]
fn a_row_is_a_reads_or_a_publishs_and_keeps_a_side_that_removed_the_key() -> Res {
    let side = Side {
        physical_key: Some("s3://b/a.txt?versionId=v1".to_string()),
        hash: None,
        size: Some(1),
    };
    for (other, text) in [
        (
            Other::Destination(Some(side.clone())),
            r#"{"current":null,"destination":{"physical_key":"s3://b/a.txt?versionId=v1","size":1}}"#,
        ),
        (
            Other::Destination(None),
            r#"{"current":null,"destination":null}"#,
        ),
        (Other::Incoming(None), r#"{"incoming":null,"current":null}"#),
    ] {
        let row = Row {
            other,
            current: None,
            resolved: None,
        };
        assert_eq!(serde_json::to_string(&row)?, text);
        assert_eq!(serde_json::from_str::<Row>(text)?, row);
    }
    for text in [
        r#"{"incoming":null,"current":null,"destination":null}"#,
        r#"{"current":null}"#,
    ] {
        assert!(serde_json::from_str::<Row>(text).is_err(), "{text}");
    }
    Ok(())
}
