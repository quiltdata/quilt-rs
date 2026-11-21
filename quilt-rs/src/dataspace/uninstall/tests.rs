use super::*;

use std::collections::BTreeMap;

use bytes::Bytes;
use test_log::test;

use crate::LogicalKey;
use crate::dataspace::contract::Registry;
use crate::dataspace::install::Install;
use crate::dataspace::install::install;
use crate::dataspace::lock::Lock;
use crate::dataspace::reattach::unattached;
use crate::domain::DomainRoot;
use crate::flow::WorkflowCheck;
use crate::flow::record;
use crate::io::manifest::RowContent;
use crate::io::remote::Remote;
use crate::io::remote::mocks::MockRemote;
use crate::io::storage::mocks::MockStorage;
use crate::lineage::PackageLineage;
use crate::manifest::Manifest;
use crate::manifest::ManifestRow;
use crate::manifest::RawNumbers;
use quilt_uri::Namespace;
use quilt_uri::S3Uri;

type Res<T = ()> = Result<T, Box<dyn std::error::Error>>;

const FIRST: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const OTHER: &str = "3333333333333333333333333333333333333333333333333333333333333333";

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

/// Stores `body` as `a.txt`, the one file of revision `hash` of `package` in
/// bucket `b`, and tags it `latest`.
async fn publish(remote: &MockRemote, package: &str, hash: &str, body: &[u8]) -> Res {
    use sha2::Digest;
    let multihash =
        multihash::Multihash::<256>::wrap(0x12, &sha2::Sha256::digest(body)).expect("multihash");
    let physical_key = format!("s3://b/{package}/a.txt?versionId={hash}");
    remote
        .put_object(
            None,
            &S3Uri::try_from(physical_key.as_str())?,
            body.to_vec(),
        )
        .await?;
    let manifest = Manifest {
        rows: vec![ManifestRow {
            logical_key: LogicalKey::new("a.txt")?,
            physical_key,
            hash: multihash.try_into()?,
            size: body.len() as u64,
            meta: None,
            meta_numbers: RawNumbers::default(),
        }],
        ..Manifest::default()
    };
    let at = |key: String| S3Uri::try_from(key.as_str());
    remote
        .put_object(
            None,
            &at(format!("s3://b/.quilt/packages/{hash}"))?,
            manifest.to_jsonlines().into_bytes(),
        )
        .await?;
    remote
        .put_object(
            None,
            &at(format!("s3://b/.quilt/named_packages/{package}/latest"))?,
            hash.as_bytes().to_vec(),
        )
        .await?;
    Ok(())
}

fn dataspace(storage: &MockStorage, contract: &str) -> std::io::Result<PathBuf> {
    let root = storage.temp_dir.path().join("rnaseq");
    std::fs::create_dir_all(&root)?;
    std::fs::write(root.join(CONTRACT_FILE), contract)?;
    Ok(root)
}

async fn lock(storage: &MockStorage, root: &Path) -> Res<Lock> {
    let contract = Contract::read(storage, root, None).await?;
    Ok(Lock::read(storage, &contract).await?.expect("a lock"))
}

/// Every declaration of the package goes, the rest of `data.toml` stays as
/// it was written, the folder a read filled goes, and so does its lock
/// entry; another package keeps its own.
#[test(tokio::test)]
async fn an_uninstall_removes_its_declarations_folder_and_lock_entry() -> Res {
    let (storage, remote) = (MockStorage::default(), MockRemote::default());
    publish(&remote, "team/raw", FIRST, b"alpha").await?;
    publish(&remote, "team/other", OTHER, b"other").await?;
    let root = dataspace(&storage, "# Our data\nregistry = \"s3://b\"\n")?;
    for uri in [
        "quilt+s3://b#package=team/raw&path=a.txt",
        "quilt+s3://b#package=team/other",
    ] {
        install(
            &storage,
            &remote,
            None,
            &root,
            &writer(),
            &Install::new(uri),
        )
        .await?;
    }
    let mut more = Install::new("quilt+s3://b#package=team/raw");
    more.paths = vec!["b/".to_string()];
    install(&storage, &remote, None, &root, &writer(), &more).await?;

    let uninstalled = uninstall(&storage, None, &root, &writer(), &key("team/raw")).await?;

    assert_eq!(uninstalled.removed, Some(root.join("team/raw")));
    assert_eq!(uninstalled.unpushed(), [] as [String; 0]);
    let text = std::fs::read_to_string(root.join(CONTRACT_FILE))?;
    assert!(
        text.starts_with("# Our data\nregistry = \"s3://b\"\n"),
        "{text}"
    );
    assert!(!text.contains("team/raw"), "{text}");
    let contract = Contract::read(&storage, &root, None).await?;
    assert_eq!(contract.entries().len(), 1);
    assert_eq!(contract.entries()[0].key(), key("team/other"));
    assert!(!root.join("team/raw").exists());
    assert_eq!(std::fs::read(root.join("team/other/a.txt"))?, b"other");
    let lock = lock(&storage, &root).await?;
    assert!(!lock.packages.contains_key(&key("team/raw")));
    assert!(lock.packages.contains_key(&key("team/other")));
    assert_eq!(lock.header.data_toml_hash, contract.data_toml_hash());
    Ok(())
}

/// Uninstalling a package whose lock names an unpushed commit drops the
/// lock entry and leaves the commit in the Domain, where `reattach` finds
/// it once the package is declared again.
#[test(tokio::test)]
async fn an_uninstall_with_unpushed_commits_keeps_them_recoverable() -> Res {
    let (storage, remote) = (MockStorage::default(), MockRemote::default());
    let declared = "registry = \"s3://b\"\n\n[packages]\nsync = [\"team/raw\"]\n";
    let root = dataspace(&storage, declared)?;
    let domain = DomainRoot::new(storage.temp_dir.path().join("domain"));
    let logical_key = LogicalKey::new("a.txt")?;
    let manifest = Manifest {
        rows: vec![ManifestRow {
            logical_key: logical_key.clone(),
            ..ManifestRow::default()
        }],
        ..Manifest::default()
    };
    let files = BTreeMap::from([(logical_key, RowContent::Bytes(Bytes::from_static(b"mine")))]);
    let (_, recorded) = record(
        &domain.registry("b")?,
        &storage,
        &remote,
        WorkflowCheck::Warn { host: None },
        &key("team/raw").package,
        PackageLineage::default(),
        manifest,
        &files,
    )
    .await?;
    let commit = recorded.hash().expect("a revision").to_string();
    let contract = Contract::read(&storage, &root, None).await?;
    let mut locked = Lock::new(&contract, &writer());
    locked.packages.insert(
        key("team/raw"),
        LockEntry {
            base: None,
            commits: vec![commit.clone()],
        },
    );
    locked.write(&storage, &contract, &writer()).await?;

    let uninstalled = uninstall(&storage, None, &root, &writer(), &key("team/raw")).await?;

    assert_eq!(uninstalled.unpushed(), std::slice::from_ref(&commit));
    assert!(lock(&storage, &root).await?.packages.is_empty());
    let store = domain.registry("b")?;
    assert!(
        store
            .installed_manifest(&key("team/raw").package, &commit)
            .exists()
    );
    std::fs::write(root.join(CONTRACT_FILE), declared)?;
    let listed = unattached(
        &storage,
        &remote,
        &domain,
        &root,
        &writer(),
        &key("team/raw"),
    )
    .await?;
    assert_eq!(
        listed
            .iter()
            .map(|commit| commit.hash.as_str())
            .collect::<Vec<_>>(),
        [commit.as_str()]
    );
    Ok(())
}

/// A working folder outside the dataspace is the user's, and is left.
#[test(tokio::test)]
async fn an_uninstall_leaves_a_folder_outside_the_dataspace() -> Res {
    let (storage, remote) = (MockStorage::default(), MockRemote::default());
    publish(&remote, "team/raw", FIRST, b"alpha").await?;
    let outside = storage.temp_dir.path().join("analysis");
    let contract = format!(
        "registry = \"s3://b\"\n[packages]\nread = [{{ package = \"team/raw\", folder = {:?} }}]\n",
        outside.to_string_lossy()
    );
    let root = dataspace(&storage, &contract)?;
    crate::dataspace::read_all::read_all(&storage, &remote, None, &root, &writer()).await?;

    let uninstalled = uninstall(&storage, None, &root, &writer(), &key("team/raw")).await?;

    assert_eq!(uninstalled.kept, Some(outside.clone()));
    assert_eq!(std::fs::read(outside.join("a.txt"))?, b"alpha");
    Ok(())
}

/// A folder reached through a link out of the dataspace is outside it, and
/// is left.
#[cfg(unix)]
#[test(tokio::test)]
async fn an_uninstall_leaves_a_folder_reached_through_a_link() -> Res {
    let (storage, remote) = (MockStorage::default(), MockRemote::default());
    publish(&remote, "team/raw", FIRST, b"alpha").await?;
    let outside = storage.temp_dir.path().join("analysis");
    std::fs::create_dir_all(&outside)?;
    let text = "registry = \"s3://b\"\n[packages]\nread = [{ package = \"team/raw\", folder = \"linked/raw\" }]\n";
    let root = dataspace(&storage, text)?;
    std::os::unix::fs::symlink(&outside, root.join("linked"))?;
    crate::dataspace::read_all::read_all(&storage, &remote, None, &root, &writer()).await?;
    assert_eq!(std::fs::read(outside.join("raw/a.txt"))?, b"alpha");

    let uninstalled = uninstall(&storage, None, &root, &writer(), &key("team/raw")).await?;

    assert_eq!(uninstalled.removed, None);
    assert_eq!(std::fs::read(outside.join("raw/a.txt"))?, b"alpha");
    Ok(())
}

/// A package neither file names is refused, and nothing is written.
#[test(tokio::test)]
async fn uninstalling_a_package_not_installed_is_refused() -> Res {
    let storage = MockStorage::default();
    let text = "registry = \"s3://b\"\n[packages]\nread = [\"team/raw\"]\n";
    let root = dataspace(&storage, text)?;

    let uninstalled = uninstall(&storage, None, &root, &writer(), &key("team/other")).await;

    assert!(
        matches!(uninstalled, Err(UninstallError::NotInstalled(_))),
        "{uninstalled:?}"
    );
    assert_eq!(std::fs::read_to_string(root.join(CONTRACT_FILE))?, text);
    Ok(())
}
