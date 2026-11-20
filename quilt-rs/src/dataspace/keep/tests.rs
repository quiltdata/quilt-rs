use super::*;

use std::path::PathBuf;

use test_log::test;

use crate::LogicalKey;
use crate::dataspace::contract::Registry;
use crate::dataspace::install::Install;
use crate::dataspace::install::install;
use crate::dataspace::lock::Lock;
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

fn writer() -> Writer {
    Writer {
        host: "here".to_string(),
        user: "me".to_string(),
        local_registry: String::new(),
    }
}

fn row(bucket: &str, package: &str, key: &str, body: &[u8]) -> ManifestRow {
    use sha2::Digest;
    let hash =
        multihash::Multihash::<256>::wrap(0x12, &sha2::Sha256::digest(body)).expect("multihash");
    ManifestRow {
        logical_key: LogicalKey::new(key).expect("key"),
        physical_key: format!(
            "s3://{bucket}/{package}/{key}?versionId={}",
            hex::encode(body)
        ),
        hash: hash.try_into().expect("hash"),
        size: body.len() as u64,
        meta: None,
        meta_numbers: RawNumbers::default(),
    }
}

/// Stores `files` as revision `hash` of `team/raw` in bucket `b`, and tags
/// it `latest`.
async fn publish(remote: &MockRemote, hash: &str, files: &[(&str, &[u8])]) -> Res {
    let mut rows = Vec::new();
    for (key, body) in files {
        let row = row("b", "team/raw", key, body);
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
            &S3Uri::try_from("s3://b/.quilt/named_packages/team/raw/latest")?,
            hash.as_bytes().to_vec(),
        )
        .await?;
    Ok(())
}

/// A dataspace with `b` as its default registry and nothing declared.
fn empty(storage: &MockStorage) -> std::io::Result<PathBuf> {
    let root = storage.temp_dir.path().join("rnaseq");
    std::fs::create_dir_all(&root)?;
    std::fs::write(root.join(CONTRACT_FILE), "registry = \"s3://b\"\n")?;
    Ok(root)
}

fn raw() -> PackageKey {
    PackageKey {
        registry: Registry::S3("b".to_string()),
        package: Namespace::try_from("team/raw").expect("package"),
    }
}

async fn contract(storage: &MockStorage, root: &Path) -> Result<Contract, ContractError> {
    Contract::read(storage, root, None).await
}

async fn lock(storage: &MockStorage, root: &Path) -> Result<Lock, Box<dyn std::error::Error>> {
    let contract = contract(storage, root).await?;
    Ok(Lock::read(storage, &contract).await?.ok_or("no lock")?)
}

/// Installs `team/raw` under `access`, covering `paths`.
async fn installed(
    storage: &MockStorage,
    remote: &MockRemote,
    root: &Path,
    access: Access,
    paths: &[&str],
) -> Res {
    let mut request = Install::new("quilt+s3://b#package=team/raw");
    request.access = access;
    request.paths = paths.iter().map(ToString::to_string).collect();
    let installed = install(storage, remote, None, root, &writer(), &request).await?;
    assert!(installed.read.completed(), "{installed:?}");
    Ok(())
}

async fn run(
    storage: &MockStorage,
    remote: &MockRemote,
    root: &Path,
    request: &Keep,
) -> Result<Kept, KeepError> {
    keep(storage, remote, None, root, &writer(), request).await
}

fn paths(scope: Scope) -> Keep {
    Keep {
        paths: Some(scope),
        ..Keep::new(raw())
    }
}

/// Widening a `sync` entry reads the files the new paths cover at the lock's
/// base, not `latest`, before anything could take their absence for a
/// deletion (25.7 rule 4); the lock keeps its base and names the new
/// contract.
#[test(tokio::test)]
async fn widening_reads_the_new_paths_at_the_base_before_absence_counts() -> Res {
    let (storage, remote) = (MockStorage::default(), MockRemote::default());
    publish(
        &remote,
        FIRST,
        &[("data/a.txt", b"alpha"), ("docs/b.txt", b"bravo")],
    )
    .await?;
    let root = empty(&storage)?;
    installed(&storage, &remote, &root, Access::Sync, &["data/"]).await?;
    publish(
        &remote,
        SECOND,
        &[("data/a.txt", b"alpha"), ("docs/b.txt", b"newer")],
    )
    .await?;
    let folder = root.join("team/raw");
    assert!(!folder.join("docs/b.txt").exists());

    let kept = run(&storage, &remote, &root, &paths(Scope::Whole)).await?;

    assert!(kept.declared);
    assert!(
        matches!(&kept.read, KeepRead::Ran(outcome) if outcome.lock_base() == Some(FIRST)),
        "{kept:?}"
    );
    assert_eq!(kept.entry.paths, Paths::Whole);
    assert_eq!(std::fs::read(folder.join("docs/b.txt"))?, b"bravo");
    let text = std::fs::read_to_string(root.join(CONTRACT_FILE))?;
    assert!(
        text.contains("sync = [{ package = \"team/raw\" }]"),
        "{text}"
    );
    let lock = lock(&storage, &root).await?;
    assert_eq!(lock.packages[&raw()], LockEntry::at(FIRST));
    assert_eq!(
        lock.header.data_toml_hash,
        contract(&storage, &root).await?.data_toml_hash()
    );
    Ok(())
}

/// Only the files the old paths did not cover are read: one deleted under
/// them, and not yet committed, stays deleted.
#[test(tokio::test)]
async fn widening_keeps_a_file_deleted_under_the_old_paths_deleted() -> Res {
    let (storage, remote) = (MockStorage::default(), MockRemote::default());
    publish(
        &remote,
        FIRST,
        &[
            ("data/a.txt", b"alpha"),
            ("data/c.txt", b"charlie"),
            ("docs/b.txt", b"bravo"),
        ],
    )
    .await?;
    let root = empty(&storage)?;
    installed(&storage, &remote, &root, Access::Sync, &["data/"]).await?;
    let folder = root.join("team/raw");
    std::fs::remove_file(folder.join("data/c.txt"))?;

    let kept = run(
        &storage,
        &remote,
        &root,
        &paths(Scope::Only(vec!["data/".to_string(), "docs/".to_string()])),
    )
    .await?;

    assert!(kept.read.completed(), "{kept:?}");
    assert_eq!(std::fs::read(folder.join("docs/b.txt"))?, b"bravo");
    assert!(
        !folder.join("data/c.txt").exists(),
        "a deletion under the old paths is the user's"
    );
    let text = std::fs::read_to_string(root.join(CONTRACT_FILE))?;
    assert!(text.contains("path = \"data/\""), "{text}");
    assert!(text.contains("path = \"docs/\""), "{text}");
    Ok(())
}

/// Narrowing rewrites the paths and reads nothing; the files outside them
/// stay in the folder, and are no longer the entry's.
#[test(tokio::test)]
async fn narrowing_leaves_the_files_and_reads_nothing() -> Res {
    let (storage, remote) = (MockStorage::default(), MockRemote::default());
    publish(
        &remote,
        FIRST,
        &[("data/a.txt", b"alpha"), ("docs/b.txt", b"bravo")],
    )
    .await?;
    let root = empty(&storage)?;
    installed(&storage, &remote, &root, Access::Sync, &[]).await?;

    let kept = run(
        &storage,
        &remote,
        &root,
        &paths(Scope::Only(vec!["data/".to_string()])),
    )
    .await?;

    assert!(matches!(kept.read, KeepRead::Nothing), "{kept:?}");
    let contract = contract(&storage, &root).await?;
    let entry = &contract.entries()[0];
    assert!(entry.paths.covers(&LogicalKey::new("data/a.txt")?));
    assert!(!entry.paths.covers(&LogicalKey::new("docs/b.txt")?));
    assert!(root.join("team/raw/docs/b.txt").exists());
    let lock = lock(&storage, &root).await?;
    assert_eq!(lock.header.data_toml_hash, contract.data_toml_hash());
    Ok(())
}

/// Unpushed commits stop only a read: narrowing needs none, so it does not
/// report one that did not happen.
#[test(tokio::test)]
async fn unpushed_commits_stop_a_widening_but_not_a_narrowing() -> Res {
    let (storage, remote) = (MockStorage::default(), MockRemote::default());
    publish(
        &remote,
        FIRST,
        &[("data/a.txt", b"a"), ("docs/b.txt", b"b")],
    )
    .await?;
    let root = empty(&storage)?;
    installed(&storage, &remote, &root, Access::Sync, &[]).await?;
    let contract = contract(&storage, &root).await?;
    let mut locked = lock(&storage, &root).await?.packages[&raw()].clone();
    locked.commits.push(SECOND.to_string());
    let mut entry = contract.entries()[0].clone();
    let only = Paths::Only(vec![PackagePath::Folder("data".parse()?)]);

    entry.paths = only.clone();
    let narrowing = read_added(
        &storage,
        &remote,
        None,
        &contract,
        &entry,
        Some(Paths::Whole),
        Some(&locked),
    )
    .await;
    entry.paths = Paths::Whole;
    let widening = read_added(
        &storage,
        &remote,
        None,
        &contract,
        &entry,
        Some(only),
        Some(&locked),
    )
    .await;

    assert!(matches!(narrowing, KeepRead::Nothing), "{narrowing:?}");
    assert!(matches!(widening, KeepRead::Unpushed), "{widening:?}");
    Ok(())
}

/// A `read` entry is pinned, then unpinned, in `data.toml`; the lock keeps
/// its base, which the next read moves.
#[test(tokio::test)]
async fn a_read_entry_is_pinned_and_unpinned() -> Res {
    let (storage, remote) = (MockStorage::default(), MockRemote::default());
    publish(&remote, FIRST, &[("a.txt", b"alpha")]).await?;
    let root = empty(&storage)?;
    installed(&storage, &remote, &root, Access::Read, &[]).await?;

    let pinned = run(
        &storage,
        &remote,
        &root,
        &Keep {
            revision: Some(Pin::At(FIRST.to_string())),
            ..Keep::new(raw())
        },
    )
    .await?;

    assert_eq!(
        pinned.entry.revision,
        RevisionPointer::Hash(FIRST.to_string())
    );
    let text = std::fs::read_to_string(root.join(CONTRACT_FILE))?;
    assert!(text.contains(&format!("revision = \"{FIRST}\"")), "{text}");

    let unpinned = run(
        &storage,
        &remote,
        &root,
        &Keep {
            revision: Some(Pin::Latest),
            ..Keep::new(raw())
        },
    )
    .await?;

    assert_eq!(unpinned.entry.revision, RevisionPointer::Tag(Tag::Latest));
    let text = std::fs::read_to_string(root.join(CONTRACT_FILE))?;
    assert!(!text.contains("revision"), "{text}");
    assert_eq!(
        lock(&storage, &root).await?.packages[&raw()],
        LockEntry::at(FIRST)
    );
    Ok(())
}

/// A pin outside `read` is what the contract refuses, so neither file
/// changes.
#[test(tokio::test)]
async fn a_pin_the_contract_refuses_changes_nothing() -> Res {
    let (storage, remote) = (MockStorage::default(), MockRemote::default());
    publish(&remote, FIRST, &[("a.txt", b"alpha")]).await?;
    let root = empty(&storage)?;
    installed(&storage, &remote, &root, Access::Sync, &[]).await?;
    let contract_before = std::fs::read(root.join(CONTRACT_FILE))?;
    let lock_before = std::fs::read(root.join(LOCK_FILE))?;

    let refused = run(
        &storage,
        &remote,
        &root,
        &Keep {
            revision: Some(Pin::At(FIRST.to_string())),
            ..Keep::new(raw())
        },
    )
    .await;

    assert!(
        matches!(refused, Err(KeepError::Contract(_))),
        "{refused:?}"
    );
    assert_eq!(std::fs::read(root.join(CONTRACT_FILE))?, contract_before);
    assert_eq!(std::fs::read(root.join(LOCK_FILE))?, lock_before);
    Ok(())
}

/// Moving a package to another list keeps its place in the file's order
/// within the list, its folder and its lock entry; moving a `write` entry to
/// `sync` reads the folder's missing files at the base, and keeps the
/// user's changes.
#[test(tokio::test)]
async fn moving_to_a_read_list_reads_what_the_folder_lacks() -> Res {
    let (storage, remote) = (MockStorage::default(), MockRemote::default());
    publish(&remote, FIRST, &[("a.txt", b"alpha"), ("b.txt", b"bravo")]).await?;
    let root = empty(&storage)?;
    installed(&storage, &remote, &root, Access::Read, &[]).await?;
    let folder = root.join("team/raw");
    let to = |access| Keep {
        access: Some(access),
        ..Keep::new(raw())
    };

    let written = run(&storage, &remote, &root, &to(Access::Write)).await?;
    assert_eq!(written.entry.access, Access::Write);
    assert!(matches!(written.read, KeepRead::Nothing), "{written:?}");
    std::fs::remove_file(folder.join("b.txt"))?;
    std::fs::write(folder.join("a.txt"), b"mine")?;

    let synced = run(&storage, &remote, &root, &to(Access::Sync)).await?;

    assert_eq!(synced.entry.access, Access::Sync);
    assert!(synced.read.completed(), "{synced:?}");
    assert_eq!(std::fs::read(folder.join("b.txt"))?, b"bravo");
    assert_eq!(std::fs::read(folder.join("a.txt"))?, b"mine");
    let text = std::fs::read_to_string(root.join(CONTRACT_FILE))?;
    assert!(text.contains("sync = ["), "{text}");
    assert!(!text.contains("write = [{"), "{text}");
    assert_eq!(
        lock(&storage, &root).await?.packages[&raw()],
        LockEntry::at(FIRST)
    );
    Ok(())
}

/// A package under two lists is changed under the one named, and asking
/// without naming one is refused.
#[test(tokio::test)]
async fn a_package_under_two_lists_takes_the_one_named() -> Res {
    let (storage, remote) = (MockStorage::default(), MockRemote::default());
    let root = empty(&storage)?;
    std::fs::write(
        root.join(CONTRACT_FILE),
        "registry = \"s3://b\"\n[packages]\nread = [{ package = \"team/raw\", path = \"in/\" }]\nwrite = [{ package = \"team/raw\", path = \"out/\" }]\n",
    )?;
    let widen = |from| Keep {
        from,
        paths: Some(Scope::Only(vec!["out/".to_string(), "more/".to_string()])),
        ..Keep::new(raw())
    };

    let ambiguous = run(&storage, &remote, &root, &widen(None)).await;
    assert!(
        matches!(&ambiguous, Err(KeepError::Ambiguous { accesses, .. }) if accesses.len() == 2),
        "{ambiguous:?}"
    );

    let kept = run(&storage, &remote, &root, &widen(Some(Access::Write))).await?;

    assert_eq!(kept.entry.access, Access::Write);
    assert!(kept.entry.paths.covers(&LogicalKey::new("more/x")?));
    let contract = contract(&storage, &root).await?;
    let read = contract
        .entries()
        .iter()
        .find(|entry| entry.access == Access::Read)
        .ok_or("no read entry")?;
    assert!(!read.paths.covers(&LogicalKey::new("more/x")?));
    Ok(())
}

/// With no change asked, the entry is given as the contract has it, and
/// neither file is written.
#[test(tokio::test)]
async fn no_change_writes_nothing() -> Res {
    let (storage, remote) = (MockStorage::default(), MockRemote::default());
    publish(&remote, FIRST, &[("a.txt", b"alpha")]).await?;
    let root = empty(&storage)?;
    installed(&storage, &remote, &root, Access::Sync, &[]).await?;
    let contract_before = std::fs::read(root.join(CONTRACT_FILE))?;
    let lock_before = std::fs::read(root.join(LOCK_FILE))?;

    let kept = run(&storage, &remote, &root, &Keep::new(raw())).await?;

    assert!(!kept.declared);
    assert_eq!(kept.entry.access, Access::Sync);
    assert_eq!(std::fs::read(root.join(CONTRACT_FILE))?, contract_before);
    assert_eq!(std::fs::read(root.join(LOCK_FILE))?, lock_before);
    let missing = run(
        &storage,
        &remote,
        &root,
        &Keep::new(PackageKey {
            package: Namespace::try_from("team/other")?,
            ..raw()
        }),
    )
    .await;
    assert!(
        matches!(missing, Err(KeepError::NotDeclared(_))),
        "{missing:?}"
    );
    Ok(())
}
