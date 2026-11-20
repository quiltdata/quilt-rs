use super::*;

use test_log::test;

use crate::LogicalKey;
use crate::dataspace::lock::LOCK_FILE;
use crate::dataspace::store::tests::publish_local;
use crate::domain::DomainRoot;
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
const OTHER: &str = "3333333333333333333333333333333333333333333333333333333333333333";

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

/// Stores `files` as revision `hash` of `package` in `bucket`, and tags it
/// `latest`.
async fn publish(
    remote: &MockRemote,
    bucket: &str,
    package: &str,
    hash: &str,
    files: &[(&str, &[u8])],
) -> Res {
    let mut rows = Vec::new();
    for (key, body) in files {
        let row = row(bucket, package, key, body);
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
            &S3Uri::try_from(format!("s3://{bucket}/.quilt/packages/{hash}").as_str())?,
            manifest.to_jsonlines().into_bytes(),
        )
        .await?;
    let latest = format!("s3://{bucket}/.quilt/named_packages/{package}/latest");
    remote
        .put_object(
            None,
            &S3Uri::try_from(latest.as_str())?,
            hash.as_bytes().to_vec(),
        )
        .await?;
    Ok(())
}

/// A dataspace whose contract is `contract`.
fn dataspace(storage: &MockStorage, contract: &str) -> std::io::Result<PathBuf> {
    let root = storage.temp_dir.path().join("rnaseq");
    std::fs::create_dir_all(&root)?;
    std::fs::write(root.join(CONTRACT_FILE), contract)?;
    Ok(root)
}

/// A dataspace with `b` as its default registry and nothing declared.
fn empty(storage: &MockStorage) -> std::io::Result<PathBuf> {
    dataspace(storage, "registry = \"s3://b\"\n")
}

async fn contract(storage: &MockStorage, root: &Path) -> Result<Contract, ContractError> {
    Contract::read(storage, root, Some("lab-mac")).await
}

async fn lock(
    storage: &MockStorage,
    root: &Path,
) -> Result<Option<Lock>, Box<dyn std::error::Error>> {
    Ok(Lock::read(storage, &contract(storage, root).await?).await?)
}

fn key(bucket: &str, package: &str) -> PackageKey {
    PackageKey {
        registry: Registry::S3(bucket.to_string()),
        package: Namespace::try_from(package).expect("package"),
    }
}

fn base(lock: &Lock, key: &PackageKey) -> Option<String> {
    lock.packages.get(key).and_then(|entry| entry.base.clone())
}

async fn run(
    storage: &MockStorage,
    remote: &MockRemote,
    root: &Path,
    request: &Install,
) -> Result<Installed, InstallError> {
    install(storage, remote, None, root, &writer(), request).await
}

/// The entry goes in `data.toml` under `sync`, unpinned and with no
/// registry; the folder is filled; the lock takes the revision as its base,
/// written for the contract the install wrote.
#[test(tokio::test)]
async fn an_install_declares_the_entry_fills_the_folder_and_locks_its_revision() -> Res {
    let (storage, remote) = (MockStorage::default(), MockRemote::default());
    publish(&remote, "b", "team/raw", FIRST, &[("a.txt", b"alpha")]).await?;
    let root = dataspace(&storage, "# Our data\nregistry = \"s3://b\"\n")?;

    let installed = run(
        &storage,
        &remote,
        &root,
        &Install::new("quilt+s3://b#package=team/raw"),
    )
    .await?;

    assert!(installed.declared);
    assert_eq!(installed.read.lock_base(), Some(FIRST), "{installed:?}");
    let text = std::fs::read_to_string(root.join(CONTRACT_FILE))?;
    assert!(text.starts_with("# Our data\n"), "{text}");
    assert!(
        text.contains("sync = [{ package = \"team/raw\" }]"),
        "{text}"
    );
    let contract = contract(&storage, &root).await?;
    let entry = &contract.entries()[0];
    assert_eq!(entry.access, Access::Sync);
    assert_eq!(entry.revision, latest());
    assert_eq!(entry.folder, root.join("team/raw"));
    assert_eq!(std::fs::read(root.join("team/raw/a.txt"))?, b"alpha");
    let lock = lock(&storage, &root).await?.expect("a lock");
    assert_eq!(base(&lock, &key("b", "team/raw")).as_deref(), Some(FIRST));
    assert_eq!(lock.header.data_toml_hash, contract.data_toml_hash());
    assert_eq!(lock.header.filled, None);
    Ok(())
}

/// A dataspace with no `data.toml` gets one holding the entry, its registry
/// stated since there is no default.
#[test(tokio::test)]
async fn an_install_into_a_folder_with_no_contract_writes_one() -> Res {
    let (storage, remote) = (MockStorage::default(), MockRemote::default());
    publish(&remote, "b", "team/raw", FIRST, &[("a.txt", b"alpha")]).await?;
    let root = storage.temp_dir.path().join("fresh");

    run(
        &storage,
        &remote,
        &root,
        &Install::new("quilt+s3://b#package=team/raw"),
    )
    .await?;

    let contract = contract(&storage, &root).await?;
    assert_eq!(contract.entries()[0].folder, root.join("b/team/raw"));
    assert_eq!(std::fs::read(root.join("b/team/raw/a.txt"))?, b"alpha");
    assert!(root.join(LOCK_FILE).exists());
    Ok(())
}

/// The lock moves only once the read completes: a read that local changes
/// block leaves the new entry in `data.toml` and no lock.
#[test(tokio::test)]
async fn a_blocked_install_keeps_the_entry_and_writes_no_lock() -> Res {
    let (storage, remote) = (MockStorage::default(), MockRemote::default());
    publish(&remote, "b", "team/raw", FIRST, &[("a.txt", b"alpha")]).await?;
    let root = empty(&storage)?;
    std::fs::create_dir_all(root.join("team/raw"))?;
    std::fs::write(root.join("team/raw/a.txt"), b"mine")?;

    let installed = run(
        &storage,
        &remote,
        &root,
        &Install::new("quilt+s3://b#package=team/raw"),
    )
    .await?;

    assert!(
        matches!(
            installed.read,
            InstallRead::Ran(ReadOutcome::Blocked { .. })
        ),
        "{installed:?}"
    );
    assert_eq!(contract(&storage, &root).await?.entries().len(), 1);
    assert!(!root.join(LOCK_FILE).exists());
    assert_eq!(std::fs::read(root.join("team/raw/a.txt"))?, b"mine");
    Ok(())
}

/// Installing again what the lock already names writes neither file.
#[test(tokio::test)]
async fn a_repeated_install_of_the_same_revision_does_nothing() -> Res {
    let (storage, remote) = (MockStorage::default(), MockRemote::default());
    publish(&remote, "b", "team/raw", FIRST, &[("a.txt", b"alpha")]).await?;
    let root = empty(&storage)?;
    run(
        &storage,
        &remote,
        &root,
        &Install::new("quilt+s3://b#package=team/raw"),
    )
    .await?;
    let contract_before = std::fs::read(root.join(CONTRACT_FILE))?;
    let lock_before = std::fs::read(root.join(LOCK_FILE))?;

    let again = run(
        &storage,
        &remote,
        &root,
        &Install::new("quilt+s3://b#package=team/raw"),
    )
    .await?;
    let pinned = Install::new(format!("quilt+s3://b#package=team/raw@{FIRST}"));
    let by_hash = run(&storage, &remote, &root, &pinned).await?;

    for installed in [&again, &by_hash] {
        assert!(!installed.declared, "{installed:?}");
        assert!(
            matches!(installed.read, InstallRead::Unchanged),
            "{installed:?}"
        );
    }
    assert_eq!(std::fs::read(root.join(CONTRACT_FILE))?, contract_before);
    assert_eq!(std::fs::read(root.join(LOCK_FILE))?, lock_before);
    Ok(())
}

/// The package under another access is refused, and the message says to
/// edit `data.toml`; nothing is written.
#[test(tokio::test)]
async fn an_install_under_another_access_fails_and_changes_nothing() -> Res {
    let (storage, remote) = (MockStorage::default(), MockRemote::default());
    publish(&remote, "b", "team/raw", FIRST, &[("a.txt", b"alpha")]).await?;
    let text = "registry = \"s3://b\"\n[packages]\nread = [\"team/raw\"]\n";
    let root = dataspace(&storage, text)?;

    let installed = run(
        &storage,
        &remote,
        &root,
        &Install::new("quilt+s3://b#package=team/raw"),
    )
    .await;

    let Err(err) = installed else {
        panic!("expected a refusal, got {installed:?}");
    };
    assert!(
        matches!(
            err,
            InstallError::Access {
                declared: Access::Read,
                requested: Access::Sync,
                ..
            }
        ),
        "{err:?}"
    );
    assert!(err.to_string().contains("edit data.toml"), "{err}");
    assert_eq!(std::fs::read_to_string(root.join(CONTRACT_FILE))?, text);
    assert!(!root.join("team/raw").exists());
    assert!(!root.join(LOCK_FILE).exists());
    Ok(())
}

/// A package of a bucket that is not the default states its registry, and
/// is placed under the bucket's folder.
#[test(tokio::test)]
async fn an_install_from_another_bucket_lands_under_the_buckets_folder() -> Res {
    let (storage, remote) = (MockStorage::default(), MockRemote::default());
    publish(&remote, "other", "team/notes", OTHER, &[("n.txt", b"note")]).await?;
    let root = empty(&storage)?;

    let request = Install::new("quilt+s3://other#package=team/notes");
    run(&storage, &remote, &root, &request).await?;

    let text = std::fs::read_to_string(root.join(CONTRACT_FILE))?;
    assert!(
        text.contains("{ registry = \"s3://other\", package = \"team/notes\" }"),
        "{text}"
    );
    assert_eq!(std::fs::read(root.join("other/team/notes/n.txt"))?, b"note");
    let lock = lock(&storage, &root).await?.expect("a lock");
    assert_eq!(
        base(&lock, &key("other", "team/notes")).as_deref(),
        Some(OTHER)
    );
    Ok(())
}

/// A revision the URI names is read and goes in the lock; the contract's
/// entry stays unpinned, so the next read moves on to `latest`.
#[test(tokio::test)]
async fn a_revision_named_goes_in_the_lock_not_the_contract() -> Res {
    let (storage, remote) = (MockStorage::default(), MockRemote::default());
    publish(&remote, "b", "team/raw", FIRST, &[("a.txt", b"alpha")]).await?;
    publish(&remote, "b", "team/raw", SECOND, &[("a.txt", b"ALPHA")]).await?;
    let root = empty(&storage)?;

    let mut request = Install::new(format!("quilt+s3://b#package=team/raw@{FIRST}"));
    request.access = Access::Read;
    run(&storage, &remote, &root, &request).await?;

    let text = std::fs::read_to_string(root.join(CONTRACT_FILE))?;
    assert!(!text.contains(FIRST), "{text}");
    assert_eq!(
        contract(&storage, &root).await?.entries()[0].revision,
        latest()
    );
    assert_eq!(std::fs::read(root.join("team/raw/a.txt"))?, b"alpha");
    let lock = lock(&storage, &root).await?.expect("a lock");
    assert_eq!(base(&lock, &key("b", "team/raw")).as_deref(), Some(FIRST));
    Ok(())
}

/// Paths installed one after another merge into one entry and one folder;
/// the later path is read at the lock's base.
#[test(tokio::test)]
async fn paths_installed_in_turn_share_one_entry_and_folder() -> Res {
    let (storage, remote) = (MockStorage::default(), MockRemote::default());
    let files: &[(&str, &[u8])] = &[
        ("a.txt", b"alpha"),
        ("b/c.txt", b"gamma"),
        ("d.txt", b"delta"),
    ];
    publish(&remote, "b", "team/raw", FIRST, files).await?;
    let root = empty(&storage)?;

    run(
        &storage,
        &remote,
        &root,
        &Install::new("quilt+s3://b#package=team/raw&path=a.txt"),
    )
    .await?;
    let mut request = Install::new("quilt+s3://b#package=team/raw");
    request.paths = vec!["b/".to_string()];
    let installed = run(&storage, &remote, &root, &request).await?;

    assert!(installed.declared, "{installed:?}");
    let contract = contract(&storage, &root).await?;
    assert_eq!(contract.entries().len(), 1);
    assert_eq!(
        contract.entries()[0].paths,
        Paths::Only(vec![
            PackagePath::Folder(LogicalKey::new("b")?),
            PackagePath::File(LogicalKey::new("a.txt")?),
        ])
    );
    assert_eq!(std::fs::read(root.join("team/raw/a.txt"))?, b"alpha");
    assert_eq!(std::fs::read(root.join("team/raw/b/c.txt"))?, b"gamma");
    assert!(!root.join("team/raw/d.txt").exists());
    let lock = lock(&storage, &root).await?.expect("a lock");
    assert_eq!(base(&lock, &key("b", "team/raw")).as_deref(), Some(FIRST));
    Ok(())
}

/// A package of the local registry installs from it, placed under the
/// registry's folder, and is locked under the registry's name.
#[test(tokio::test)]
async fn a_local_package_installs() -> Res {
    let (storage, remote) = (MockStorage::default(), MockRemote::default());
    let local = DomainRoot::new(storage.temp_dir.path().join("domain")).local();
    local.rename(&storage, "lab-mac").await?;
    let hash = publish_local(&storage, &local, "me/analysis", &[("r.txt", b"result")]).await?;
    let root = empty(&storage)?;
    let writer = Writer {
        local_registry: "lab-mac".to_string(),
        ..writer()
    };

    let request = Install::new("quilt+local://#package=me/analysis");
    let installed = install(&storage, &remote, Some(&local), &root, &writer, &request).await?;

    assert_eq!(
        installed.read.lock_base(),
        Some(hash.as_str()),
        "{installed:?}"
    );
    let text = std::fs::read_to_string(root.join(CONTRACT_FILE))?;
    assert!(
        text.contains("registry = \"quilt+local://lab-mac\""),
        "{text}"
    );
    let folder = root.join("lab-mac@local/me/analysis");
    assert_eq!(std::fs::read(folder.join("r.txt"))?, b"result");
    let lock = lock(&storage, &root).await?.expect("a lock");
    let key = PackageKey {
        registry: Registry::Local("lab-mac".to_string()),
        package: Namespace::try_from("me/analysis")?,
    };
    assert_eq!(base(&lock, &key), Some(hash));
    Ok(())
}

/// A local package with no local registry given, or of another machine's,
/// is refused before `data.toml` is written.
#[test(tokio::test)]
async fn a_local_package_this_machine_does_not_hold_is_refused() -> Res {
    let (storage, remote) = (MockStorage::default(), MockRemote::default());
    let local = DomainRoot::new(storage.temp_dir.path().join("domain")).local();
    local.rename(&storage, "lab-mac").await?;
    let root = empty(&storage)?;
    let elsewhere = Install::new("quilt+local://other-mac#package=me/analysis");
    let here = Install::new("quilt+local://lab-mac#package=me/analysis");

    let foreign = install(
        &storage,
        &remote,
        Some(&local),
        &root,
        &writer(),
        &elsewhere,
    )
    .await;
    let unreached = install(&storage, &remote, None, &root, &writer(), &here).await;

    for refused in [&foreign, &unreached] {
        assert!(
            matches!(refused, Err(InstallError::LocalRegistry)),
            "{refused:?}"
        );
    }
    let text = std::fs::read_to_string(root.join(CONTRACT_FILE))?;
    assert_eq!(text, "registry = \"s3://b\"\n");
    Ok(())
}

/// A `write` entry is declared and not read; it takes no revision.
#[test(tokio::test)]
async fn a_write_entry_is_declared_and_not_read() -> Res {
    let (storage, remote) = (MockStorage::default(), MockRemote::default());
    publish(&remote, "b", "team/raw", FIRST, &[("a.txt", b"alpha")]).await?;
    let root = empty(&storage)?;

    let mut request = Install::new("quilt+s3://b#package=team/raw");
    request.access = Access::Write;
    let installed = run(&storage, &remote, &root, &request).await?;
    request.uri = format!("quilt+s3://b#package=team/raw@{FIRST}");
    let pinned = run(&storage, &remote, &root, &request).await;

    assert!(
        matches!(installed.read, InstallRead::NotRead),
        "{installed:?}"
    );
    assert!(!root.join("team/raw").exists());
    assert_eq!(
        contract(&storage, &root).await?.entries()[0].access,
        Access::Write
    );
    assert!(
        matches!(pinned, Err(InstallError::RevisionNotRead(Access::Write))),
        "{pinned:?}"
    );
    Ok(())
}
