use super::*;

use std::path::PathBuf;

use test_log::test;

use crate::LogicalKey;
use crate::dataspace::contract::PackageKey;
use crate::dataspace::contract::Registry;
use crate::dataspace::read::ReadOutcome;
use crate::dataspace::read_all::read_all;
use crate::io::manifest::fetch_jsonl;
use crate::io::remote::mocks::MockRemote;
use crate::io::storage::mocks::MockStorage;
use crate::manifest::Manifest;
use crate::manifest::ManifestRow;
use crate::manifest::RawNumbers;
use quilt_uri::ManifestUri;
use quilt_uri::Namespace;
use quilt_uri::S3Uri;

type Res = Result<(), Box<dyn std::error::Error>>;

const FIRST: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const SECOND: &str = "2222222222222222222222222222222222222222222222222222222222222222";
const NO_USER_RULES: &str = "/no-user-rules/.quiltignore";

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

/// Stores `files` as revision `hash` of `team/raw` in bucket `b`, and tags it
/// `latest`.
async fn seed(remote: &MockRemote, hash: &str, files: &[(&str, &[u8])]) -> Res {
    use sha2::Digest;
    let mut rows = Vec::new();
    for (key, body) in files {
        let hash =
            multihash::Multihash::<256>::wrap(0x12, &sha2::Sha256::digest(body)).expect("hash");
        let row = ManifestRow {
            logical_key: LogicalKey::new(*key)?,
            physical_key: format!("s3://b/team/raw/{key}?versionId={}", hex::encode(body)),
            hash: hash.try_into()?,
            size: body.len() as u64,
            meta: None,
            meta_numbers: RawNumbers::default(),
        };
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

async fn latest(remote: &MockRemote, package: &str) -> Result<String, crate::Error> {
    let tag = format!("s3://b/.quilt/named_packages/{package}/latest");
    let bytes = remote.current(&tag).await?.expect("a latest tag");
    Ok(String::from_utf8(bytes).expect("utf-8"))
}

async fn keys(remote: &MockRemote, package: &str, hash: &str) -> Result<Vec<String>, crate::Error> {
    let uri = ManifestUri {
        hash: hash.to_string(),
        bucket: "b".to_string(),
        namespace: Namespace::try_from(package).expect("package"),
        origin: None,
    };
    let manifest = fetch_jsonl(remote, &uri).await?;
    let mut keys: Vec<String> = manifest
        .rows
        .iter()
        .map(|row| row.logical_key.to_string())
        .collect();
    keys.sort();
    Ok(keys)
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

/// Writes `body` at `key` in the working folder of `package`.
fn write(root: &Path, package: &str, key: &str, body: &[u8]) -> std::io::Result<()> {
    let path = root.join(package).join(key);
    std::fs::create_dir_all(path.parent().expect("a folder"))?;
    std::fs::write(path, body)
}

async fn base(
    storage: &MockStorage,
    root: &Path,
    package: &str,
) -> Result<Option<String>, Box<dyn std::error::Error>> {
    let contract = Contract::read(storage, root).await?;
    let lock = Lock::read(storage, &contract).await?;
    Ok(lock.and_then(|lock| lock.packages.get(&key(package)).map(|e| e.base.clone())))
}

async fn publish_all_here(
    storage: &MockStorage,
    remote: &MockRemote,
    root: &Path,
    writer: &Writer,
) -> Result<DataspacePublish, PublishAllError> {
    Box::pin(publish_all(
        storage,
        remote,
        root,
        writer,
        Path::new(NO_USER_RULES),
    ))
    .await
}

fn advanced(entry: &PackagePublish) -> &str {
    match &entry.published {
        EntryPublish::Ran(Published::Advanced { revision, .. }) => revision,
        other => panic!("expected latest to advance, got {other:?}"),
    }
}

/// A package under `read` for `in/` and under `write` for `out/`.
const READ_AND_WRITE: &str = "read = [{ package = \"team/raw\", path = \"in/\" }]\nwrite = [{ package = \"team/raw\", path = \"out/\" }]\n";

#[test(tokio::test)]
async fn each_write_and_replace_entry_is_published_and_its_revision_locked() -> Res {
    let remote = MockRemote::default();
    let storage = MockStorage::default();
    seed(&remote, FIRST, &[("a.txt", b"a")]).await?;
    let root = dataspace(
        &storage,
        "write = [\"team/raw\"]\nreplace = [\"team/ref\"]\n",
    )?;
    write(&root, "team/raw", "new.txt", b"n")?;
    write(&root, "team/ref", "b.txt", b"b")?;

    let published = publish_all_here(&storage, &remote, &root, &writer()).await?;

    assert!(published.completed());
    let raw = advanced(&published.entries[0]);
    let reference = advanced(&published.entries[1]);
    assert_eq!(keys(&remote, "team/raw", raw).await?, ["a.txt", "new.txt"]);
    assert_eq!(keys(&remote, "team/ref", reference).await?, ["b.txt"]);
    assert_eq!(latest(&remote, "team/ref").await?, reference);
    assert_eq!(
        base(&storage, &root, "team/raw").await?.as_deref(),
        Some(raw)
    );
    assert_eq!(
        base(&storage, &root, "team/ref").await?.as_deref(),
        Some(reference)
    );
    Ok(())
}

#[test(tokio::test)]
async fn a_package_under_read_and_write_publishes_on_its_base_while_that_is_latest() -> Res {
    let remote = MockRemote::default();
    let storage = MockStorage::default();
    seed(&remote, FIRST, &[("in/a.txt", b"a")]).await?;
    let root = dataspace(&storage, READ_AND_WRITE)?;
    read_all(&storage, &remote, &root, &writer()).await?;
    write(&root, "team/raw", "out/b.txt", b"b")?;

    let published = publish_all_here(&storage, &remote, &root, &writer()).await?;

    let entry = &published.entries[0];
    assert!(entry.read.is_none(), "{entry:?}");
    let new = advanced(entry);
    assert_eq!(
        keys(&remote, "team/raw", new).await?,
        ["in/a.txt", "out/b.txt"]
    );
    assert_eq!(
        base(&storage, &root, "team/raw").await?.as_deref(),
        Some(new)
    );
    Ok(())
}

#[test(tokio::test)]
async fn a_package_under_read_and_write_reads_first_when_latest_has_moved() -> Res {
    let remote = MockRemote::default();
    let storage = MockStorage::default();
    seed(&remote, FIRST, &[("in/a.txt", b"a")]).await?;
    let root = dataspace(&storage, READ_AND_WRITE)?;
    read_all(&storage, &remote, &root, &writer()).await?;
    seed(&remote, SECOND, &[("in/a.txt", b"A")]).await?;
    write(&root, "team/raw", "out/b.txt", b"b")?;

    let published = publish_all_here(&storage, &remote, &root, &writer()).await?;

    let entry = &published.entries[0];
    assert!(
        matches!(
            &entry.read,
            Some(EntryRead::Ran(ReadOutcome::Read(report))) if report.revision == SECOND
        ),
        "{entry:?}"
    );
    assert_eq!(std::fs::read(root.join("team/raw/in/a.txt"))?, b"A");
    let new = advanced(entry);
    assert_eq!(
        keys(&remote, "team/raw", new).await?,
        ["in/a.txt", "out/b.txt"]
    );
    assert_eq!(
        base(&storage, &root, "team/raw").await?.as_deref(),
        Some(new)
    );
    Ok(())
}

#[test(tokio::test)]
async fn a_package_under_read_and_write_never_read_is_read_first() -> Res {
    let remote = MockRemote::default();
    let storage = MockStorage::default();
    seed(&remote, FIRST, &[("in/a.txt", b"a")]).await?;
    let root = dataspace(&storage, READ_AND_WRITE)?;
    write(&root, "team/raw", "out/b.txt", b"b")?;

    let published = publish_all_here(&storage, &remote, &root, &writer()).await?;

    let entry = &published.entries[0];
    assert!(entry.read.is_some(), "{entry:?}");
    assert_eq!(std::fs::read(root.join("team/raw/in/a.txt"))?, b"a");
    let new = advanced(entry);
    assert_eq!(
        base(&storage, &root, "team/raw").await?.as_deref(),
        Some(new)
    );
    Ok(())
}

#[test(tokio::test)]
async fn read_paths_added_to_a_written_package_are_read_before_it_publishes() -> Res {
    let remote = MockRemote::default();
    let storage = MockStorage::default();
    seed(&remote, FIRST, &[("in/a.txt", b"a")]).await?;
    let root = dataspace(
        &storage,
        "write = [{ package = \"team/raw\", path = \"out/\" }]\n",
    )?;
    write(&root, "team/raw", "out/b.txt", b"b")?;
    publish_all_here(&storage, &remote, &root, &writer()).await?;
    dataspace(&storage, READ_AND_WRITE)?;
    write(&root, "team/raw", "out/b.txt", b"B")?;

    let published = publish_all_here(&storage, &remote, &root, &writer()).await?;

    let entry = &published.entries[0];
    assert!(entry.read.is_some(), "{entry:?}");
    assert_eq!(std::fs::read(root.join("team/raw/in/a.txt"))?, b"a");
    let new = advanced(entry);
    assert_eq!(
        base(&storage, &root, "team/raw").await?.as_deref(),
        Some(new)
    );
    Ok(())
}

#[test(tokio::test)]
async fn a_base_whose_new_read_paths_could_not_be_read_is_dropped_from_the_lock() -> Res {
    let remote = MockRemote::default();
    let storage = MockStorage::default();
    seed(&remote, FIRST, &[("in/a.txt", b"a")]).await?;
    let root = dataspace(
        &storage,
        "write = [{ package = \"team/raw\", path = \"out/\" }]\n",
    )?;
    write(&root, "team/raw", "out/b.txt", b"b")?;
    publish_all_here(&storage, &remote, &root, &writer()).await?;
    dataspace(
        &storage,
        "read = [{ package = \"team/raw\", path = \"in/\" }]\nwrite = [{ package = \"team/raw\", path = \"out/\" }, \"team/ref\"]\n",
    )?;
    write(&root, "team/ref", "c.txt", b"c")?;
    remote.deny_reads("s3://b/team/raw/in/a.txt?versionId=61");

    let published = publish_all_here(&storage, &remote, &root, &writer()).await?;

    assert!(
        matches!(published.entries[0].published, EntryPublish::NotRead),
        "{published:?}"
    );
    advanced(&published.entries[1]);
    assert_eq!(base(&storage, &root, "team/raw").await?, None);
    Ok(())
}

#[test(tokio::test)]
async fn a_package_under_read_and_write_with_no_revision_yet_gets_its_first() -> Res {
    let remote = MockRemote::default();
    let storage = MockStorage::default();
    let root = dataspace(&storage, READ_AND_WRITE)?;
    write(&root, "team/raw", "out/b.txt", b"b")?;

    let published = publish_all_here(&storage, &remote, &root, &writer()).await?;

    let entry = &published.entries[0];
    assert!(entry.read.is_none(), "{entry:?}");
    let new = advanced(entry);
    assert_eq!(keys(&remote, "team/raw", new).await?, ["out/b.txt"]);
    assert_eq!(
        base(&storage, &root, "team/raw").await?.as_deref(),
        Some(new)
    );
    Ok(())
}

#[test(tokio::test)]
async fn a_package_whose_read_first_is_blocked_publishes_nothing() -> Res {
    let remote = MockRemote::default();
    let storage = MockStorage::default();
    seed(&remote, FIRST, &[("in/a.txt", b"a")]).await?;
    let root = dataspace(&storage, READ_AND_WRITE)?;
    read_all(&storage, &remote, &root, &writer()).await?;
    seed(&remote, SECOND, &[("in/a.txt", b"A")]).await?;
    write(&root, "team/raw", "in/a.txt", b"edited")?;
    write(&root, "team/raw", "out/b.txt", b"b")?;

    let published = publish_all_here(&storage, &remote, &root, &writer()).await?;

    assert!(!published.completed());
    let entry = &published.entries[0];
    assert!(
        matches!(entry.published, EntryPublish::NotRead),
        "{entry:?}"
    );
    assert_eq!(latest(&remote, "team/raw").await?, SECOND);
    assert_eq!(
        base(&storage, &root, "team/raw").await?.as_deref(),
        Some(FIRST)
    );
    Ok(())
}

#[test(tokio::test)]
async fn a_package_with_unpushed_commits_is_not_published() -> Res {
    let remote = MockRemote::default();
    let storage = MockStorage::default();
    seed(&remote, FIRST, &[("a.txt", b"a")]).await?;
    let root = dataspace(&storage, "write = [\"team/raw\"]\n")?;
    let contract = Contract::read(&storage, &root).await?;
    let mut lock = Lock::new(&contract, &writer());
    lock.packages.insert(
        key("team/raw"),
        LockEntry {
            base: FIRST.to_string(),
            commits: vec![SECOND.to_string()],
        },
    );
    lock.write(&storage, &contract, &writer()).await?;
    write(&root, "team/raw", "new.txt", b"n")?;

    let published = publish_all_here(&storage, &remote, &root, &writer()).await?;

    assert!(matches!(
        published.entries[0].published,
        EntryPublish::Unpushed
    ));
    assert_eq!(latest(&remote, "team/raw").await?, FIRST);
    Ok(())
}

#[test(tokio::test)]
async fn a_lock_from_another_host_is_refused_and_nothing_published() -> Res {
    let remote = MockRemote::default();
    let storage = MockStorage::default();
    seed(&remote, FIRST, &[("a.txt", b"a")]).await?;
    let root = dataspace(&storage, "write = [\"team/raw\"]\n")?;
    let elsewhere = Writer {
        host: "elsewhere".to_string(),
        ..writer()
    };
    let contract = Contract::read(&storage, &root).await?;
    Lock::new(&contract, &elsewhere)
        .write(&storage, &contract, &elsewhere)
        .await?;
    write(&root, "team/raw", "new.txt", b"n")?;

    let refused = publish_all_here(&storage, &remote, &root, &writer()).await;

    assert!(
        matches!(&refused, Err(PublishAllError::Mismatch(found)) if found == &[Mismatch::Host]),
        "{refused:?}"
    );
    assert_eq!(latest(&remote, "team/raw").await?, FIRST);
    Ok(())
}
