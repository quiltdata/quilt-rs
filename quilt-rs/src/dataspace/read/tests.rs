use super::*;

use std::path::Path;

use test_log::test;

use crate::dataspace::contract::Contract;
use crate::dataspace::contract::FolderCase;
use crate::io::remote::mocks::MockRemote;
use crate::io::storage::mocks::MockStorage;
use crate::manifest::ManifestHeader;
use crate::manifest::RawNumbers;
use quilt_uri::S3Uri;

type Res = Result<(), Box<dyn std::error::Error>>;

const DATASPACE: &str = "/work/rnaseq";
const FIRST: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const SECOND: &str = "2222222222222222222222222222222222222222222222222222222222222222";

fn entry(packages: &str) -> Entry {
    let text = format!("registry = \"s3://b\"\n[packages]\n{packages}");
    let contract = Contract::parse(
        text.as_bytes(),
        Path::new(DATASPACE),
        FolderCase::Sensitive,
        None,
    )
    .expect("contract");
    contract.entries()[0].clone()
}

/// The whole package under `read`, in its default folder.
fn whole() -> Entry {
    entry("read = [\"team/raw\"]")
}

fn folder() -> PathBuf {
    Path::new(DATASPACE).join("team/raw")
}

/// A row for `body`, stored on the remote at a version of its own.
fn row(key: &str, body: &[u8]) -> ManifestRow {
    use sha2::Digest;
    let hash =
        multihash::Multihash::<256>::wrap(0x12, &sha2::Sha256::digest(body)).expect("multihash");
    ManifestRow {
        logical_key: LogicalKey::new(key).expect("key"),
        physical_key: format!("s3://b/team/raw/{key}?versionId={}", hex::encode(body)),
        hash: hash.try_into().expect("hash"),
        size: body.len() as u64,
        meta: None,
        meta_numbers: RawNumbers::default(),
    }
}

/// Stores `files` and a manifest of them as revision `hash`, and tags it
/// `latest`.
async fn publish(
    remote: &MockRemote,
    hash: &str,
    files: &[(&str, &[u8])],
) -> Result<Manifest, Box<dyn std::error::Error>> {
    publish_with(remote, hash, files, ManifestHeader::default()).await
}

async fn publish_with(
    remote: &MockRemote,
    hash: &str,
    files: &[(&str, &[u8])],
    header: ManifestHeader,
) -> Result<Manifest, Box<dyn std::error::Error>> {
    let mut rows = Vec::new();
    for (key, body) in files {
        let row = row(key, body);
        remote
            .put_object(
                None,
                &S3Uri::try_from(row.physical_key.as_str())?,
                body.to_vec(),
            )
            .await?;
        rows.push(row);
    }
    let manifest = Manifest {
        header,
        rows,
        ..Manifest::default()
    };
    put_manifest(remote, hash, &manifest).await?;
    remote
        .put_object(
            None,
            &S3Uri::try_from("s3://b/.quilt/named_packages/team/raw/latest")?,
            hash.as_bytes().to_vec(),
        )
        .await?;
    Ok(manifest)
}

async fn put_manifest(
    remote: &MockRemote,
    hash: &str,
    manifest: &Manifest,
) -> Result<(), crate::Error> {
    remote
        .put_object(
            None,
            &S3Uri::try_from(format!("s3://b/.quilt/packages/{hash}").as_str())?,
            manifest.to_jsonlines().into_bytes(),
        )
        .await
}

async fn write(storage: &MockStorage, key: &str, body: &[u8]) -> Result<(), crate::Error> {
    storage
        .write_byte_stream(folder().join(key), ByteStream::from(body.to_vec()))
        .await
}

async fn text(storage: &MockStorage, key: &str) -> Option<String> {
    let bytes = storage.read_bytes(folder().join(key)).await.ok()?;
    Some(String::from_utf8(bytes).expect("utf-8"))
}

fn report(outcome: ReadOutcome) -> ReadReport {
    match outcome {
        ReadOutcome::Read(report) => report,
        blocked @ ReadOutcome::Blocked { .. } => panic!("expected a read, got {blocked:?}"),
    }
}

fn keys(names: &[&str]) -> Vec<PathBuf> {
    names.iter().map(PathBuf::from).collect()
}

/// Whether the disk `storage` keeps its files on finds a name in any case.
async fn ignores_case(storage: &MockStorage) -> Result<bool, crate::Error> {
    write(storage, "probe", b"").await?;
    let ignores = storage.exists(folder().join("PROBE")).await;
    storage.remove_file(folder().join("probe")).await?;
    Ok(ignores)
}

#[test(tokio::test)]
async fn a_first_read_fills_the_entrys_paths_and_writes_the_metadata() -> Res {
    let remote = MockRemote::default();
    let storage = MockStorage::default();
    let mut header = ManifestHeader::default();
    let (meta, numbers) = RawNumbers::parse(r#"{"run": 18446744073709551616}"#)?;
    header.set_meta(meta, numbers);
    let files: &[(&str, &[u8])] = &[("src/a.txt", b"alpha"), ("docs/b.txt", b"beta")];
    let mut manifest = publish_with(&remote, FIRST, files, header).await?;
    manifest.rows[0].meta = Some(serde_json::json!({"user_meta": {"kind": "raw"}}));
    put_manifest(&remote, FIRST, &manifest).await?;
    let entry = entry(&format!(
        "read = [{{ package = \"team/raw\", revision = \"{FIRST}\", path = \"src/\" }}]"
    ));

    let outcome = read(&storage, &remote, &entry, None).await?;

    assert_eq!(outcome.lock_base(), Some(FIRST));
    assert_eq!(report(outcome).placed, keys(&["src/a.txt", METADATA_FILE]));
    assert_eq!(text(&storage, "src/a.txt").await.as_deref(), Some("alpha"));
    assert_eq!(
        text(&storage, "docs/b.txt").await,
        None,
        "outside the entry's paths"
    );
    assert_eq!(
        text(&storage, METADATA_FILE).await.as_deref(),
        Some("{\".\":{\"run\":18446744073709551616},\"src/a.txt\":{\"kind\":\"raw\"}}\n")
    );
    Ok(())
}

#[test(tokio::test)]
async fn a_package_holding_anything_at_the_metadata_files_place_is_refused() -> Res {
    for name in [
        METADATA_FILE,
        "Package-Metadata.JSON",
        "package-metadata.json/inside.txt",
        "package-metadata.json\\inside.txt",
    ] {
        let remote = MockRemote::default();
        let storage = MockStorage::default();
        publish(&remote, FIRST, &[("a.txt", b"alpha"), (name, b"{}")]).await?;

        let outcome = read(&storage, &remote, &whole(), None).await;

        assert!(
            matches!(outcome, Err(EntryError::HoldsMetadataFile)),
            "{name}: {outcome:?}"
        );
        assert_eq!(text(&storage, "a.txt").await, None);
    }

    let remote = MockRemote::default();
    let storage = MockStorage::default();
    let mut manifest = publish(&remote, FIRST, &[("a.txt", b"alpha")]).await?;
    manifest.dir_meta = vec![crate::manifest::DirMeta::new(
        LogicalKey::new(METADATA_FILE)?,
        serde_json::json!({"note": "a folder"}),
    )];
    put_manifest(&remote, FIRST, &manifest).await?;

    let folder_meta = read(&storage, &remote, &whole(), None).await;

    assert!(
        matches!(folder_meta, Err(EntryError::HoldsMetadataFile)),
        "folder metadata: {folder_meta:?}"
    );
    Ok(())
}

#[test(tokio::test)]
async fn an_entry_under_another_access_is_not_read() -> Res {
    let remote = MockRemote::default();
    let storage = MockStorage::default();
    publish(&remote, FIRST, &[("a.txt", b"alpha")]).await?;

    let outcome = read(&storage, &remote, &entry("write = [\"team/raw\"]"), None).await;

    assert!(
        matches!(outcome, Err(EntryError::Access(Access::Write))),
        "{outcome:?}"
    );
    Ok(())
}

/// A local registry is in the Domain, which a read without one cannot reach.
#[test(tokio::test)]
async fn a_local_registrys_package_is_not_read() {
    let entry = entry("read = [\"quilt+local://lab-mac#package=team/raw\"]");

    let outcome = read(
        &MockStorage::default(),
        &MockRemote::default(),
        &entry,
        None,
    )
    .await;

    assert!(
        matches!(outcome, Err(EntryError::LocalRegistry)),
        "{outcome:?}"
    );
}

#[test(tokio::test)]
async fn a_first_read_agrees_with_identical_files_and_leaves_local_work() -> Res {
    let remote = MockRemote::default();
    let storage = MockStorage::default();
    publish(&remote, FIRST, &[("a.txt", b"alpha"), ("b.txt", b"beta")]).await?;
    write(&storage, "a.txt", b"alpha").await?;
    write(&storage, "mine.txt", b"local").await?;

    let outcome = read(&storage, &remote, &whole(), None).await?;

    assert_eq!(report(outcome).placed, keys(&["b.txt", METADATA_FILE]));
    assert_eq!(text(&storage, "mine.txt").await.as_deref(), Some("local"));
    Ok(())
}

#[test(tokio::test)]
async fn a_first_read_over_a_differing_file_is_blocked_and_changes_nothing() -> Res {
    let remote = MockRemote::default();
    let storage = MockStorage::default();
    publish(&remote, FIRST, &[("a.txt", b"alpha"), ("b.txt", b"beta")]).await?;
    write(&storage, "a.txt", b"mine").await?;

    let outcome = read(&storage, &remote, &whole(), None).await?;

    assert_eq!(outcome.lock_base(), None);
    assert_eq!(
        outcome,
        ReadOutcome::Blocked {
            revision: FIRST.to_string(),
            conflicts: keys(&["a.txt"]),
        }
    );
    assert_eq!(text(&storage, "a.txt").await.as_deref(), Some("mine"));
    assert_eq!(text(&storage, "b.txt").await, None);
    assert_eq!(text(&storage, METADATA_FILE).await, None);
    Ok(())
}

#[test(tokio::test)]
async fn a_newer_revision_replaces_unchanged_files_and_keeps_local_changes() -> Res {
    let remote = MockRemote::default();
    let storage = MockStorage::default();
    let entry = whole();
    publish(
        &remote,
        FIRST,
        &[
            ("same.txt", b"same"),
            ("edit.txt", b"v1"),
            ("gone.txt", b"old"),
            ("mine.txt", b"v1"),
        ],
    )
    .await?;
    read(&storage, &remote, &entry, None).await?;
    write(&storage, "mine.txt", b"local edit").await?;
    publish(
        &remote,
        SECOND,
        &[
            ("same.txt", b"same"),
            ("edit.txt", b"v2"),
            ("new.txt", b"new"),
            ("mine.txt", b"v1"),
        ],
    )
    .await?;

    let outcome = read(&storage, &remote, &entry, Some(FIRST)).await?;

    assert_eq!(outcome.lock_base(), Some(SECOND));
    let report = report(outcome);
    assert_eq!(report.placed, keys(&["edit.txt", "new.txt"]));
    assert_eq!(report.removed, keys(&["gone.txt"]));
    assert_eq!(report.kept, keys(&["mine.txt"]));
    assert_eq!(text(&storage, "edit.txt").await.as_deref(), Some("v2"));
    assert_eq!(text(&storage, "gone.txt").await, None);
    assert_eq!(
        text(&storage, "mine.txt").await.as_deref(),
        Some("local edit")
    );
    Ok(())
}

#[test(tokio::test)]
async fn a_local_change_to_a_file_the_revision_changed_blocks_the_read() -> Res {
    let remote = MockRemote::default();
    let storage = MockStorage::default();
    let entry = whole();
    publish(
        &remote,
        FIRST,
        &[
            ("edit.txt", b"v1"),
            ("gone.txt", b"old"),
            ("other.txt", b"v1"),
        ],
    )
    .await?;
    read(&storage, &remote, &entry, None).await?;
    write(&storage, "edit.txt", b"local edit").await?;
    write(&storage, "gone.txt", b"local edit").await?;
    publish(
        &remote,
        SECOND,
        &[("edit.txt", b"v2"), ("other.txt", b"v2")],
    )
    .await?;

    let outcome = read(&storage, &remote, &entry, Some(FIRST)).await?;

    assert_eq!(
        outcome,
        ReadOutcome::Blocked {
            revision: SECOND.to_string(),
            conflicts: keys(&["edit.txt", "gone.txt"]),
        }
    );
    assert_eq!(text(&storage, "other.txt").await.as_deref(), Some("v1"));
    Ok(())
}

#[test(tokio::test)]
async fn a_file_missing_from_the_folder_is_filled_again() -> Res {
    let remote = MockRemote::default();
    let storage = MockStorage::default();
    let entry = whole();
    publish(&remote, FIRST, &[("a.txt", b"alpha")]).await?;
    read(&storage, &remote, &entry, None).await?;
    storage.remove_file(folder().join("a.txt")).await?;

    let outcome = read(&storage, &remote, &entry, Some(FIRST)).await?;

    assert_eq!(report(outcome).placed, keys(&["a.txt"]));
    assert_eq!(text(&storage, "a.txt").await.as_deref(), Some("alpha"));
    Ok(())
}

#[test(tokio::test)]
async fn the_metadata_file_is_compared_like_any_other_file() -> Res {
    let remote = MockRemote::default();
    let storage = MockStorage::default();
    let entry = whole();
    let with_meta = |version: u64| {
        let mut header = ManifestHeader::default();
        header.set_meta(serde_json::json!({ "v": version }), RawNumbers::default());
        header
    };
    let files: &[(&str, &[u8])] = &[("a.txt", b"alpha")];
    publish_with(&remote, FIRST, files, with_meta(1)).await?;
    read(&storage, &remote, &entry, None).await?;
    write(&storage, METADATA_FILE, b"{\".\":{\"v\":\"mine\"}}\n").await?;
    publish_with(&remote, SECOND, files, with_meta(1)).await?;

    let unchanged_remotely = read(&storage, &remote, &entry, Some(FIRST)).await?;

    assert_eq!(report(unchanged_remotely).kept, keys(&[METADATA_FILE]));
    publish_with(&remote, SECOND, files, with_meta(2)).await?;
    let changed_remotely = read(&storage, &remote, &entry, Some(FIRST)).await?;
    assert_eq!(
        changed_remotely,
        ReadOutcome::Blocked {
            revision: SECOND.to_string(),
            conflicts: keys(&[METADATA_FILE]),
        }
    );
    Ok(())
}

#[test(tokio::test)]
async fn a_revision_some_of_whose_bytes_are_gone_changes_nothing() -> Res {
    let remote = MockRemote::default();
    let storage = MockStorage::default();
    publish(&remote, FIRST, &[("a.txt", b"alpha")]).await?;
    let mut manifest = Manifest::default();
    let mut replaced = row("b.txt", b"beta");
    replaced.physical_key = "s3://b/team/raw/b.txt".to_string();
    remote
        .put_object(
            None,
            &S3Uri::try_from("s3://b/team/raw/b.txt")?,
            b"newer".to_vec(),
        )
        .await?;
    manifest.rows = vec![row("a.txt", b"alpha"), replaced];
    put_manifest(&remote, FIRST, &manifest).await?;

    let outcome = read(&storage, &remote, &whole(), None).await;

    assert!(
        matches!(
            &outcome,
            Ok(ReadOutcome::Read(report))
                if report.skipped == keys(&["b.txt"]) && report.placed.is_empty()
        ),
        "{outcome:?}"
    );
    assert_eq!(outcome?.lock_base(), None);
    assert_eq!(text(&storage, "a.txt").await, None);
    assert_eq!(text(&storage, METADATA_FILE).await, None);
    Ok(())
}

#[test(tokio::test)]
async fn a_file_edited_while_the_revision_downloads_stops_the_read_before_any_change() -> Res {
    let remote = MockRemote::default();
    let storage = MockStorage::default();
    let entry = whole();
    publish(&remote, FIRST, &[("a.txt", b"v1"), ("edit.txt", b"v1")]).await?;
    read(&storage, &remote, &entry, None).await?;
    publish(&remote, SECOND, &[("a.txt", b"v2"), ("edit.txt", b"v2")]).await?;
    let download = remote.park(&row("edit.txt", b"v2").physical_key);
    let edit = async {
        download.arrived().await;
        write(&storage, "edit.txt", b"mine").await?;
        download.release();
        Ok::<_, crate::Error>(())
    };

    let (outcome, edited) = tokio::join!(read(&storage, &remote, &entry, Some(FIRST)), edit);
    edited?;

    assert!(
        matches!(&outcome, Err(EntryError::Changed(key)) if key == Path::new("edit.txt")),
        "{outcome:?}"
    );
    assert_eq!(text(&storage, "edit.txt").await.as_deref(), Some("mine"));
    assert_eq!(text(&storage, "a.txt").await.as_deref(), Some("v1"));
    Ok(())
}

#[test(tokio::test)]
async fn a_dropped_file_deleted_while_the_revision_downloads_is_not_reported_removed() -> Res {
    let remote = MockRemote::default();
    let storage = MockStorage::default();
    let entry = whole();
    publish(&remote, FIRST, &[("a.txt", b"v1"), ("gone.txt", b"old")]).await?;
    read(&storage, &remote, &entry, None).await?;
    publish(&remote, SECOND, &[("a.txt", b"v2")]).await?;
    let download = remote.park(&row("a.txt", b"v2").physical_key);
    let delete = async {
        download.arrived().await;
        storage.remove_file(folder().join("gone.txt")).await?;
        download.release();
        Ok::<_, std::io::Error>(())
    };

    let (outcome, deleted) = tokio::join!(read(&storage, &remote, &entry, Some(FIRST)), delete);
    deleted?;

    let report = report(outcome?);
    assert_eq!(report.removed, Vec::<PathBuf>::new());
    assert_eq!(report.placed, keys(&["a.txt"]));
    Ok(())
}

#[test(tokio::test)]
async fn a_metadata_file_edited_while_the_revision_downloads_is_left_as_it_is() -> Res {
    let remote = MockRemote::default();
    let storage = MockStorage::default();
    let entry = whole();
    let with_meta = |version: u64| {
        let mut header = ManifestHeader::default();
        header.set_meta(serde_json::json!({ "v": version }), RawNumbers::default());
        header
    };
    publish_with(&remote, FIRST, &[("a.txt", b"v1")], with_meta(1)).await?;
    read(&storage, &remote, &entry, None).await?;
    publish_with(&remote, SECOND, &[("a.txt", b"v2")], with_meta(2)).await?;
    let download = remote.park(&row("a.txt", b"v2").physical_key);
    let edit = async {
        download.arrived().await;
        write(&storage, METADATA_FILE, b"mine").await?;
        download.release();
        Ok::<_, crate::Error>(())
    };

    let (outcome, edited) = tokio::join!(read(&storage, &remote, &entry, Some(FIRST)), edit);
    edited?;

    assert!(
        matches!(&outcome, Err(EntryError::Changed(key)) if key == Path::new(METADATA_FILE)),
        "{outcome:?}"
    );
    assert_eq!(text(&storage, METADATA_FILE).await.as_deref(), Some("mine"));
    assert_eq!(text(&storage, "a.txt").await.as_deref(), Some("v1"));
    Ok(())
}

/// A link would put a write or a deletion outside the working folder.
#[cfg(unix)]
#[test(tokio::test)]
async fn a_linked_folder_inside_the_working_folder_is_not_written_through() -> Res {
    let remote = MockRemote::default();
    let storage = MockStorage::default();
    let entry = whole();
    publish(&remote, FIRST, &[("data/x.csv", b"v1")]).await?;
    read(&storage, &remote, &entry, None).await?;
    let outside = Path::new("/elsewhere");
    storage
        .write_byte_stream(outside.join("x.csv"), ByteStream::from(b"v1".to_vec()))
        .await?;
    storage.remove_dir_all(folder().join("data")).await?;
    let root = storage.temp_dir.path();
    std::os::unix::fs::symlink(
        root.join("elsewhere"),
        root.join("work/rnaseq/team/raw/data"),
    )?;
    publish(&remote, SECOND, &[("data/x.csv", b"v2")]).await?;

    let outcome = read(&storage, &remote, &entry, Some(FIRST)).await;

    assert!(matches!(outcome, Err(EntryError::Failed(_))), "{outcome:?}");
    let outside_bytes = storage.read_bytes(outside.join("x.csv")).await?;
    assert_eq!(outside_bytes, b"v1");
    let new_bytes = row("data/x.csv", b"v2").physical_key;
    assert_eq!(
        remote.get_object_count(&new_bytes),
        0,
        "refused before the download"
    );
    Ok(())
}

#[test(tokio::test)]
async fn a_key_renamed_only_in_case_keeps_its_file() -> Res {
    let remote = MockRemote::default();
    let storage = MockStorage::default();
    let entry = whole();
    publish(
        &remote,
        FIRST,
        &[("data.csv", b"rows"), ("sub/a.csv", b"a")],
    )
    .await?;
    read(&storage, &remote, &entry, None).await?;
    publish(
        &remote,
        SECOND,
        &[("Data.csv", b"rows"), ("Sub/a.csv", b"a")],
    )
    .await?;

    let outcome = read(&storage, &remote, &entry, Some(FIRST)).await?;

    assert_eq!(outcome.lock_base(), Some(SECOND));
    assert_eq!(text(&storage, "Data.csv").await.as_deref(), Some("rows"));
    assert_eq!(text(&storage, "Sub/a.csv").await.as_deref(), Some("a"));
    Ok(())
}

#[test(tokio::test)]
async fn on_a_disk_that_ignores_case_a_local_change_to_a_key_renamed_only_in_case_is_kept() -> Res {
    let remote = MockRemote::default();
    let storage = MockStorage::default();
    if !ignores_case(&storage).await? {
        return Ok(());
    }
    let entry = whole();
    publish(&remote, FIRST, &[("data.csv", b"v1")]).await?;
    read(&storage, &remote, &entry, None).await?;
    write(&storage, "data.csv", b"mine").await?;
    publish(&remote, SECOND, &[("Data.csv", b"v1")]).await?;

    let outcome = read(&storage, &remote, &entry, Some(FIRST)).await?;

    assert!(matches!(outcome, ReadOutcome::Read(_)), "{outcome:?}");
    assert_eq!(text(&storage, "data.csv").await.as_deref(), Some("mine"));
    Ok(())
}

#[test(tokio::test)]
async fn a_key_renamed_in_case_and_changed_takes_the_revisions_version() -> Res {
    let remote = MockRemote::default();
    let storage = MockStorage::default();
    let entry = whole();
    publish(&remote, FIRST, &[("data.csv", b"v1")]).await?;
    read(&storage, &remote, &entry, None).await?;
    publish(&remote, SECOND, &[("Data.csv", b"v2")]).await?;

    let outcome = read(&storage, &remote, &entry, Some(FIRST)).await?;

    assert_eq!(outcome.lock_base(), Some(SECOND));
    assert_eq!(text(&storage, "Data.csv").await.as_deref(), Some("v2"));
    Ok(())
}

#[test(tokio::test)]
async fn a_file_renamed_locally_to_the_revisions_spelling_is_kept() -> Res {
    let remote = MockRemote::default();
    let storage = MockStorage::default();
    let entry = whole();
    publish(&remote, FIRST, &[("a.txt", b"alpha")]).await?;
    read(&storage, &remote, &entry, None).await?;
    storage.remove_file(folder().join("a.txt")).await?;
    write(&storage, "A.txt", b"alpha").await?;
    publish(&remote, SECOND, &[("A.txt", b"alpha")]).await?;

    let outcome = read(&storage, &remote, &entry, Some(FIRST)).await?;

    assert_eq!(outcome.lock_base(), Some(SECOND));
    assert_eq!(text(&storage, "A.txt").await.as_deref(), Some("alpha"));
    Ok(())
}

#[test(tokio::test)]
async fn a_folder_at_the_metadata_files_place_collides() -> Res {
    let remote = MockRemote::default();
    let storage = MockStorage::default();
    publish(&remote, FIRST, &[("a.txt", b"alpha")]).await?;
    storage.create_dir_all(folder().join(METADATA_FILE)).await?;

    let outcome = read(&storage, &remote, &whole(), None).await;

    assert!(
        matches!(
            &outcome,
            Ok(ReadOutcome::Blocked { conflicts, .. }) if *conflicts == keys(&[METADATA_FILE])
        ),
        "{outcome:?}"
    );
    Ok(())
}

/// A rename would replace the link with a file.
#[cfg(unix)]
#[test(tokio::test)]
async fn a_link_where_a_file_goes_collides() -> Res {
    let remote = MockRemote::default();
    let storage = MockStorage::default();
    publish(&remote, FIRST, &[("a.txt", b"alpha")]).await?;
    storage.create_dir_all(folder()).await?;
    let link = storage.temp_dir.path().join("work/rnaseq/team/raw/a.txt");
    std::os::unix::fs::symlink(storage.temp_dir.path().join("nowhere"), &link)?;

    let outcome = read(&storage, &remote, &whole(), None).await;

    assert!(
        matches!(
            &outcome,
            Ok(ReadOutcome::Blocked { conflicts, .. }) if *conflicts == keys(&["a.txt"])
        ),
        "{outcome:?}"
    );
    assert!(storage.is_symlink(folder().join("a.txt")).await);
    Ok(())
}

#[test(tokio::test)]
async fn a_file_where_the_revision_needs_a_folder_collides() -> Res {
    let remote = MockRemote::default();
    let storage = MockStorage::default();
    publish(
        &remote,
        FIRST,
        &[("b.txt", b"beta"), ("results/a.csv", b"a")],
    )
    .await?;
    write(&storage, "results", b"mine").await?;

    let outcome = read(&storage, &remote, &whole(), None).await;

    assert!(
        matches!(
            &outcome,
            Ok(ReadOutcome::Blocked { conflicts, .. }) if *conflicts == keys(&["results/a.csv"])
        ),
        "{outcome:?}"
    );
    assert_eq!(text(&storage, "b.txt").await, None);
    Ok(())
}

#[test(tokio::test)]
async fn a_base_the_registry_no_longer_holds_is_read_as_a_first_read() -> Res {
    const GONE: &str = "3333333333333333333333333333333333333333333333333333333333333333";
    let remote = MockRemote::default();
    let storage = MockStorage::default();
    publish(&remote, SECOND, &[("a.txt", b"alpha"), ("b.txt", b"beta")]).await?;
    write(&storage, "a.txt", b"alpha").await?;

    let outcome = read(&storage, &remote, &whole(), Some(GONE)).await;

    assert!(outcome.is_ok(), "{outcome:?}");
    let outcome = outcome?;
    assert_eq!(outcome.lock_base(), Some(SECOND));
    assert_eq!(report(outcome).placed, keys(&["b.txt", METADATA_FILE]));
    Ok(())
}

#[test(tokio::test)]
async fn metadata_held_in_another_order_is_the_same_metadata() -> Res {
    let remote = MockRemote::default();
    let storage = MockStorage::default();
    let entry = whole();
    let with_meta = |value: serde_json::Value| ManifestHeader {
        user_meta: Some(value),
        ..ManifestHeader::default()
    };
    let files: &[(&str, &[u8])] = &[("a.txt", b"alpha")];
    publish_with(
        &remote,
        FIRST,
        files,
        with_meta(serde_json::json!({"x": 1, "y": 2})),
    )
    .await?;
    read(&storage, &remote, &entry, None).await?;
    write(&storage, METADATA_FILE, b"{\".\":{\"x\":\"mine\"}}\n").await?;
    publish_with(
        &remote,
        SECOND,
        files,
        with_meta(serde_json::json!({"y": 2, "x": 1})),
    )
    .await?;

    let outcome = read(&storage, &remote, &entry, Some(FIRST)).await?;

    assert_eq!(report(outcome).kept, keys(&[METADATA_FILE]));
    Ok(())
}

/// Two keys differing only in case are two files where the disk tells them
/// apart; there, dropping one deletes it.
#[test(tokio::test)]
async fn on_a_disk_that_tells_case_apart_a_key_dropped_in_one_case_is_removed() -> Res {
    let remote = MockRemote::default();
    let storage = MockStorage::default();
    if ignores_case(&storage).await? {
        return Ok(());
    }
    let entry = whole();
    publish(
        &remote,
        FIRST,
        &[("data.csv", b"rows"), ("Data.csv", b"rows")],
    )
    .await?;
    read(&storage, &remote, &entry, None).await?;
    publish(&remote, SECOND, &[("Data.csv", b"rows")]).await?;

    let outcome = read(&storage, &remote, &entry, Some(FIRST)).await?;

    assert_eq!(report(outcome).removed, keys(&["data.csv"]));
    assert_eq!(text(&storage, "data.csv").await, None);
    assert_eq!(text(&storage, "Data.csv").await.as_deref(), Some("rows"));
    Ok(())
}

/// A rename would replace the link with a file.
#[cfg(unix)]
#[test(tokio::test)]
async fn a_link_at_the_metadata_files_place_collides() -> Res {
    let remote = MockRemote::default();
    let storage = MockStorage::default();
    publish(&remote, FIRST, &[("a.txt", b"alpha")]).await?;
    storage.create_dir_all(folder()).await?;
    let root = storage.temp_dir.path();
    std::os::unix::fs::symlink(
        root.join("nowhere"),
        root.join("work/rnaseq/team/raw").join(METADATA_FILE),
    )?;

    let outcome = read(&storage, &remote, &whole(), None).await;

    assert!(
        matches!(
            &outcome,
            Ok(ReadOutcome::Blocked { conflicts, .. }) if *conflicts == keys(&[METADATA_FILE])
        ),
        "{outcome:?}"
    );
    assert!(storage.is_symlink(folder().join(METADATA_FILE)).await);
    Ok(())
}

#[cfg(unix)]
#[test(tokio::test)]
async fn a_link_put_in_a_files_place_while_the_revision_downloads_stops_the_read() -> Res {
    let remote = MockRemote::default();
    let storage = MockStorage::default();
    let entry = whole();
    publish(&remote, FIRST, &[("edit.txt", b"v1")]).await?;
    read(&storage, &remote, &entry, None).await?;
    publish(&remote, SECOND, &[("edit.txt", b"v2")]).await?;
    let root = storage.temp_dir.path().to_path_buf();
    storage
        .write_byte_stream(
            Path::new("/elsewhere/v1.txt"),
            ByteStream::from(b"v1".to_vec()),
        )
        .await?;
    let download = remote.park(&row("edit.txt", b"v2").physical_key);
    let swap = async {
        download.arrived().await;
        let target = root.join("work/rnaseq/team/raw/edit.txt");
        std::fs::remove_file(&target)?;
        std::os::unix::fs::symlink(root.join("elsewhere/v1.txt"), &target)?;
        download.release();
        Ok::<_, std::io::Error>(())
    };

    let (outcome, swapped) = tokio::join!(read(&storage, &remote, &entry, Some(FIRST)), swap);
    swapped?;

    assert!(
        matches!(&outcome, Err(EntryError::Changed(key)) if key == Path::new("edit.txt")),
        "{outcome:?}"
    );
    assert!(storage.is_symlink(folder().join("edit.txt")).await);
    Ok(())
}
