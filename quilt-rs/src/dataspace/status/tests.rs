use std::path::PathBuf;

use tempfile::TempDir;
use test_log::test;

use super::*;

use crate::dataspace::contract::PackageKey;
use crate::dataspace::store::tests::publish_local;
use crate::dataspace::sync::EntrySync;
use crate::dataspace::sync::sync;
use crate::domain::LocalRegistry;
use crate::failpoint::stopping_at;
use crate::io::remote::mocks::MockRemote;
use crate::io::storage::LocalStorage;
use crate::manifest::RawNumbers;
use crate::queue::KeptReason;
use quilt_uri::Namespace;
use quilt_uri::S3Uri;

pub(crate) type Res = Result<(), Box<dyn std::error::Error>>;
pub(crate) type Of<T> = Result<T, Box<dyn std::error::Error>>;

pub(crate) const FIRST: &str = "1111111111111111111111111111111111111111111111111111111111111111";
pub(crate) const SECOND: &str = "2222222222222222222222222222222222222222222222222222222222222222";
pub(crate) const THIRD: &str = "3333333333333333333333333333333333333333333333333333333333333333";

pub(crate) fn writer() -> Writer {
    Writer {
        host: "here".to_string(),
        user: "me".to_string(),
        local_registry: String::new(),
    }
}

/// A dataspace whose default registry is bucket `b` on a mock remote, its
/// Domain beside it.
pub(crate) struct World {
    pub(crate) dir: TempDir,
    pub(crate) remote: MockRemote,
    pub(crate) storage: LocalStorage,
}

impl World {
    /// A dataspace whose `[packages]` table is `packages`.
    pub(crate) fn new(packages: &str) -> std::io::Result<Self> {
        let world = Self {
            dir: TempDir::new()?,
            remote: MockRemote::default(),
            storage: LocalStorage::new(),
        };
        world.contract(packages)?;
        Ok(world)
    }

    /// The whole of `team/raw` under `sync`.
    pub(crate) fn synced() -> std::io::Result<Self> {
        Self::new("sync = [\"team/raw\"]")
    }

    /// Writes `data.toml` with `packages` as its `[packages]` table, and
    /// anything after it.
    pub(crate) fn contract(&self, packages: &str) -> std::io::Result<()> {
        std::fs::create_dir_all(self.dataspace())?;
        std::fs::write(
            self.dataspace().join("data.toml"),
            format!("registry = \"s3://b\"\n\n[packages]\n{packages}\n"),
        )
    }

    pub(crate) fn dataspace(&self) -> PathBuf {
        self.dir.path().join("rnaseq")
    }

    pub(crate) fn folder(&self) -> PathBuf {
        self.dataspace().join("team/raw")
    }

    pub(crate) fn domain(&self) -> DomainRoot {
        DomainRoot::new(self.dir.path().join("domain"))
    }

    pub(crate) fn user_rules(&self) -> PathBuf {
        self.dir.path().join("user.quiltignore")
    }

    pub(crate) async fn queue(&self) -> Of<Queue> {
        Ok(Queue::open(&self.domain().queue_db()).await?)
    }

    /// The Domain's local registry, named `name`.
    pub(crate) async fn local_named(&self, name: &str) -> Of<LocalRegistry> {
        let local = self.domain().local();
        local.rename(&self.storage, name).await?;
        Ok(local)
    }

    /// A tick of the dataspace.
    pub(crate) async fn sync(&self) -> Of<Vec<EntrySync>> {
        let synced = Box::pin(sync(
            &self.storage,
            &self.remote,
            &self.domain(),
            &self.queue().await?,
            &self.dataspace(),
            &writer(),
            "autosync",
            &self.user_rules(),
        ))
        .await?;
        Ok(synced.entries.into_iter().map(|(_, sync)| sync).collect())
    }

    /// A tick that commits the folder's changes and stops before pushing
    /// them, so the lock names them unpushed.
    pub(crate) async fn commit(&self) -> Res {
        stopping_at("push.object", 0, self.sync()).await?;
        Ok(())
    }

    /// Stores `files` as revision `hash` of `team/raw`, tags it `latest`,
    /// and lists it in the bucket's history at `second`.
    pub(crate) async fn publish(&self, hash: &str, files: &[(&str, &[u8])]) -> Res {
        self.publish_at(hash, files, 1_700_000_000).await
    }

    /// [`World::publish`], listed at `second`.
    pub(crate) async fn publish_at(&self, hash: &str, files: &[(&str, &[u8])], second: i64) -> Res {
        self.publish_to("team/raw", hash, files, second).await
    }

    /// [`World::publish_at`] for `package`. A bucket keeps a manifest by its
    /// hash alone, so each package needs hashes of its own.
    pub(crate) async fn publish_to(
        &self,
        package: &str,
        hash: &str,
        files: &[(&str, &[u8])],
        second: i64,
    ) -> Res {
        let mut rows = Vec::new();
        for (key, body) in files {
            use sha2::Digest;
            let digest = multihash::Multihash::<256>::wrap(0x12, &sha2::Sha256::digest(body))?;
            let row = ManifestRow {
                logical_key: LogicalKey::new(*key)?,
                physical_key: format!("s3://b/{package}/{key}?versionId={}", hex::encode(body)),
                hash: digest.try_into()?,
                size: body.len() as u64,
                meta: None,
                meta_numbers: RawNumbers::default(),
            };
            self.put(&row.physical_key, body.to_vec()).await?;
            rows.push(row);
        }
        let mut manifest = Manifest {
            rows,
            ..Manifest::default()
        };
        manifest.header.message = crate::manifest::HeaderMessage::Text(format!("made {hash}"));
        self.put(
            &format!("s3://b/.quilt/packages/{hash}"),
            manifest.to_jsonlines().into_bytes(),
        )
        .await?;
        self.put(
            &format!("s3://b/.quilt/named_packages/{package}/{second}"),
            hash.as_bytes().to_vec(),
        )
        .await?;
        self.put(
            &format!("s3://b/.quilt/named_packages/{package}/latest"),
            hash.as_bytes().to_vec(),
        )
        .await
    }

    /// Points `team/raw`'s `latest` at `hash`.
    pub(crate) async fn tag(&self, hash: &str) -> Res {
        self.put(
            "s3://b/.quilt/named_packages/team/raw/latest",
            hash.as_bytes().to_vec(),
        )
        .await
    }

    async fn put(&self, uri: &str, body: Vec<u8>) -> Res {
        self.remote
            .put_object(None, &S3Uri::try_from(uri)?, body)
            .await?;
        Ok(())
    }

    /// A dataspace syncing `team/raw`, filled at [`FIRST`] holding `files`.
    pub(crate) async fn filled(files: &[(&str, &[u8])]) -> Of<Self> {
        let world = Self::synced()?;
        world.fill(files).await?;
        Ok(world)
    }

    /// Publishes [`FIRST`] holding `files`, and fills the dataspace.
    pub(crate) async fn fill(&self, files: &[(&str, &[u8])]) -> Res {
        self.publish(FIRST, files).await?;
        self.sync().await?;
        Ok(())
    }

    pub(crate) fn write(&self, key: &str, body: &str) -> std::io::Result<()> {
        let path = self.folder().join(key);
        std::fs::create_dir_all(path.parent().expect("a folder"))?;
        std::fs::write(path, body)
    }

    /// The lock, as the dataspace holds it.
    pub(crate) async fn lock(&self) -> Of<Lock> {
        let contract = Contract::read(&self.storage, &self.dataspace(), None).await?;
        Ok(Lock::read(&self.storage, &contract)
            .await?
            .ok_or("no lock")?)
    }

    /// The status of the dataspace, with `latest` asked of the bucket.
    pub(crate) async fn status(&self) -> Of<DataspaceStatus> {
        let queue = self.queue().await?;
        Ok(Box::pin(status(
            &self.storage,
            &self.remote,
            Some(&self.domain()),
            Some(&queue),
            &self.dataspace(),
            &writer(),
            &self.user_rules(),
            true,
        ))
        .await?)
    }

    /// The status of the contract's only entry.
    pub(crate) async fn entry(&self) -> Of<EntryStatus> {
        let mut status = self.status().await?;
        assert_eq!(status.entries.len(), 1, "{status:?}");
        Ok(status.entries.remove(0))
    }
}

pub(crate) fn package() -> Namespace {
    ("team", "raw").into()
}

pub(crate) fn key() -> PackageKey {
    PackageKey {
        registry: Registry::S3("b".to_string()),
        package: package(),
    }
}

/// The changes, by path, as the words a status shows.
fn changed(status: &EntryStatus) -> Vec<(String, &'static str)> {
    status
        .changes
        .iter()
        .map(|(path, change)| {
            let word = match change {
                Change::Added(_) => "added",
                Change::Modified(_) => "modified",
                Change::Removed(_) => "removed",
            };
            (path.display().to_string().replace('\\', "/"), word)
        })
        .collect()
}

fn hashes(base: Option<&str>, current: Option<&str>, latest: Option<&str>) -> Hashes {
    Hashes {
        base: base.map(str::to_string),
        current: current.map(str::to_string),
        latest: latest.map(str::to_string),
        remote: base.map(str::to_string),
    }
}

/// A filled folder is up to date, and behind once `latest` moves.
#[test(tokio::test)]
async fn a_filled_package_is_up_to_date_then_behind_a_newer_latest() -> Res {
    let world = World::filled(&[("a.txt", b"v1")]).await?;

    let entry = world.entry().await?;

    assert_eq!(entry.upstream_state, UpstreamState::UpToDate, "{entry:?}");
    assert_eq!(entry.hashes, hashes(Some(FIRST), Some(FIRST), Some(FIRST)));
    assert_eq!(entry.latest_source, LatestSource::Registry);
    assert!(entry.installed);
    assert!(entry.changes.is_empty(), "{entry:?}");
    assert_eq!(entry.state(), PackageState::Latest);

    world.publish(SECOND, &[("a.txt", b"v2")]).await?;
    let entry = world.entry().await?;

    assert_eq!(entry.upstream_state, UpstreamState::Behind, "{entry:?}");
    assert_eq!(entry.hashes, hashes(Some(FIRST), Some(FIRST), Some(SECOND)));
    assert_eq!(entry.state(), PackageState::Behind);
    Ok(())
}

/// A commit not pushed is ahead, and diverged once `latest` moves too.
#[test(tokio::test)]
async fn an_unpushed_commit_is_ahead_then_diverged_from_a_newer_latest() -> Res {
    let world = World::filled(&[("a.txt", b"v1")]).await?;
    world.write("a.txt", "v2")?;
    world.commit().await?;
    let commit = world
        .lock()
        .await?
        .packages
        .get(&key())
        .and_then(|locked| locked.commits.last().cloned())
        .ok_or("no commit")?;

    let entry = world.entry().await?;

    assert_eq!(entry.upstream_state, UpstreamState::Ahead, "{entry:?}");
    assert_eq!(entry.unpushed, vec![commit.clone()]);
    assert_eq!(
        entry.hashes,
        hashes(Some(FIRST), Some(&commit), Some(FIRST))
    );
    // The folder holds the commit, so nothing is pending but the push.
    assert!(entry.changes.is_empty(), "{entry:?}");
    assert_eq!(entry.state(), PackageState::PendingCommit);

    world.publish(SECOND, &[("a.txt", b"theirs")]).await?;
    let entry = world.entry().await?;

    assert_eq!(entry.upstream_state, UpstreamState::Diverged, "{entry:?}");
    assert_eq!(entry.state(), PackageState::Diverged);
    Ok(())
}

/// A package never published, with a commit and no base, is local while
/// nothing is published, up to date once `latest` names its commit, and
/// diverged from any other `latest` (entry 25m item 1).
#[test(tokio::test)]
async fn a_base_less_commit_is_local_then_up_to_date_or_diverged() -> Res {
    let world = World::synced()?;
    world.sync().await?;
    world.write("a.txt", "mine")?;
    world.commit().await?;
    let locked = world.lock().await?.packages.get(&key()).cloned();
    let commit = match locked {
        Some(LockEntry {
            base: None,
            commits,
        }) if commits.len() == 1 => commits[0].clone(),
        other => panic!("expected one commit with no base, got {other:?}"),
    };

    let entry = world.entry().await?;

    assert_eq!(entry.upstream_state, UpstreamState::Local, "{entry:?}");
    assert_eq!(entry.hashes, hashes(None, Some(&commit), None));
    assert_eq!(entry.state(), PackageState::Unpublished);

    world.tag(&commit).await?;
    let entry = world.entry().await?;
    assert_eq!(entry.upstream_state, UpstreamState::UpToDate, "{entry:?}");

    world.publish(SECOND, &[("a.txt", b"theirs")]).await?;
    let entry = world.entry().await?;
    assert_eq!(entry.upstream_state, UpstreamState::Diverged, "{entry:?}");
    Ok(())
}

/// A package the lock does not name is not installed, and behind a
/// `latest` it has not read.
#[test(tokio::test)]
async fn a_package_not_in_the_lock_is_not_installed_and_behind() -> Res {
    let world = World::synced()?;
    world.publish(FIRST, &[("a.txt", b"v1")]).await?;

    let entry = world.entry().await?;

    assert!(!entry.installed, "{entry:?}");
    assert_eq!(entry.upstream_state, UpstreamState::Behind);
    assert_eq!(entry.hashes, hashes(None, None, Some(FIRST)));
    Ok(())
}

/// The folder's changes are what the tick's commit would take: the
/// contract's ignore policy leaves out what it names, and the lock is
/// never a file of the package.
#[test(tokio::test)]
async fn pending_changes_leave_out_what_the_ignore_policy_leaves_out() -> Res {
    let world = World::new("sync = [\"team/raw\"]\n\n[ignore]\nextend_patterns = [\"*.tmp\"]")?;
    world.fill(&[("a.txt", b"v1"), ("gone.txt", b"v1")]).await?;
    world.write("a.txt", "v2")?;
    world.write("new.txt", "added")?;
    world.write("scratch.tmp", "ignored")?;
    std::fs::remove_file(world.folder().join("gone.txt"))?;

    let entry = world.entry().await?;

    assert_eq!(
        changed(&entry),
        [
            ("a.txt".to_string(), "modified"),
            ("gone.txt".to_string(), "removed"),
            ("new.txt".to_string(), "added"),
        ]
    );
    assert_eq!(entry.state(), PackageState::PendingChanges { files: 3 });
    Ok(())
}

/// A folder inside the working folder that holds a `data.toml` of its own
/// is another dataspace: its files are no change, and status says it was
/// left out and why (25o item 10).
#[test(tokio::test)]
async fn a_nested_dataspace_is_reported_as_left_out() -> Res {
    let world = World::synced()?;
    world.fill(&[("a.txt", b"v1")]).await?;
    world.write("inner/data.toml", "[packages]\n")?;
    world.write("inner/b.txt", "theirs")?;

    let entry = world.entry().await?;

    assert_eq!(changed(&entry), []);
    assert_eq!(
        entry.left_out,
        [LeftOut {
            path: PathBuf::from("inner"),
            reason: LeftOutReason::Dataspace,
        }]
    );
    assert_eq!(
        LeftOutReason::Dataspace.to_string(),
        "it holds a data.toml of its own, so it is another dataspace"
    );
    Ok(())
}

/// A file missing from a `read` folder is filled by the next read, so it
/// is not a deletion; an added or changed one is a local change.
#[test(tokio::test)]
async fn a_missing_file_is_no_deletion_under_read() -> Res {
    let world = World::new("read = [\"team/raw\"]")?;
    world
        .publish(FIRST, &[("a.txt", b"v1"), ("b.txt", b"v1")])
        .await?;
    crate::dataspace::read_all::read_all(
        &world.storage,
        &world.remote,
        None,
        &world.dataspace(),
        &writer(),
    )
    .await?;
    std::fs::remove_file(world.folder().join("a.txt"))?;
    world.write("b.txt", "mine")?;

    let entry = world.entry().await?;

    assert_eq!(changed(&entry), [("b.txt".to_string(), "modified")]);
    Ok(())
}

/// One entry's status is that entry's alone, not the contract's first, and
/// an entry the contract lacks has none.
#[test(tokio::test)]
async fn one_entry_status_reads_that_entry_alone() -> Res {
    let world = World::new("sync = [\"team/raw\"]\nread = [\"team/other\"]")?;
    world.write("a.txt", "new")?;
    let whole = world.status().await?;
    let [first, other] = whole.entries.as_slice() else {
        panic!("{whole:?}");
    };

    let one = entry_status(
        &world.storage,
        &world.remote,
        Some(&world.domain()),
        None,
        &world.dataspace(),
        &writer(),
        &world.user_rules(),
        &other.entry,
        false,
    )
    .await?
    .ok_or("no status")?;
    assert_eq!(one.entry, other.entry);
    assert_eq!(one.changes.len(), other.changes.len());
    let mut gone = first.entry.clone();
    gone.package = ("team", "none").into();
    let none = entry_status(
        &world.storage,
        &world.remote,
        Some(&world.domain()),
        None,
        &world.dataspace(),
        &writer(),
        &world.user_rules(),
        &gone,
        false,
    )
    .await?;
    assert!(none.is_none(), "{none:?}");
    Ok(())
}

/// A folder the lock says was filled, now missing, is unavailable rather
/// than every file deleted.
#[test(tokio::test)]
async fn a_missing_folder_is_unavailable_not_deleted() -> Res {
    let world = World::filled(&[("a.txt", b"v1")]).await?;
    std::fs::remove_dir_all(world.folder())?;

    let entry = world.entry().await?;

    assert_eq!(
        entry.working_copy,
        WorkingCopy::Unavailable(entry.entry.folder.clone())
    );
    assert!(entry.changes.is_empty(), "{entry:?}");
    assert_eq!(entry.state(), PackageState::Unavailable);
    Ok(())
}

/// The package's rows in `conflict.json`, written by a tick a collision
/// blocked, and the revision `queue.db` kept, both show.
#[test(tokio::test)]
async fn conflicts_and_kept_revisions_show() -> Res {
    let world = World::filled(&[("a.txt", b"v1")]).await?;
    world.publish(SECOND, &[("a.txt", b"v2")]).await?;
    world.write("a.txt", "mine")?;
    let ticked = world.sync().await?;
    assert!(
        matches!(ticked.as_slice(), [EntrySync::Blocked { .. }]),
        "{ticked:?}"
    );
    let kept = KeptRevision {
        hash: THIRD.to_string(),
        base: Some(FIRST.to_string()),
        reason: KeptReason::WorkflowBlocked,
    };
    let contract = Contract::read(&world.storage, &world.dataspace(), None).await?;
    let entry = contract.entries().first().ok_or("no entry")?;
    world
        .queue()
        .await?
        .keep(&folder_key(entry), kept.clone())
        .await?;

    let entry = world.entry().await?;

    assert_eq!(
        entry.conflicts.keys().collect::<Vec<_>>(),
        [&LogicalKey::new("a.txt")?]
    );
    assert_eq!(entry.kept, Some(kept));
    Ok(())
}

/// A `quilt+local://` entry is read from the Domain's local registry: its
/// `latest` is the registry's own, and its folder is compared as a
/// bucket's is.
#[test(tokio::test)]
async fn a_local_entry_reads_its_status_from_the_local_registry() -> Res {
    let world = World::new("sync = [\"quilt+local://lab-mac#package=me/out\"]")?;
    let local = world.local_named("lab-mac").await?;
    let first = publish_local(&world.storage, &local, "me/out", &[("o.txt", b"o1")]).await?;
    world.sync().await?;

    let entry = world.entry().await?;

    assert_eq!(entry.upstream_state, UpstreamState::UpToDate, "{entry:?}");
    assert_eq!(entry.latest_source, LatestSource::Registry);
    assert_eq!(entry.hashes.base.as_deref(), Some(first.as_str()));
    assert!(entry.problems.is_empty(), "{entry:?}");

    std::fs::write(entry.entry.folder.join("p.txt"), "added")?;
    let second = publish_local(&world.storage, &local, "me/out", &[("o.txt", b"o2")]).await?;
    let entry = world.entry().await?;

    assert_eq!(entry.upstream_state, UpstreamState::Behind, "{entry:?}");
    assert_eq!(entry.hashes.latest.as_deref(), Some(second.as_str()));
    assert_eq!(changed(&entry), [("p.txt".to_string(), "added")]);
    Ok(())
}

/// An entry read by the name the local registry had before it was renamed
/// is still that entry.
#[test(tokio::test)]
async fn one_entry_is_found_by_the_name_the_local_registry_had() -> Res {
    let world = World::new("sync = [\"quilt+local://old-mac#package=me/out\"]")?;
    let local = world.local_named("old-mac").await?;
    publish_local(&world.storage, &local, "me/out", &[("o.txt", b"o1")]).await?;
    world.sync().await?;
    let mut wanted = world.entry().await?.entry;
    world.local_named("lab-mac").await?;
    wanted.registry = Registry::Local("old-mac".to_string());

    let one = entry_status(
        &world.storage,
        &world.remote,
        Some(&world.domain()),
        None,
        &world.dataspace(),
        &writer(),
        &world.user_rules(),
        &wanted,
        false,
    )
    .await?;

    assert!(one.is_some_and(|one| one.installed));
    Ok(())
}

/// A package moved to its bucket whose move has not landed is read from
/// the lock where it still is, under the local registry's key.
#[test(tokio::test)]
async fn a_moved_package_is_read_where_the_lock_holds_it() -> Res {
    let (world, _, head) = crate::dataspace::sync::tests::moving().await?;
    world.local_named("lab-mac").await?;
    let contract = Contract::read(&world.storage, &world.dataspace(), None).await?;
    let here = Writer {
        local_registry: "lab-mac".to_string(),
        ..writer()
    };

    let one = entry_status(
        &world.storage,
        &world.remote,
        Some(&world.domain()),
        None,
        &world.dataspace(),
        &here,
        &world.user_rules(),
        &contract.entries()[0],
        false,
    )
    .await?
    .ok_or("no status")?;

    assert!(one.installed, "{one:?}");
    assert_eq!(one.hashes.current.as_deref(), Some(head.as_str()));
    Ok(())
}

/// Whether the dataspace was filled, and what in the lock's header does not
/// fit, show.
#[test(tokio::test)]
async fn filled_and_a_lock_mismatch_show() -> Res {
    let world = World::synced()?;
    world.publish(FIRST, &[("a.txt", b"v1")]).await?;
    let before = world.status().await?;
    assert!(!before.locked && before.filled.is_none(), "{before:?}");

    world.sync().await?;
    let filled = world.status().await?;
    assert!(filled.locked && filled.filled.is_some(), "{filled:?}");
    assert!(filled.mismatches.is_empty(), "{filled:?}");

    world.contract("sync = [\"team/raw\"]\n# edited by hand")?;
    let edited = world.status().await?;
    assert_eq!(edited.mismatches, [Mismatch::DataToml]);
    Ok(())
}

/// A file's bytes and modification time, or `None` for a folder.
type Seen = Option<(Vec<u8>, std::time::SystemTime)>;

/// Every file under `dir`, with its bytes and modification time, and every
/// folder.
fn snapshot(dir: &Path) -> std::io::Result<Vec<(PathBuf, Seen)>> {
    let mut seen = Vec::new();
    let mut queue = vec![dir.to_path_buf()];
    while let Some(dir) = queue.pop() {
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            if entry.file_type()?.is_dir() {
                queue.push(path.clone());
                seen.push((path, None));
            } else {
                let modified = entry.metadata()?.modified()?;
                seen.push((path.clone(), Some((std::fs::read(&path)?, modified))));
            }
        }
    }
    seen.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(seen)
}

/// A status writes nothing: not in the dataspace, its folders, the lock,
/// `conflict.json` or `queue.db`, and not in the Domain, whose local
/// registry it does not name. A bucket's revision is not cached.
#[test(tokio::test)]
async fn a_status_writes_nothing() -> Res {
    let world = World::new("sync = [\"team/raw\"]\nread = [\"team/other\"]")?;
    world
        .publish_to("team/other", THIRD, &[("o.txt", b"o1")], 1_700_000_000)
        .await?;
    // Read with no Domain, so the Domain holds none of its revision.
    crate::dataspace::read_all::read_all(
        &world.storage,
        &world.remote,
        None,
        &world.dataspace(),
        &writer(),
    )
    .await?;
    world.fill(&[("a.txt", b"v1")]).await?;
    world.write("a.txt", "v2")?;
    world.commit().await?;
    world.publish(SECOND, &[("a.txt", b"theirs")]).await?;
    world.write("b.txt", "pending")?;
    std::fs::write(world.dataspace().join("team/other/o.txt"), "mine")?;
    world.contract("sync = [\"team/raw\"]\nread = [\"team/other\"]\n# by hand")?;
    let queue = world.queue().await?;
    queue.jobs().await?;
    let before = snapshot(world.dir.path())?;
    assert!(
        !world.domain().local().dir().join("registry.json").exists(),
        "the local registry was named before the status"
    );

    let status = Box::pin(status(
        &world.storage,
        &world.remote,
        Some(&world.domain()),
        Some(&queue),
        &world.dataspace(),
        &writer(),
        &world.user_rules(),
        true,
    ))
    .await?;
    let without_domain = Box::pin(status_without_domain(&world)).await?;

    assert_eq!(status.entries.len(), 2, "{status:?}");
    let raw = status
        .entries
        .iter()
        .find(|entry| entry.entry.package == package())
        .ok_or("no team/raw")?;
    assert_eq!(raw.upstream_state, UpstreamState::Diverged, "{raw:?}");
    assert!(!raw.changes.is_empty(), "{raw:?}");
    let other = status
        .entries
        .iter()
        .find(|entry| entry.entry.package != package())
        .ok_or("no team/other")?;
    assert_eq!(other.upstream_state, UpstreamState::UpToDate, "{other:?}");
    assert_eq!(changed(other), [("o.txt".to_string(), "modified")]);
    assert_eq!(status.mismatches, [Mismatch::DataToml]);
    assert_eq!(without_domain.entries.len(), 2, "{without_domain:?}");
    let after = snapshot(world.dir.path())?;
    let changed: Vec<&PathBuf> = after
        .iter()
        .filter(|seen| !before.contains(seen))
        .chain(before.iter().filter(|seen| !after.contains(seen)))
        .map(|(path, _)| path)
        .collect();
    assert!(changed.is_empty(), "a status wrote {changed:?}");
    Ok(())
}

/// A folder that could not be compared is not called up to date.
#[test(tokio::test)]
async fn a_folder_that_cannot_be_compared_is_unknown() -> Res {
    let world = World::synced()?;
    world
        .publish_at(FIRST, &[("a.txt", b"v1")], 1_700_000_000)
        .await?;
    world.sync().await?;
    world.write(crate::dataspace::METADATA_FILE, "not = [valid")?;

    let entry = world.entry().await?;

    assert!(!entry.problems.is_empty(), "{entry:?}");
    assert_eq!(entry.state(), PackageState::Unknown);
    Ok(())
}

/// A package read at a time is up to date at what it was read as, whatever
/// `latest` is now.
#[test]
fn a_package_pinned_by_time_is_not_behind_a_newer_latest() {
    let hashes = Hashes {
        base: Some(FIRST.to_string()),
        current: Some(FIRST.to_string()),
        latest: Some(SECOND.to_string()),
        remote: Some(FIRST.to_string()),
    };
    let pinned = RevisionPointer::Tag(quilt_uri::Tag::Timestamp(quilt_uri::Seconds(1_700_000_000)));

    assert_eq!(verdict(&hashes, &[], &pinned), UpstreamState::UpToDate);
    assert_eq!(
        verdict(&hashes, &[], &RevisionPointer::Tag(quilt_uri::Tag::Latest)),
        UpstreamState::Behind
    );
}

/// [`status`] with no Domain and no queue.
async fn status_without_domain(world: &World) -> Of<DataspaceStatus> {
    Ok(status(
        &world.storage,
        &world.remote,
        None,
        None,
        &world.dataspace(),
        &writer(),
        &world.user_rules(),
        true,
    )
    .await?)
}
