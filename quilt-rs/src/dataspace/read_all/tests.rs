use super::*;

use std::path::PathBuf;

use test_log::test;

use crate::LogicalKey;
use crate::dataspace::contract::PackageKey;
use crate::dataspace::contract::Registry;
use crate::dataspace::lock::LOCK_FILE;
use crate::dataspace::read::ReadReport;
use crate::io::remote::mocks::MockRemote;
use crate::io::storage::mocks::MockStorage;
use crate::manifest::Manifest;
use crate::manifest::ManifestRow;
use crate::manifest::RawNumbers;
use quilt_uri::Namespace;
use quilt_uri::S3Uri;

type Res = Result<(), Box<dyn std::error::Error>>;

const FIRST: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const SECOND: &str = "2222222222222222222222222222222222222222222222222222222222222222";
// A bucket stores a manifest by its hash alone, so each package has its own.
const REF_FIRST: &str = "3333333333333333333333333333333333333333333333333333333333333333";
const REF_SECOND: &str = "4444444444444444444444444444444444444444444444444444444444444444";

fn writer() -> Writer {
    Writer {
        host: "here".to_string(),
        user: "me".to_string(),
        local_registry: String::new(),
    }
}

fn key(package: &str) -> PackageKey {
    PackageKey {
        registry: Registry::S3("b".to_string()),
        package: Namespace::try_from(package).expect("package"),
    }
}

fn row(package: &str, key: &str, body: &[u8]) -> ManifestRow {
    use sha2::Digest;
    let hash =
        multihash::Multihash::<256>::wrap(0x12, &sha2::Sha256::digest(body)).expect("multihash");
    ManifestRow {
        logical_key: LogicalKey::new(key).expect("key"),
        physical_key: format!("s3://b/{package}/{key}?versionId={}", hex::encode(body)),
        hash: hash.try_into().expect("hash"),
        size: body.len() as u64,
        meta: None,
        meta_numbers: RawNumbers::default(),
    }
}

/// Stores `files` as revision `hash` of `package` in bucket `b`, and tags it
/// `latest`.
async fn publish(remote: &MockRemote, package: &str, hash: &str, files: &[(&str, &[u8])]) -> Res {
    let mut rows = Vec::new();
    for (key, body) in files {
        let row = row(package, key, body);
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
        rows,
        ..Manifest::default()
    };
    remote
        .put_object(
            None,
            &S3Uri::try_from(format!("s3://b/.quilt/packages/{hash}").as_str())?,
            manifest.to_jsonlines().into_bytes(),
        )
        .await?;
    remote
        .put_object(
            None,
            &S3Uri::try_from(format!("s3://b/.quilt/named_packages/{package}/latest").as_str())?,
            hash.as_bytes().to_vec(),
        )
        .await?;
    Ok(())
}

/// A dataspace holding `packages` as its contract's `[packages]` table, with
/// `b` as the default registry.
fn dataspace(storage: &MockStorage, packages: &str) -> std::io::Result<PathBuf> {
    let root = storage.temp_dir.path().join("rnaseq");
    std::fs::create_dir_all(&root)?;
    std::fs::write(
        root.join("data.toml"),
        format!("registry = \"s3://b\"\n\n[packages]\n{packages}"),
    )?;
    Ok(root)
}

async fn lock(
    storage: &MockStorage,
    root: &Path,
) -> Result<Option<Lock>, Box<dyn std::error::Error>> {
    let contract = Contract::read(storage, root).await?;
    Ok(Lock::read(storage, &contract).await?)
}

fn base(lock: &Lock, package: &str) -> Option<String> {
    lock.packages
        .get(&key(package))
        .map(|entry| entry.base.clone())
}

fn report(read: &EntryRead) -> &ReadReport {
    match read {
        EntryRead::Ran(ReadOutcome::Read(report)) => report,
        other => panic!("expected a read, got {other:?}"),
    }
}

#[test(tokio::test)]
async fn a_first_read_fills_each_read_entry_and_records_its_base() -> Res {
    let remote = MockRemote::default();
    let storage = MockStorage::default();
    publish(&remote, "team/raw", FIRST, &[("a.txt", b"alpha")]).await?;
    publish(&remote, "team/ref", REF_FIRST, &[("b.txt", b"beta")]).await?;
    let root = dataspace(
        &storage,
        "read = [\"team/raw\", \"team/ref\"]\nwrite = [\"team/out\"]\n",
    )?;

    let read = read_all(&storage, &remote, &root, &writer()).await?;

    assert!(read.completed());
    let packages: Vec<String> = read
        .entries
        .iter()
        .map(|(entry, _)| entry.package.to_string())
        .collect();
    assert_eq!(
        packages,
        ["team/raw", "team/ref"],
        "the write entry is not read"
    );
    assert_eq!(std::fs::read(root.join("team/raw/a.txt"))?, b"alpha");
    assert_eq!(std::fs::read(root.join("team/ref/b.txt"))?, b"beta");
    let lock = lock(&storage, &root).await?.expect("a lock");
    assert_eq!(base(&lock, "team/raw").as_deref(), Some(FIRST));
    assert_eq!(base(&lock, "team/ref").as_deref(), Some(REF_FIRST));
    assert_eq!(lock.packages.len(), 2);
    assert!(lock.header.filled.is_some(), "every read entry was read");
    let contract = Contract::read(&storage, &root).await?;
    assert_eq!(lock.mismatches(&contract, &writer()), []);
    Ok(())
}

#[test(tokio::test)]
async fn a_later_read_compares_against_the_recorded_base_and_moves_it() -> Res {
    let remote = MockRemote::default();
    let storage = MockStorage::default();
    publish(
        &remote,
        "team/raw",
        FIRST,
        &[("a.txt", b"alpha"), ("b.txt", b"beta")],
    )
    .await?;
    let root = dataspace(&storage, "read = [\"team/raw\"]\n")?;
    read_all(&storage, &remote, &root, &writer()).await?;
    // Only a base says this differs because it was edited here; a first read
    // would call it a collision.
    std::fs::write(root.join("team/raw/a.txt"), b"edited")?;
    publish(
        &remote,
        "team/raw",
        SECOND,
        &[("a.txt", b"alpha"), ("b.txt", b"BETA")],
    )
    .await?;

    let read = read_all(&storage, &remote, &root, &writer()).await?;

    let report = report(&read.entries[0].1);
    assert_eq!(report.kept, [PathBuf::from("a.txt")]);
    assert_eq!(std::fs::read(root.join("team/raw/b.txt"))?, b"BETA");
    let lock = lock(&storage, &root).await?.expect("a lock");
    assert_eq!(base(&lock, "team/raw").as_deref(), Some(SECOND));
    Ok(())
}

#[test(tokio::test)]
async fn a_blocked_or_failed_read_leaves_the_packages_base() -> Res {
    let remote = MockRemote::default();
    let storage = MockStorage::default();
    publish(&remote, "team/raw", FIRST, &[("a.txt", b"alpha")]).await?;
    publish(&remote, "team/ref", REF_FIRST, &[("b.txt", b"beta")]).await?;
    let root = dataspace(&storage, "read = [\"team/raw\", \"team/ref\"]\n")?;
    read_all(&storage, &remote, &root, &writer()).await?;
    std::fs::write(root.join("team/raw/a.txt"), b"edited")?;
    publish(&remote, "team/raw", SECOND, &[("a.txt", b"ALPHA")]).await?;
    publish(
        &remote,
        "team/ref",
        REF_SECOND,
        &[("package-metadata.json", b"{}")],
    )
    .await?;

    let read = read_all(&storage, &remote, &root, &writer()).await?;

    assert!(!read.completed());
    assert!(matches!(
        &read.entries[0].1,
        EntryRead::Ran(ReadOutcome::Blocked { conflicts, .. }) if conflicts == &[PathBuf::from("a.txt")]
    ));
    assert!(matches!(
        &read.entries[1].1,
        EntryRead::Failed(EntryError::HoldsMetadataFile)
    ));
    assert_eq!(std::fs::read(root.join("team/raw/a.txt"))?, b"edited");
    let lock = lock(&storage, &root).await?.expect("a lock");
    assert_eq!(base(&lock, "team/raw").as_deref(), Some(FIRST));
    assert_eq!(base(&lock, "team/ref").as_deref(), Some(REF_FIRST));
    Ok(())
}

#[test(tokio::test)]
async fn a_dataspace_none_of_whose_reads_completes_gets_no_lock() -> Res {
    let remote = MockRemote::default();
    let storage = MockStorage::default();
    publish(&remote, "team/raw", FIRST, &[("a.txt", b"alpha")]).await?;
    let root = dataspace(&storage, "read = [\"team/raw\"]\n")?;
    std::fs::create_dir_all(root.join("team/raw"))?;
    std::fs::write(root.join("team/raw/a.txt"), b"mine")?;

    let read = read_all(&storage, &remote, &root, &writer()).await?;

    assert!(!read.completed());
    assert!(!root.join(LOCK_FILE).exists());
    Ok(())
}

#[test(tokio::test)]
async fn a_package_with_unpushed_commits_is_not_read() -> Res {
    let remote = MockRemote::default();
    let storage = MockStorage::default();
    publish(&remote, "team/raw", FIRST, &[("a.txt", b"alpha")]).await?;
    let root = dataspace(&storage, "read = [\"team/raw\"]\n")?;
    let contract = Contract::read(&storage, &root).await?;
    let mut lock = Lock::new(&contract, &writer());
    let committed = LockEntry {
        base: SECOND.to_string(),
        commits: vec![FIRST.to_string()],
    };
    lock.packages.insert(key("team/raw"), committed.clone());
    lock.write(&storage, &contract, &writer()).await?;

    let read = read_all(&storage, &remote, &root, &writer()).await?;

    assert!(matches!(read.entries[0].1, EntryRead::Unpushed));
    assert!(!root.join("team/raw/a.txt").exists());
    let lock = self::lock(&storage, &root).await?.expect("a lock");
    assert_eq!(lock.packages.get(&key("team/raw")), Some(&committed));
    Ok(())
}

#[test(tokio::test)]
async fn a_dataspace_with_a_sync_entry_is_not_marked_filled() -> Res {
    let remote = MockRemote::default();
    let storage = MockStorage::default();
    publish(&remote, "team/raw", FIRST, &[("a.txt", b"alpha")]).await?;
    let root = dataspace(&storage, "read = [\"team/raw\"]\nsync = [\"team/notes\"]\n")?;

    read_all(&storage, &remote, &root, &writer()).await?;

    let lock = lock(&storage, &root).await?.expect("a lock");
    assert_eq!(base(&lock, "team/raw").as_deref(), Some(FIRST));
    assert_eq!(lock.header.filled, None);
    Ok(())
}

#[test(tokio::test)]
async fn a_contract_edited_by_hand_keeps_the_bases_of_the_packages_it_still_names() -> Res {
    let remote = MockRemote::default();
    let storage = MockStorage::default();
    publish(
        &remote,
        "team/raw",
        FIRST,
        &[("a.txt", b"alpha"), ("b.txt", b"beta")],
    )
    .await?;
    publish(&remote, "team/ref", REF_FIRST, &[("c.txt", b"gamma")]).await?;
    let root = dataspace(&storage, "read = [\"team/raw\"]\n")?;
    read_all(&storage, &remote, &root, &writer()).await?;
    std::fs::write(root.join("team/raw/a.txt"), b"edited")?;
    publish(
        &remote,
        "team/raw",
        SECOND,
        &[("a.txt", b"alpha"), ("b.txt", b"BETA")],
    )
    .await?;
    dataspace(&storage, "read = [\"team/raw\", \"team/ref\"]\n")?;

    let read = read_all(&storage, &remote, &root, &writer()).await?;

    assert!(read.completed());
    assert_eq!(report(&read.entries[0].1).kept, [PathBuf::from("a.txt")]);
    assert_eq!(std::fs::read(root.join("team/ref/c.txt"))?, b"gamma");
    let contract = Contract::read(&storage, &root).await?;
    let lock = lock(&storage, &root).await?.expect("a lock");
    assert_eq!(lock.mismatches(&contract, &writer()), []);
    Ok(())
}

fn long_ago() -> chrono::DateTime<Utc> {
    "2026-01-01T00:00:00Z".parse().expect("a time")
}

/// A lock `writer` wrote for the dataspace, filled [`long_ago`], naming
/// `team/raw` as `entry`.
async fn lock_from(storage: &MockStorage, root: &Path, writer: &Writer, entry: LockEntry) -> Res {
    let contract = Contract::read(storage, root).await?;
    let mut lock = Lock::new(&contract, writer);
    lock.header.filled = Some(long_ago());
    lock.packages.insert(key("team/raw"), entry);
    lock.write(storage, &contract, writer).await?;
    Ok(())
}

fn at(base: &str) -> LockEntry {
    LockEntry {
        base: base.to_string(),
        commits: Vec::new(),
    }
}

fn elsewhere() -> Writer {
    Writer {
        host: "elsewhere".to_string(),
        user: "me".to_string(),
        local_registry: "elsewhere".to_string(),
    }
}

#[test(tokio::test)]
async fn a_lock_from_another_host_is_refused_while_a_working_folder_holds_files() -> Res {
    let remote = MockRemote::default();
    let storage = MockStorage::default();
    publish(&remote, "team/raw", SECOND, &[("a.txt", b"alpha")]).await?;
    let root = dataspace(&storage, "read = [\"team/raw\"]\nwrite = [\"team/out\"]\n")?;
    lock_from(&storage, &root, &elsewhere(), at(FIRST)).await?;
    std::fs::create_dir_all(root.join("team/out"))?;
    std::fs::write(root.join("team/out/result.csv"), b"1,2")?;
    let before = std::fs::read(root.join(LOCK_FILE))?;

    let refused = read_all(&storage, &remote, &root, &writer()).await;

    assert!(
        matches!(&refused, Err(ReadAllError::Mismatch(mismatches)) if mismatches == &[Mismatch::Host]),
        "{refused:?}"
    );
    assert!(!root.join("team/raw").exists());
    assert_eq!(std::fs::read(root.join(LOCK_FILE))?, before);
    Ok(())
}

#[test(tokio::test)]
async fn a_lock_from_another_host_fills_a_checkout_that_holds_no_files_at_its_revisions() -> Res {
    let remote = MockRemote::default();
    let storage = MockStorage::default();
    publish(&remote, "team/raw", FIRST, &[("a.txt", b"alpha")]).await?;
    publish(&remote, "team/raw", SECOND, &[("a.txt", b"ALPHA")]).await?;
    let root = dataspace(&storage, "read = [\"team/raw\"]\n")?;
    lock_from(&storage, &root, &elsewhere(), at(FIRST)).await?;

    let read = read_all(&storage, &remote, &root, &writer()).await;

    assert!(read.is_ok_and(|read| read.completed()));
    assert_eq!(std::fs::read(root.join("team/raw/a.txt"))?, b"alpha");
    let contract = Contract::read(&storage, &root).await?;
    let lock = lock(&storage, &root).await?.expect("a lock");
    assert_eq!(
        lock.mismatches(&contract, &writer()),
        [],
        "written for here"
    );
    assert_eq!(base(&lock, "team/raw").as_deref(), Some(FIRST));
    assert!(
        lock.header
            .filled
            .is_some_and(|filled| filled != long_ago()),
        "filled here, not where the lock was written"
    );
    assert_eq!(lock.header.writer, writer(), "with nothing of the other's");
    Ok(())
}

#[test(tokio::test)]
async fn a_checkout_filled_from_another_hosts_lock_takes_a_revision_the_contract_names() -> Res {
    let remote = MockRemote::default();
    let storage = MockStorage::default();
    publish(&remote, "team/raw", FIRST, &[("a.txt", b"alpha")]).await?;
    publish(&remote, "team/raw", SECOND, &[("a.txt", b"ALPHA")]).await?;
    let root = dataspace(&storage, "read = [\"team/raw\"]\n")?;
    lock_from(&storage, &root, &elsewhere(), at(FIRST)).await?;
    dataspace(
        &storage,
        &format!("read = [{{ package = \"team/raw\", revision = \"{SECOND}\" }}]\n"),
    )?;

    let read = read_all(&storage, &remote, &root, &writer()).await;

    assert!(read.is_ok_and(|read| read.completed()));
    assert_eq!(std::fs::read(root.join("team/raw/a.txt"))?, b"ALPHA");
    let lock = lock(&storage, &root).await?.expect("a lock");
    assert_eq!(base(&lock, "team/raw").as_deref(), Some(SECOND));
    Ok(())
}

#[test(tokio::test)]
async fn a_lock_from_another_host_naming_unpushed_commits_is_refused() -> Res {
    let remote = MockRemote::default();
    let storage = MockStorage::default();
    publish(&remote, "team/raw", FIRST, &[("a.txt", b"alpha")]).await?;
    let root = dataspace(&storage, "read = [\"team/raw\"]\n")?;
    let committed = LockEntry {
        base: FIRST.to_string(),
        commits: vec![SECOND.to_string()],
    };
    lock_from(&storage, &root, &elsewhere(), committed).await?;

    let refused = read_all(&storage, &remote, &root, &writer()).await;

    assert!(
        matches!(&refused, Err(ReadAllError::Mismatch(mismatches)) if mismatches == &[Mismatch::Host]),
        "{refused:?}"
    );
    assert!(!root.join("team/raw").exists());
    Ok(())
}

#[test(tokio::test)]
async fn the_local_registry_a_lock_names_is_kept_and_not_compared() -> Res {
    let remote = MockRemote::default();
    let storage = MockStorage::default();
    publish(&remote, "team/raw", FIRST, &[("a.txt", b"alpha")]).await?;
    let root = dataspace(&storage, "read = [\"team/raw\"]\n")?;
    let with_domain = Writer {
        local_registry: "ernest-mbp".to_string(),
        ..writer()
    };
    lock_from(&storage, &root, &with_domain, at(FIRST)).await?;
    std::fs::create_dir_all(root.join("team/raw"))?;
    std::fs::write(root.join("team/raw/a.txt"), b"alpha")?;

    let read = read_all(&storage, &remote, &root, &writer()).await;

    assert!(read.is_ok_and(|read| read.completed()));
    let lock = lock(&storage, &root).await?.expect("a lock");
    assert_eq!(lock.header.writer.local_registry, "ernest-mbp");
    Ok(())
}

#[test(tokio::test)]
async fn a_working_folder_gone_from_the_dataspace_is_filled_at_the_locks_revision() -> Res {
    let remote = MockRemote::default();
    let storage = MockStorage::default();
    publish(&remote, "team/raw", FIRST, &[("a.txt", b"alpha")]).await?;
    let root = dataspace(&storage, "read = [\"team/raw\"]\n")?;
    read_all(&storage, &remote, &root, &writer()).await?;
    publish(&remote, "team/raw", SECOND, &[("a.txt", b"ALPHA")]).await?;
    std::fs::remove_dir_all(root.join("team/raw"))?;

    let refilled = read_all(&storage, &remote, &root, &writer()).await?;
    let refilled_with = std::fs::read(root.join("team/raw/a.txt"))?;
    read_all(&storage, &remote, &root, &writer()).await?;

    assert!(
        matches!(&refilled.entries[0].1, EntryRead::Refilled(report) if report.revision == FIRST),
        "reported as a refill: {refilled:?}"
    );
    assert_eq!(refilled_with, b"alpha");
    assert_eq!(std::fs::read(root.join("team/raw/a.txt"))?, b"ALPHA");
    Ok(())
}

#[test(tokio::test)]
async fn a_folder_whose_locked_revision_is_gone_is_filled_at_the_contracts() -> Res {
    let remote = MockRemote::default();
    let storage = MockStorage::default();
    publish(&remote, "team/raw", FIRST, &[("a.txt", b"alpha")]).await?;
    let root = dataspace(&storage, "read = [\"team/raw\"]\n")?;
    lock_from(&storage, &root, &writer(), at(SECOND)).await?;

    let read = read_all(&storage, &remote, &root, &writer()).await?;

    assert!(read.completed());
    assert_eq!(std::fs::read(root.join("team/raw/a.txt"))?, b"alpha");
    let lock = lock(&storage, &root).await?.expect("a lock");
    assert_eq!(base(&lock, "team/raw").as_deref(), Some(FIRST));
    Ok(())
}

#[test(tokio::test)]
async fn a_refill_the_bucket_refuses_fails_rather_than_taking_latest() -> Res {
    let remote = MockRemote::default();
    let storage = MockStorage::default();
    publish(&remote, "team/raw", FIRST, &[("a.txt", b"alpha")]).await?;
    publish(&remote, "team/raw", SECOND, &[("a.txt", b"ALPHA")]).await?;
    // FIRST's own bytes, which a read at SECOND never fetches.
    remote.deny_reads(&row("team/raw", "a.txt", b"alpha").physical_key);
    let root = dataspace(&storage, "read = [\"team/raw\"]\n")?;
    lock_from(&storage, &root, &writer(), at(FIRST)).await?;

    let read = read_all(&storage, &remote, &root, &writer()).await?;

    assert!(
        matches!(read.entries[0].1, EntryRead::Failed(_)),
        "{read:?}"
    );
    assert!(!root.join("team/raw/a.txt").exists());
    Ok(())
}

#[cfg(unix)]
#[test(tokio::test)]
async fn a_lock_from_another_host_is_refused_while_a_working_folder_cannot_be_listed() -> Res {
    use std::os::unix::fs::PermissionsExt;
    let remote = MockRemote::default();
    let storage = MockStorage::default();
    publish(&remote, "team/raw", FIRST, &[("a.txt", b"alpha")]).await?;
    let root = dataspace(&storage, "read = [\"team/raw\"]\n")?;
    lock_from(&storage, &root, &elsewhere(), at(FIRST)).await?;
    let team = root.join("team");
    std::fs::create_dir_all(team.join("raw"))?;
    std::fs::set_permissions(&team, std::fs::Permissions::from_mode(0o000))?;

    let refused = read_all(&storage, &remote, &root, &writer()).await;

    std::fs::set_permissions(&team, std::fs::Permissions::from_mode(0o755))?;
    assert!(
        matches!(&refused, Err(ReadAllError::Mismatch(_))),
        "{refused:?}"
    );
    Ok(())
}
