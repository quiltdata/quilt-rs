use super::*;

use std::sync::Arc;

use aws_sdk_s3::primitives::ByteStream;
use test_log::test;

use crate::dataspace::contract::Contract;
use crate::dataspace::contract::FolderCase;
use crate::io::remote::mocks::Gate;
use crate::io::remote::mocks::MockRemote;
use crate::io::storage::mocks::MockStorage;
use quilt_uri::ManifestUri;

type Res = Result<(), Box<dyn std::error::Error>>;

const DATASPACE: &str = "/work/rnaseq";
const LATEST: &str = "s3://b/.quilt/named_packages/team/raw/latest";
const FIRST: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const SECOND: &str = "2222222222222222222222222222222222222222222222222222222222222222";
const THIRD: &str = "3333333333333333333333333333333333333333333333333333333333333333";
const FOURTH: &str = "4444444444444444444444444444444444444444444444444444444444444444";

fn entry(packages: &str) -> Entry {
    let text = format!("registry = \"s3://b\"\n[packages]\n{packages}");
    let contract = Contract::parse(text.as_bytes(), Path::new(DATASPACE), FolderCase::Sensitive)
        .expect("contract");
    contract.entries()[0].clone()
}

fn folder() -> PathBuf {
    Path::new(DATASPACE).join("team/raw")
}

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
async fn seed(
    remote: &MockRemote,
    hash: &str,
    files: &[(&str, &[u8])],
    header: ManifestHeader,
) -> Result<(), crate::Error> {
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
    seed_manifest(remote, hash, &manifest).await
}

async fn seed_manifest(
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
        .await?;
    remote
        .put_object(None, &S3Uri::try_from(LATEST)?, hash.as_bytes().to_vec())
        .await
}

fn with_meta(value: serde_json::Value) -> ManifestHeader {
    ManifestHeader {
        user_meta: Some(value),
        ..ManifestHeader::default()
    }
}

async fn write(storage: &MockStorage, key: &str, body: &[u8]) -> Result<(), crate::Error> {
    storage
        .write_byte_stream(folder().join(key), ByteStream::from(body.to_vec()))
        .await
}

async fn latest_hash(remote: &MockRemote) -> Result<String, crate::Error> {
    let bytes = remote.current(LATEST).await?.expect("a latest tag");
    Ok(String::from_utf8(bytes).expect("utf-8"))
}

async fn revision(remote: &MockRemote, hash: &str) -> Result<Manifest, crate::Error> {
    let uri = ManifestUri {
        hash: hash.to_string(),
        bucket: "b".to_string(),
        namespace: ("team", "raw").into(),
        origin: None,
    };
    fetch_jsonl(remote, &uri).await
}

fn keys(manifest: &Manifest) -> Vec<String> {
    let mut keys: Vec<String> = manifest
        .rows
        .iter()
        .map(|row| row.logical_key.to_string())
        .collect();
    keys.sort();
    keys
}

fn row_of<'a>(manifest: &'a Manifest, key: &str) -> &'a ManifestRow {
    manifest
        .rows
        .iter()
        .find(|row| row.logical_key.as_path() == Path::new(key))
        .expect("a row for the key")
}

fn advanced(outcome: &Published) -> &str {
    match outcome {
        Published::Advanced { revision, .. } => revision,
        other => panic!("expected latest to advance, got {other:?}"),
    }
}

/// Waits for the access `gate` holds, then lets it go and holds the next.
async fn next(remote: &MockRemote, gate: Arc<Gate>) -> Arc<Gate> {
    gate.arrived().await;
    let following = remote.park(LATEST);
    gate.release();
    following
}

/// Runs `publishing` beside `rival`, which is dropped where the publish ends
/// first, so a publish that stops short fails its test's assertions rather
/// than leaving the rival waiting.
async fn beside(
    publishing: impl Future<Output = Result<Published, EntryError>>,
    rival: impl Future<Output = Result<(), crate::Error>>,
) -> Result<Result<Published, EntryError>, crate::Error> {
    let mut publishing = std::pin::pin!(publishing);
    let mut rival = std::pin::pin!(rival);
    Ok(tokio::select! {
        outcome = &mut publishing => outcome,
        rivalled = &mut rival => {
            rivalled?;
            publishing.await
        }
    })
}

/// Runs `publishing` while another writer tags `rivals` holding
/// `rival_files`, one each time the publish reads `latest` to tag its own
/// revision.
async fn race(
    remote: &MockRemote,
    rivals: &[&str],
    rival_files: &[(&str, &[u8])],
    publishing: impl Future<Output = Result<Published, EntryError>>,
) -> Result<Published, Box<dyn std::error::Error>> {
    let mut gate = remote.park(LATEST);
    let compete = async move {
        let mut rivals = rivals.iter().peekable();
        while let Some(rival) = rivals.next() {
            // Past the publish's own read, to the read before its tag write.
            let tagging = next(remote, gate).await;
            tagging.arrived().await;
            seed(remote, rival, rival_files, ManifestHeader::default()).await?;
            if rivals.peek().is_none() {
                tagging.release();
                break;
            }
            gate = remote.park(LATEST);
            tagging.release();
        }
        Ok::<_, crate::Error>(())
    };
    Ok(beside(publishing, compete).await??)
}

#[test(tokio::test)]
async fn a_write_lays_its_files_over_latest_and_keeps_every_other_key() -> Res {
    let remote = MockRemote::default();
    let storage = &remote.storage;
    seed(
        &remote,
        FIRST,
        &[("a.txt", b"a"), ("keep.txt", b"k")],
        with_meta(serde_json::json!({"v": 1})),
    )
    .await?;
    write(storage, "a.txt", b"changed").await?;
    write(storage, "new.txt", b"new").await?;

    let outcome = publish(
        storage,
        &remote,
        &entry("write = [\"team/raw\"]"),
        Some("more"),
    )
    .await?;

    let new = advanced(&outcome).to_string();
    assert_eq!(outcome.lock_base(), Some(new.as_str()));
    assert_eq!(latest_hash(&remote).await?, new);
    let published = revision(&remote, &new).await?;
    assert_eq!(keys(&published), ["a.txt", "keep.txt", "new.txt"]);
    let changed = row_of(&published, "a.txt");
    assert_eq!(
        (&changed.hash, changed.size),
        (
            &crate::io::manifest::hash_bytes(b"changed", changed.hash.algorithm()).await?,
            7
        )
    );
    assert!(row_of(&published, "keep.txt").matches_content(&row("keep.txt", b"k")));
    assert_eq!(
        published.header.user_meta,
        Some(serde_json::json!({"v": 1})),
        "no metadata file keeps it"
    );
    Ok(())
}

#[test(tokio::test)]
async fn a_write_publishes_only_the_files_under_its_paths() -> Res {
    let remote = MockRemote::default();
    let storage = &remote.storage;
    seed(
        &remote,
        FIRST,
        &[("a.txt", b"a")],
        ManifestHeader::default(),
    )
    .await?;
    write(storage, "src/x.txt", b"x").await?;
    write(storage, "other/y.txt", b"y").await?;

    let outcome = publish(
        storage,
        &remote,
        &entry("write = [{ package = \"team/raw\", path = \"src/\" }]"),
        None,
    )
    .await?;

    let published = revision(&remote, advanced(&outcome)).await?;
    assert_eq!(keys(&published), ["a.txt", "src/x.txt"]);
    Ok(())
}

#[test(tokio::test)]
async fn a_replace_removes_every_key_the_folder_lacks() -> Res {
    let remote = MockRemote::default();
    let storage = &remote.storage;
    seed(
        &remote,
        FIRST,
        &[("a.txt", b"a"), ("gone.txt", b"g")],
        ManifestHeader::default(),
    )
    .await?;
    write(storage, "a.txt", b"a").await?;
    write(storage, "new.txt", b"new").await?;

    let outcome = publish(storage, &remote, &entry("replace = [\"team/raw\"]"), None).await?;

    let published = revision(&remote, advanced(&outcome)).await?;
    assert_eq!(keys(&published), ["a.txt", "new.txt"]);
    Ok(())
}

#[test(tokio::test)]
async fn a_replace_that_turns_a_folder_into_a_file_drops_the_folders_metadata() -> Res {
    let remote = MockRemote::default();
    let storage = &remote.storage;
    seed(
        &remote,
        FIRST,
        &[("results/a.csv", b"a"), ("keep/b.csv", b"b")],
        ManifestHeader::default(),
    )
    .await?;
    let folder_meta = |key: &str| crate::manifest::DirMeta {
        logical_key: LogicalKey::new(key).expect("key"),
        meta: serde_json::json!({"kind": key}),
        meta_numbers: RawNumbers::default(),
    };
    let manifest = Manifest {
        rows: vec![row("results/a.csv", b"a"), row("keep/b.csv", b"b")],
        dir_meta: vec![folder_meta("results"), folder_meta("keep")],
        ..Manifest::default()
    };
    seed_manifest(&remote, FIRST, &manifest).await?;
    write(storage, "results", b"now a file").await?;
    write(storage, "keep/b.csv", b"b").await?;

    let outcome = publish(storage, &remote, &entry("replace = [\"team/raw\"]"), None).await;

    let Ok(Published::Advanced { revision: new, .. }) = &outcome else {
        panic!("expected the snapshot published, got {outcome:?}");
    };
    let published = revision(&remote, new).await?;
    assert_eq!(keys(&published), ["keep/b.csv", "results"]);
    assert_eq!(published.dir_meta, [folder_meta("keep")]);
    Ok(())
}

#[test(tokio::test)]
async fn a_package_with_no_latest_is_published_as_its_first_revision() -> Res {
    let remote = MockRemote::default();
    let storage = &remote.storage;
    write(storage, "a.txt", b"a").await?;

    let outcome = publish(storage, &remote, &entry("write = [\"team/raw\"]"), None).await;

    let Ok(Published::Advanced { revision: new, .. }) = &outcome else {
        panic!("expected a first revision, got {outcome:?}");
    };
    assert_eq!(&latest_hash(&remote).await?, new);
    assert_eq!(keys(&revision(&remote, new).await?), ["a.txt"]);
    Ok(())
}

#[test(tokio::test)]
async fn a_first_revision_may_hold_only_metadata() -> Res {
    let remote = MockRemote::default();
    let storage = &remote.storage;
    write(storage, METADATA_FILE, br#"{".": {"v": 1}}"#).await?;

    let outcome = publish(storage, &remote, &entry("write = [\"team/raw\"]"), None).await;

    let Ok(Published::Advanced { revision: new, .. }) = &outcome else {
        panic!("expected a first revision, got {outcome:?}");
    };
    let published = revision(&remote, new).await?;
    assert_eq!(
        published.header.user_meta,
        Some(serde_json::json!({"v": 1}))
    );
    assert_eq!(keys(&published), [""; 0]);
    Ok(())
}

#[test(tokio::test)]
async fn an_empty_write_to_a_package_with_no_latest_publishes_nothing() -> Res {
    let remote = MockRemote::default();
    let storage = &remote.storage;
    storage.create_dir_all(folder()).await?;

    let outcome = publish(storage, &remote, &entry("write = [\"team/raw\"]"), None).await;

    assert!(
        matches!(outcome, Ok(Published::Unchanged { latest: None })),
        "{outcome:?}"
    );
    assert_eq!(remote.current(LATEST).await?, None);
    Ok(())
}

#[test(tokio::test)]
async fn a_replace_whose_only_change_drops_a_folders_metadata_publishes_it() -> Res {
    let remote = MockRemote::default();
    let storage = &remote.storage;
    seed(
        &remote,
        FIRST,
        &[("a.txt", b"a")],
        ManifestHeader::default(),
    )
    .await?;
    let manifest = Manifest {
        rows: vec![row("a.txt", b"a")],
        dir_meta: vec![crate::manifest::DirMeta {
            logical_key: LogicalKey::new("old").expect("key"),
            meta: serde_json::json!({"kind": "old"}),
            meta_numbers: RawNumbers::default(),
        }],
        ..Manifest::default()
    };
    seed_manifest(&remote, FIRST, &manifest).await?;
    write(storage, "a.txt", b"a").await?;

    let outcome = publish(storage, &remote, &entry("replace = [\"team/raw\"]"), None).await;

    let Ok(Published::Advanced { revision: new, .. }) = &outcome else {
        panic!("expected the snapshot published, got {outcome:?}");
    };
    assert_eq!(revision(&remote, new).await?.dir_meta, []);
    Ok(())
}

#[test(tokio::test)]
async fn a_folder_holding_nothing_new_publishes_nothing() -> Res {
    let remote = MockRemote::default();
    let storage = &remote.storage;
    seed(
        &remote,
        FIRST,
        &[("a.txt", b"a"), ("b.txt", b"b")],
        ManifestHeader::default(),
    )
    .await?;
    write(storage, "a.txt", b"a").await?;

    let outcome = publish(storage, &remote, &entry("write = [\"team/raw\"]"), None).await?;

    assert_eq!(
        outcome,
        Published::Unchanged {
            latest: Some(FIRST.to_string())
        }
    );
    assert_eq!(outcome.lock_base(), None);
    assert_eq!(latest_hash(&remote).await?, FIRST);
    Ok(())
}

#[test(tokio::test)]
async fn a_write_racing_another_writer_keeps_the_other_writers_additions() -> Res {
    let remote = MockRemote::default();
    let storage = &remote.storage;
    seed(
        &remote,
        FIRST,
        &[("a.txt", b"a")],
        ManifestHeader::default(),
    )
    .await?;
    write(storage, "mine.txt", b"m").await?;
    let entry = entry("write = [\"team/raw\"]");

    let outcome = race(
        &remote,
        &[SECOND],
        &[("theirs.txt", b"t")],
        Box::pin(publish(storage, &remote, &entry, None)),
    )
    .await?;

    let new = advanced(&outcome).to_string();
    assert_eq!(latest_hash(&remote).await?, new);
    assert_eq!(
        keys(&revision(&remote, &new).await?),
        ["mine.txt", "theirs.txt"]
    );
    Ok(())
}

#[test(tokio::test)]
async fn a_write_that_keeps_losing_the_race_is_stored_and_reported_with_its_hash() -> Res {
    let remote = MockRemote::default();
    let storage = &remote.storage;
    seed(
        &remote,
        FIRST,
        &[("a.txt", b"a")],
        ManifestHeader::default(),
    )
    .await?;
    write(storage, "mine.txt", b"m").await?;
    let entry = entry("write = [\"team/raw\"]");

    let outcome = race(
        &remote,
        &[SECOND, THIRD, FOURTH],
        &[("theirs.txt", b"t")],
        Box::pin(publish(storage, &remote, &entry, None)),
    )
    .await?;

    let Published::Stored {
        revision: stored,
        latest,
    } = &outcome
    else {
        panic!("expected a stored revision, got {outcome:?}");
    };
    assert_eq!(latest.as_deref(), Some(FOURTH));
    assert_eq!(latest_hash(&remote).await?, FOURTH);
    assert_eq!(
        keys(&revision(&remote, stored).await?),
        ["mine.txt", "theirs.txt"],
        "composed on the last latest it read"
    );
    Ok(())
}

#[test(tokio::test)]
async fn a_replace_racing_another_writer_is_stored_and_reported_with_its_hash() -> Res {
    let remote = MockRemote::default();
    let storage = &remote.storage;
    seed(
        &remote,
        FIRST,
        &[("a.txt", b"a")],
        ManifestHeader::default(),
    )
    .await?;
    write(storage, "mine.txt", b"m").await?;
    let entry = entry("replace = [\"team/raw\"]");

    let outcome = race(
        &remote,
        &[SECOND],
        &[("theirs.txt", b"t")],
        Box::pin(publish(storage, &remote, &entry, None)),
    )
    .await?;

    let Published::Stored {
        revision: stored,
        latest,
    } = &outcome
    else {
        panic!("expected a stored revision, got {outcome:?}");
    };
    assert_eq!(latest.as_deref(), Some(SECOND));
    assert_eq!(outcome.lock_base(), None);
    assert_eq!(latest_hash(&remote).await?, SECOND);
    assert_eq!(keys(&revision(&remote, stored).await?), ["mine.txt"]);
    Ok(())
}

/// The rival's `latest` already holds the write's file, so the second
/// attempt finds nothing to publish; the first attempt's revision is still
/// reported.
#[test(tokio::test)]
async fn a_write_whose_retry_finds_nothing_new_reports_the_revision_it_stored() -> Res {
    let remote = MockRemote::default();
    let storage = &remote.storage;
    seed(
        &remote,
        FIRST,
        &[("a.txt", b"a")],
        ManifestHeader::default(),
    )
    .await?;
    write(storage, "mine.txt", b"m").await?;
    let entry = entry("write = [\"team/raw\"]");

    let outcome = race(
        &remote,
        &[SECOND],
        &[("mine.txt", b"m"), ("theirs.txt", b"t")],
        Box::pin(publish(storage, &remote, &entry, None)),
    )
    .await?;

    let Published::Stored {
        revision: stored,
        latest,
    } = &outcome
    else {
        panic!("expected a stored revision, got {outcome:?}");
    };
    assert_eq!(latest.as_deref(), Some(SECOND));
    assert_eq!(
        keys(&revision(&remote, stored).await?),
        ["a.txt", "mine.txt"]
    );
    Ok(())
}

/// The rival tags a revision whose manifest cannot be fetched, so the second
/// attempt fails; the first attempt's revision is still reported.
#[test(tokio::test)]
async fn a_write_whose_retry_fails_reports_the_revision_it_stored() -> Res {
    let remote = MockRemote::default();
    let storage = &remote.storage;
    seed(
        &remote,
        FIRST,
        &[("a.txt", b"a")],
        ManifestHeader::default(),
    )
    .await?;
    write(storage, "mine.txt", b"m").await?;
    let entry = entry("write = [\"team/raw\"]");

    let gate = remote.park(LATEST);
    let rival = async {
        let tagging = next(&remote, gate).await;
        tagging.arrived().await;
        remote
            .put_object(None, &S3Uri::try_from(LATEST)?, SECOND.as_bytes().to_vec())
            .await?;
        tagging.release();
        Ok(())
    };
    let outcome = beside(Box::pin(publish(storage, &remote, &entry, None)), rival).await?;

    let Ok(Published::Stored {
        revision: stored,
        latest,
    }) = &outcome
    else {
        panic!("expected a stored revision, got {outcome:?}");
    };
    assert_eq!(latest.as_deref(), Some(SECOND));
    assert_eq!(
        keys(&revision(&remote, stored).await?),
        ["a.txt", "mine.txt"]
    );
    Ok(())
}

/// Refuses the publish's conditional write of `latest` and fails the read
/// after it, so the push cannot say where `latest` went, then runs `reread`
/// before the publish reads `latest` again.
fn unclear_tag_write<'a>(
    remote: &'a MockRemote,
    reread: impl Future<Output = Result<(), crate::Error>> + 'a,
) -> impl Future<Output = Result<(), crate::Error>> + 'a {
    let gate = remote.park(LATEST);
    async move {
        let gate = next(remote, gate).await;
        let conditional = next(remote, gate).await;
        conditional.arrived().await;
        remote
            .put_object(None, &S3Uri::try_from(LATEST)?, b"unreadable".to_vec())
            .await?;
        let after_refusal = remote.park(LATEST);
        conditional.release();
        let again = next(remote, after_refusal).await;
        again.arrived().await;
        reread.await?;
        again.release();
        Ok(())
    }
}

#[test(tokio::test)]
async fn a_publish_that_cannot_tell_where_latest_went_reads_it_again() -> Res {
    let remote = MockRemote::default();
    let storage = &remote.storage;
    seed(
        &remote,
        FIRST,
        &[("a.txt", b"a")],
        ManifestHeader::default(),
    )
    .await?;
    write(storage, "mine.txt", b"m").await?;
    let entry = entry("replace = [\"team/raw\"]");

    let rival = unclear_tag_write(
        &remote,
        seed(
            &remote,
            SECOND,
            &[("theirs.txt", b"t")],
            ManifestHeader::default(),
        ),
    );
    let outcome = beside(Box::pin(publish(storage, &remote, &entry, None)), rival).await?;

    let Ok(Published::Stored {
        revision: stored,
        latest,
    }) = &outcome
    else {
        panic!("expected a stored revision, got {outcome:?}");
    };
    assert_eq!(latest.as_deref(), Some(SECOND));
    assert_eq!(keys(&revision(&remote, stored).await?), ["mine.txt"]);
    Ok(())
}

/// Read again, `latest` cannot be read, so where the write went is unknown.
#[test(tokio::test)]
async fn a_tag_write_that_failed_with_latest_unreadable_after_is_an_error() -> Res {
    let remote = MockRemote::default();
    let storage = &remote.storage;
    seed(
        &remote,
        FIRST,
        &[("a.txt", b"a")],
        ManifestHeader::default(),
    )
    .await?;
    write(storage, "mine.txt", b"m").await?;
    let entry = entry("write = [\"team/raw\"]");

    let rival = unclear_tag_write(&remote, async {
        remote.deny_reads(LATEST);
        Ok(())
    });
    let outcome = beside(Box::pin(publish(storage, &remote, &entry, None)), rival).await?;

    assert!(
        matches!(
            &outcome,
            Err(EntryError::Failed(err)) if matches!(
                **err,
                Error::PackageOp(PackageOpError::PushTagFailed { .. })
            )
        ),
        "{outcome:?}"
    );
    Ok(())
}

/// Read again, `latest` still names the publish's base: nothing raced it, so
/// the failed tag write is the publish's own error, not a lost race.
#[test(tokio::test)]
async fn a_tag_write_that_failed_with_latest_unmoved_is_an_error() -> Res {
    let remote = MockRemote::default();
    let storage = &remote.storage;
    seed(
        &remote,
        FIRST,
        &[("a.txt", b"a")],
        ManifestHeader::default(),
    )
    .await?;
    write(storage, "mine.txt", b"m").await?;
    let entry = entry("write = [\"team/raw\"]");

    let rival = unclear_tag_write(&remote, async {
        remote
            .put_object(None, &S3Uri::try_from(LATEST)?, FIRST.as_bytes().to_vec())
            .await
    });
    let outcome = beside(Box::pin(publish(storage, &remote, &entry, None)), rival).await?;

    assert!(
        matches!(
            &outcome,
            Err(EntryError::Failed(err)) if matches!(
                **err,
                Error::PackageOp(PackageOpError::PushTagFailed { .. })
            )
        ),
        "{outcome:?}"
    );
    assert_eq!(latest_hash(&remote).await?, FIRST);
    Ok(())
}

#[test(tokio::test)]
async fn the_metadata_file_sets_the_package_and_entry_metadata() -> Res {
    let remote = MockRemote::default();
    let storage = &remote.storage;
    seed(
        &remote,
        FIRST,
        &[("a.txt", b"a")],
        with_meta(serde_json::json!({"v": 1})),
    )
    .await?;
    write(storage, "a.txt", b"a").await?;
    write(
        storage,
        METADATA_FILE,
        br#"{".": {"v": 2, "big": 123456789012345678901234567890}, "a.txt": {"big": 98765432109876543210987654321}}"#,
    )
    .await?;

    let outcome = publish(storage, &remote, &entry("write = [\"team/raw\"]"), None).await?;

    let published = revision(&remote, advanced(&outcome)).await?;
    assert_eq!(
        published.header.user_meta_text().as_deref(),
        Some(r#"{"big":123456789012345678901234567890,"v":2}"#),
        "integers beyond 64 bits keep their digits"
    );
    let (meta, meta_numbers) =
        RawNumbers::parse(r#"{"user_meta": {"big": 98765432109876543210987654321}}"#)?;
    let expected = ManifestRow {
        meta: Some(meta),
        meta_numbers,
        ..row("a.txt", b"a")
    };
    assert!(
        row_of(&published, "a.txt").same_meta(&expected),
        "{:?}",
        row_of(&published, "a.txt").meta
    );
    assert_eq!(
        keys(&published),
        ["a.txt"],
        "the metadata file is not a file of the package"
    );
    Ok(())
}

/// What the metadata file leaves out is kept under `write`, removed under
/// `replace`; an empty file clears what it covers.
#[test(tokio::test)]
async fn metadata_the_file_leaves_out_is_kept_by_write_and_dropped_by_replace() -> Res {
    for (access, file, kept) in [
        ("write", &br#"{"a.txt": {"k": 2}}"#[..], true),
        ("replace", &br#"{"a.txt": {"k": 2}}"#[..], false),
        ("write", &b""[..], false),
    ] {
        let remote = MockRemote::default();
        let storage = &remote.storage;
        let mut manifest = Manifest {
            header: with_meta(serde_json::json!({"v": 1})),
            rows: vec![row("a.txt", b"a"), row("b.txt", b"b")],
            ..Manifest::default()
        };
        manifest.rows[1].meta = Some(serde_json::json!({"user_meta": {"k": 1}}));
        seed(
            &remote,
            FIRST,
            &[("a.txt", b"a"), ("b.txt", b"b")],
            ManifestHeader::default(),
        )
        .await?;
        seed_manifest(&remote, FIRST, &manifest).await?;
        write(storage, "a.txt", b"a").await?;
        write(storage, "b.txt", b"b").await?;
        write(storage, METADATA_FILE, file).await?;

        let outcome = publish(
            storage,
            &remote,
            &entry(&format!("{access} = [\"team/raw\"]")),
            None,
        )
        .await?;

        let published = revision(&remote, advanced(&outcome)).await?;
        let b_meta = row_of(&published, "b.txt")
            .meta
            .as_ref()
            .and_then(|meta| meta.get("user_meta").cloned());
        let package_meta = published.header.user_meta.clone();
        if kept {
            assert_eq!(b_meta, Some(serde_json::json!({"k": 1})), "{access}");
            assert_eq!(package_meta, Some(serde_json::json!({"v": 1})), "{access}");
        } else {
            assert_eq!(b_meta, None, "{access}");
            assert_eq!(package_meta, None, "{access}");
        }
    }
    Ok(())
}

/// A `meta` that is not an object has no `user_meta` the metadata file could
/// clear.
#[test(tokio::test)]
async fn metadata_that_is_not_an_object_survives_a_replace_that_leaves_it_out() -> Res {
    let remote = MockRemote::default();
    let storage = &remote.storage;
    seed(
        &remote,
        FIRST,
        &[("a.txt", b"a"), ("b.txt", b"b")],
        ManifestHeader::default(),
    )
    .await?;
    let mut manifest = Manifest {
        rows: vec![row("a.txt", b"a"), row("b.txt", b"b")],
        ..Manifest::default()
    };
    manifest.rows[1].meta = Some(serde_json::json!("legacy annotation"));
    seed_manifest(&remote, FIRST, &manifest).await?;
    write(storage, "a.txt", b"a").await?;
    write(storage, "b.txt", b"b").await?;
    write(storage, METADATA_FILE, br#"{"a.txt": {"k": 2}}"#).await?;

    let outcome = publish(storage, &remote, &entry("replace = [\"team/raw\"]"), None).await?;

    let published = revision(&remote, advanced(&outcome)).await?;
    assert_eq!(
        row_of(&published, "b.txt").meta,
        Some(serde_json::json!("legacy annotation"))
    );
    Ok(())
}

#[test(tokio::test)]
async fn a_file_named_like_the_metadata_file_in_another_case_is_not_published() -> Res {
    let remote = MockRemote::default();
    let storage = &remote.storage;
    write(storage, "a.txt", b"a").await?;
    write(storage, "Package-Metadata.json", b"{}").await?;

    let outcome = publish(storage, &remote, &entry("write = [\"team/raw\"]"), None).await?;

    assert_eq!(
        keys(&revision(&remote, advanced(&outcome)).await?),
        ["a.txt"]
    );
    Ok(())
}

/// A read refuses a package with such a key, reading `\` as a separator,
/// so publishing one would leave a revision no dataspace could use. Only on
/// Unix is `\` part of a name; on Windows the path is a folder, as below.
#[cfg(unix)]
#[test(tokio::test)]
async fn a_file_inside_the_metadata_files_name_after_a_backslash_is_not_published() -> Res {
    let remote = MockRemote::default();
    let storage = &remote.storage;
    write(storage, "a.txt", b"a").await?;
    write(storage, "package-metadata.json\\inside.txt", b"x").await?;

    let outcome = publish(storage, &remote, &entry("write = [\"team/raw\"]"), None).await?;

    assert_eq!(
        keys(&revision(&remote, advanced(&outcome)).await?),
        ["a.txt"]
    );
    Ok(())
}

/// A read collides on a folder at the metadata file's place, so a publish
/// refuses it rather than read it as the file or publish what it holds.
#[test(tokio::test)]
async fn a_folder_at_the_metadata_files_place_is_refused() -> Res {
    let remote = MockRemote::default();
    let storage = &remote.storage;
    write(storage, "a.txt", b"a").await?;
    write(storage, "package-metadata.json/inside.txt", b"x").await?;

    let outcome = publish(storage, &remote, &entry("write = [\"team/raw\"]"), None).await;

    assert!(
        matches!(outcome, Err(EntryError::MetadataFile(_))),
        "{outcome:?}"
    );
    assert_eq!(remote.current(LATEST).await?, None);
    Ok(())
}

#[test(tokio::test)]
async fn what_an_interrupted_read_leaves_is_not_published() -> Res {
    let remote = MockRemote::default();
    let storage = &remote.storage;
    write(storage, "a.txt", b"a").await?;
    write(storage, ".quilt-read-0123/a.txt", b"staged").await?;
    write(storage, "package-metadata.json.tmp-0123", b"{}").await?;
    write(
        storage,
        ".tmp-67e55044-10b1-426f-9247-bb680e5fe0c8",
        b"{\"partial",
    )
    .await?;

    let outcome = publish(storage, &remote, &entry("replace = [\"team/raw\"]"), None).await?;

    assert_eq!(
        keys(&revision(&remote, advanced(&outcome)).await?),
        ["a.txt"]
    );
    Ok(())
}

#[test(tokio::test)]
async fn an_entry_or_folder_that_cannot_be_published_is_refused() -> Res {
    let remote = MockRemote::default();
    let storage = &remote.storage;
    seed(
        &remote,
        FIRST,
        &[("a.txt", b"a")],
        ManifestHeader::default(),
    )
    .await?;

    let read = publish(storage, &remote, &entry("read = [\"team/raw\"]"), None).await;
    assert!(
        matches!(read, Err(EntryError::Access(Access::Read))),
        "{read:?}"
    );

    let missing = publish(storage, &remote, &entry("write = [\"team/raw\"]"), None).await;
    assert!(
        matches!(missing, Err(EntryError::FolderUnavailable(_))),
        "{missing:?}"
    );

    storage.create_dir_all(folder()).await?;
    let empty = publish(storage, &remote, &entry("replace = [\"team/raw\"]"), None).await;
    assert!(
        matches!(empty, Err(EntryError::NothingToReplace(_))),
        "{empty:?}"
    );

    write(storage, "a.txt", b"a2").await?;
    write(storage, METADATA_FILE, b"[1, 2]").await?;
    let invalid = publish(storage, &remote, &entry("write = [\"team/raw\"]"), None).await;
    assert!(
        matches!(invalid, Err(EntryError::MetadataFile(_))),
        "{invalid:?}"
    );

    assert_eq!(latest_hash(&remote).await?, FIRST, "nothing was published");
    Ok(())
}

#[test(tokio::test)]
async fn latest_holding_a_file_named_for_the_metadata_is_refused() -> Res {
    let remote = MockRemote::default();
    let storage = &remote.storage;
    seed(
        &remote,
        FIRST,
        &[("a.txt", b"a"), (METADATA_FILE, b"{}")],
        ManifestHeader::default(),
    )
    .await?;
    write(storage, "a.txt", b"a2").await?;

    let outcome = publish(storage, &remote, &entry("write = [\"team/raw\"]"), None).await;

    assert!(
        matches!(outcome, Err(EntryError::HoldsMetadataFile)),
        "{outcome:?}"
    );
    assert_eq!(latest_hash(&remote).await?, FIRST);
    Ok(())
}

/// A row of `latest` that names a local file would have that file uploaded
/// from this machine, wherever it is.
#[test(tokio::test)]
async fn latest_naming_a_local_file_is_refused() -> Res {
    let remote = MockRemote::default();
    let storage = &remote.storage;
    storage
        .write_byte_stream("/outside/secret.txt", ByteStream::from_static(b"s"))
        .await?;
    let local = ManifestRow {
        physical_key: "file:///outside/secret.txt".to_string(),
        ..row("secret.txt", b"s")
    };
    let manifest = Manifest {
        rows: vec![row("a.txt", b"a"), local],
        ..Manifest::default()
    };
    seed_manifest(&remote, FIRST, &manifest).await?;
    write(storage, "mine.txt", b"m").await?;

    let outcome = publish(storage, &remote, &entry("write = [\"team/raw\"]"), None).await;

    assert!(
        matches!(
            &outcome,
            Err(EntryError::Failed(err)) if matches!(
                &**err,
                Error::PackageOp(PackageOpError::Push(message)) if message.contains("not in S3")
            )
        ),
        "{outcome:?}"
    );
    assert_eq!(latest_hash(&remote).await?, FIRST);

    // A snapshot without the row uploads nothing it names.
    let replaced = publish(storage, &remote, &entry("replace = [\"team/raw\"]"), None).await;
    let Ok(Published::Advanced { revision: new, .. }) = &replaced else {
        panic!("expected the snapshot published, got {replaced:?}");
    };
    assert_eq!(keys(&revision(&remote, new).await?), ["mine.txt"]);
    Ok(())
}

#[test(tokio::test)]
async fn a_write_that_cannot_read_latest_is_refused() -> Res {
    let remote = MockRemote::default();
    let storage = &remote.storage;
    seed(
        &remote,
        FIRST,
        &[("a.txt", b"a")],
        ManifestHeader::default(),
    )
    .await?;
    remote.deny_reads(LATEST);
    write(storage, "b.txt", b"b").await?;

    let outcome = publish(storage, &remote, &entry("write = [\"team/raw\"]"), None).await;

    assert!(
        matches!(outcome, Err(EntryError::LatestUnreadable)),
        "{outcome:?}"
    );
    Ok(())
}

#[test(tokio::test)]
async fn a_replace_that_cannot_read_latest_writes_it_unchecked() -> Res {
    let remote = MockRemote::default();
    let storage = &remote.storage;
    seed(
        &remote,
        FIRST,
        &[("a.txt", b"a")],
        ManifestHeader::default(),
    )
    .await?;
    remote.deny_reads(LATEST);
    write(storage, "b.txt", b"b").await?;

    let outcome = publish(storage, &remote, &entry("replace = [\"team/raw\"]"), None).await;

    let Ok(Published::Advanced {
        revision: new,
        latest_unchecked: true,
    }) = &outcome
    else {
        panic!("expected latest written unchecked, got {outcome:?}");
    };
    assert_eq!(&latest_hash(&remote).await?, new);
    assert_eq!(keys(&revision(&remote, new).await?), ["b.txt"]);
    Ok(())
}

#[test(tokio::test)]
async fn a_publish_takes_the_buckets_default_workflow() -> Res {
    let remote = MockRemote::default();
    let storage = &remote.storage;
    remote
        .put_object(
            None,
            &S3Uri::try_from("s3://b/.quilt/workflows/config.yml")?,
            b"version: '1'\ndefault_workflow: gate\nworkflows:\n  gate:\n    name: Gate\n".to_vec(),
        )
        .await?;
    write(storage, "a.txt", b"a").await?;

    let outcome = publish(storage, &remote, &entry("write = [\"team/raw\"]"), None).await;

    let Ok(published) = &outcome else {
        panic!("expected a publish under the default workflow, got {outcome:?}");
    };
    let workflow = revision(&remote, advanced(published))
        .await?
        .header
        .workflow;
    assert_eq!(
        workflow.and_then(|workflow| workflow.id).map(|id| id.id),
        Some("gate".to_string())
    );
    Ok(())
}
