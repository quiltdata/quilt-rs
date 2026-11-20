use super::*;

use std::collections::BTreeMap;
use std::path::PathBuf;

use bytes::Bytes;
use test_log::test;

use crate::LogicalKey;
use crate::domain::RegistryStore;
use crate::flow::WorkflowCheck;
use crate::flow::record;
use crate::io::manifest::RowContent;
use crate::io::remote::mocks::MockRemote;
use crate::io::storage::LocalStorage;
use crate::lineage::PackageLineage;
use crate::manifest::HeaderMessage;
use crate::manifest::ManifestHeader;
use crate::manifest::ManifestRow;
use quilt_uri::Namespace;

type Res<T = ()> = Result<T, Box<dyn std::error::Error>>;

const BASE: &str = "1111111111111111111111111111111111111111111111111111111111111111";

fn writer() -> Writer {
    Writer {
        host: "here".to_string(),
        user: "me".to_string(),
        local_registry: String::new(),
    }
}

fn key() -> PackageKey {
    PackageKey {
        registry: Registry::S3("b".to_string()),
        package: Namespace::from(("team", "raw")),
    }
}

/// A dataspace listing `team/raw` in bucket `b` under `access`, and a Domain
/// beside it.
struct Fixture {
    _dir: tempfile::TempDir,
    dataspace: PathBuf,
    domain: DomainRoot,
    remote: MockRemote,
}

impl Fixture {
    fn new(access: &str) -> Res<Self> {
        let dir = tempfile::tempdir()?;
        let dataspace = dir.path().join("rnaseq");
        std::fs::create_dir_all(&dataspace)?;
        std::fs::write(
            dataspace.join("data.toml"),
            format!("registry = \"s3://b\"\n\n[packages]\n{access} = [\"team/raw\"]\n"),
        )?;
        let domain = DomainRoot::new(dir.path().join("domain"));
        Ok(Self {
            _dir: dir,
            dataspace,
            domain,
            remote: MockRemote::default(),
        })
    }

    fn store(&self) -> Res<RegistryStore> {
        Ok(self.domain.registry("b")?)
    }

    /// Commits a revision of `team/raw` holding `body`, with `message`, into
    /// the Domain; returns its top hash.
    async fn commit(&self, message: &str, body: &'static [u8]) -> Res<String> {
        let logical_key = LogicalKey::new("a.txt")?;
        let manifest = Manifest {
            header: ManifestHeader {
                message: HeaderMessage::Text(message.to_string()),
                ..ManifestHeader::default()
            },
            rows: vec![ManifestRow {
                logical_key: logical_key.clone(),
                ..ManifestRow::default()
            }],
            ..Manifest::default()
        };
        let files = BTreeMap::from([(logical_key, RowContent::Bytes(Bytes::from_static(body)))]);
        let (_, recorded) = record(
            &self.store()?,
            &LocalStorage::new(),
            &MockRemote::default(),
            WorkflowCheck::Warn { host: None },
            &key().package,
            PackageLineage::default(),
            manifest,
            &files,
        )
        .await?;
        Ok(recorded.hash().expect("a revision").to_string())
    }

    /// Puts the manifest `hash` in bucket `b`, so it is published.
    async fn publish(&self, hash: &str) -> Res {
        let uri = S3Uri {
            bucket: "b".to_string(),
            key: get_manifest_key(hash),
            version: None,
        };
        self.remote.put_object(None, &uri, b"{}".to_vec()).await?;
        Ok(())
    }

    async fn contract(&self) -> Res<Contract> {
        let storage = LocalStorage::new();
        let here = self.domain.local().names(&storage).await?.name;
        Ok(Contract::read(&storage, &self.dataspace, Some(&here)).await?)
    }

    /// Writes a lock holding `entry` for `team/raw`, as `writer`.
    async fn lock(&self, entry: LockEntry, writer: &Writer) -> Res {
        let contract = self.contract().await?;
        let mut lock = Lock::new(&contract, writer);
        lock.packages.insert(key(), entry);
        lock.write(&LocalStorage::new(), &contract, writer).await?;
        Ok(())
    }

    async fn read_lock(&self) -> Res<Option<Lock>> {
        let contract = self.contract().await?;
        Ok(Lock::read(&LocalStorage::new(), &contract).await?)
    }

    async fn unattached(&self) -> Result<Vec<Unattached>, ReattachError> {
        unattached(
            &LocalStorage::new(),
            &self.remote,
            &self.domain,
            &self.dataspace,
            &writer(),
            &key(),
        )
        .await
    }

    async fn reattach<'a>(
        &self,
        commit: &str,
        base: impl Into<Option<&'a str>>,
    ) -> Result<(), ReattachError> {
        reattach(
            &LocalStorage::new(),
            &self.remote,
            &self.domain,
            &self.dataspace,
            &writer(),
            &key(),
            commit,
            base.into(),
        )
        .await
    }
}

/// Of the package's manifests in the Domain, a published revision and the
/// commit the lock names are left out; the rest are listed with their
/// messages.
#[test(tokio::test)]
async fn only_local_commits_the_lock_leaves_out_are_listed() -> Res {
    let fixture = Fixture::new("sync")?;
    let published = fixture.commit("published", b"p").await?;
    fixture.publish(&published).await?;
    let named = fixture.commit("named", b"n").await?;
    let orphan = fixture.commit("orphaned", b"o").await?;
    fixture
        .lock(
            LockEntry {
                base: Some(BASE.to_string()),
                commits: vec![named],
            },
            &writer(),
        )
        .await?;

    let listed = fixture.unattached().await?;

    assert_eq!(
        listed,
        [Unattached {
            hash: orphan,
            message: Some("orphaned".to_string()),
        }]
    );
    Ok(())
}

/// With the lock deleted, the commit it held is listed, and reattached on
/// the base the user names: the new lock holds that base and the commit,
/// and the listing no longer holds it.
#[test(tokio::test)]
async fn a_commit_a_deleted_lock_held_is_reattached_on_the_named_base() -> Res {
    let fixture = Fixture::new("sync")?;
    let commit = fixture.commit("mine", b"m").await?;
    fixture.publish(BASE).await?;

    assert_eq!(fixture.unattached().await?.len(), 1);
    fixture.reattach(&commit, BASE).await?;

    let lock = fixture.read_lock().await?.expect("a lock");
    assert_eq!(
        lock.packages.get(&key()),
        Some(&LockEntry {
            base: Some(BASE.to_string()),
            commits: vec![commit],
        })
    );
    assert_eq!(fixture.unattached().await?, []);
    Ok(())
}

/// A commit of a package never published is reattached with no base, which
/// its first publish establishes.
#[test(tokio::test)]
async fn a_commit_of_a_package_never_published_is_reattached_with_no_base() -> Res {
    let fixture = Fixture::new("sync")?;
    let commit = fixture.commit("mine", b"m").await?;

    assert_eq!(fixture.unattached().await?.len(), 1);
    fixture.reattach(&commit, None).await?;

    let lock = fixture.read_lock().await?.expect("a lock");
    assert_eq!(
        lock.packages.get(&key()),
        Some(&LockEntry {
            base: None,
            commits: vec![commit],
        })
    );
    assert_eq!(fixture.unattached().await?, []);
    Ok(())
}

/// A lock entry holding a base and no commits, as a read leaves, takes the
/// base the user names.
#[test(tokio::test)]
async fn an_entry_with_no_commits_takes_the_named_base() -> Res {
    let fixture = Fixture::new("sync")?;
    let commit = fixture.commit("mine", b"m").await?;
    fixture.publish(BASE).await?;
    let older = "2222222222222222222222222222222222222222222222222222222222222222";
    fixture.lock(LockEntry::at(older), &writer()).await?;

    fixture.reattach(&commit, BASE).await?;

    let lock = fixture.read_lock().await?.expect("a lock");
    assert_eq!(lock.packages[&key()].base.as_deref(), Some(BASE));
    Ok(())
}

/// A lock that already names unpushed commits of the package is left as it
/// is: reattaching would drop them from it.
#[test(tokio::test)]
async fn a_lock_naming_commits_is_not_reattached_to() -> Res {
    let fixture = Fixture::new("sync")?;
    let held = fixture.commit("held", b"h").await?;
    let orphan = fixture.commit("orphaned", b"o").await?;
    let entry = LockEntry {
        base: Some(BASE.to_string()),
        commits: vec![held],
    };
    fixture.lock(entry.clone(), &writer()).await?;

    let refused = fixture.reattach(&orphan, BASE).await;

    assert!(
        matches!(refused, Err(ReattachError::Attached)),
        "{refused:?}"
    );
    let lock = fixture.read_lock().await?.expect("a lock");
    assert_eq!(lock.packages[&key()], entry);
    Ok(())
}

/// A base that is not a whole top hash or that the bucket lacks, and a
/// commit the listing does not hold, as one the bucket has or one the Domain
/// lacks, are refused, and no lock is written.
#[test(tokio::test)]
async fn a_bad_base_or_a_commit_not_listed_writes_no_lock() -> Res {
    let fixture = Fixture::new("sync")?;
    let commit = fixture.commit("mine", b"m").await?;
    let published = fixture.commit("published", b"p").await?;
    fixture.publish(&published).await?;
    let unheld = "3333333333333333333333333333333333333333333333333333333333333333";

    let short = fixture.reattach(&commit, "1111").await;
    let not_held = fixture.reattach(&commit, unheld).await;
    let pushed = fixture.reattach(&published, BASE).await;
    let absent = fixture.reattach(BASE, BASE).await;

    assert!(matches!(short, Err(ReattachError::Base(_))), "{short:?}");
    assert!(
        matches!(&not_held, Err(ReattachError::Base(hash)) if hash == unheld),
        "{not_held:?}"
    );
    assert!(
        matches!(&pushed, Err(ReattachError::NotUnattached(hash)) if *hash == published),
        "{pushed:?}"
    );
    assert!(
        matches!(&absent, Err(ReattachError::NotUnattached(hash)) if hash == BASE),
        "{absent:?}"
    );
    assert!(fixture.read_lock().await?.is_none());
    Ok(())
}

/// A package listed only under `read` never publishes, so it holds no
/// commits to attach.
#[test(tokio::test)]
async fn a_package_only_read_is_refused() -> Res {
    let fixture = Fixture::new("read")?;
    fixture.commit("mine", b"m").await?;

    let listed = fixture.unattached().await;

    assert!(
        matches!(listed, Err(ReattachError::Undeclared)),
        "{listed:?}"
    );
    Ok(())
}

/// A local registry's package is the Domain's, not a bucket's, so its
/// commits are not listed here.
#[test(tokio::test)]
async fn a_local_registrys_package_is_refused() -> Res {
    let fixture = Fixture::new("sync")?;
    std::fs::write(
        fixture.dataspace.join("data.toml"),
        "[packages]\nsync = [\"quilt+local://lab-mac#package=team/raw\"]\n",
    )?;
    let local = PackageKey {
        registry: Registry::Local("lab-mac".to_string()),
        ..key()
    };

    let listed = unattached(
        &LocalStorage::new(),
        &fixture.remote,
        &fixture.domain,
        &fixture.dataspace,
        &writer(),
        &local,
    )
    .await;

    assert!(
        matches!(listed, Err(ReattachError::LocalRegistry)),
        "{listed:?}"
    );
    Ok(())
}

/// `quilt+local://` with no name stands for the Domain's own local registry,
/// so a contract that holds one still lists a bucket package's commits.
#[test(tokio::test)]
async fn a_contract_naming_this_machines_registry_still_lists() -> Res {
    let fixture = Fixture::new("sync")?;
    std::fs::write(
        fixture.dataspace.join("data.toml"),
        "registry = \"s3://b\"\n[packages]\nsync = [\"team/raw\", \"quilt+local://#package=me/x\"]\n",
    )?;

    let commit = fixture.commit("mine", b"m").await?;
    fixture.publish(BASE).await?;

    assert_eq!(fixture.unattached().await?.len(), 1);
    fixture.reattach(&commit, BASE).await?;

    // The new lock names the registry its keys were resolved with.
    let names = fixture.domain.local().names(&LocalStorage::new()).await?;
    let lock = fixture.read_lock().await?.expect("a lock");
    assert_eq!(lock.header.writer.local_registry, names.name);
    Ok(())
}

/// A lock written by another host is not used.
#[test(tokio::test)]
async fn a_lock_written_elsewhere_is_refused() -> Res {
    let fixture = Fixture::new("sync")?;
    fixture.commit("mine", b"m").await?;
    let elsewhere = Writer {
        host: "elsewhere".to_string(),
        ..writer()
    };
    fixture.lock(LockEntry::at(BASE), &elsewhere).await?;

    let listed = fixture.unattached().await;

    assert!(
        matches!(&listed, Err(ReattachError::Mismatch(found)) if *found == [Mismatch::Host]),
        "{listed:?}"
    );
    Ok(())
}
