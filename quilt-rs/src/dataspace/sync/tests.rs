use super::*;

use tempfile::TempDir;
use test_log::test;

use crate::dataspace::METADATA_FILE;
use crate::dataspace::contract::PackageKey;
use crate::failpoint::stopping_at;
use crate::io::manifest::fetch_jsonl;
use crate::io::remote::mocks::MockRemote;
use crate::io::storage::LocalStorage;
use crate::manifest::ManifestHeader;
use quilt_uri::Namespace;

type Res = Result<(), Box<dyn std::error::Error>>;

const LATEST: &str = "s3://b/.quilt/named_packages/team/raw/latest";
const FIRST: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const SECOND: &str = "2222222222222222222222222222222222222222222222222222222222222222";
// A bucket stores a manifest by its hash alone, so each package has its own.
const THIRD: &str = "3333333333333333333333333333333333333333333333333333333333333333";
const FOURTH: &str = "4444444444444444444444444444444444444444444444444444444444444444";
const MESSAGE: &str = "autosync";

fn writer() -> Writer {
    Writer {
        host: "here".to_string(),
        user: "me".to_string(),
        local_registry: String::new(),
    }
}

/// A dataspace with `team/raw` in bucket `b`, its Domain beside it, and the
/// bucket on a mock remote.
struct World {
    dir: TempDir,
    remote: MockRemote,
    storage: LocalStorage,
}

impl World {
    /// A dataspace whose `[packages]` table is `packages`.
    fn new(packages: &str) -> std::io::Result<Self> {
        let world = Self {
            dir: TempDir::new()?,
            remote: MockRemote::default(),
            storage: LocalStorage::new(),
        };
        world.contract(packages)?;
        Ok(world)
    }

    /// The whole package under `sync`.
    fn synced() -> std::io::Result<Self> {
        Self::new("sync = [\"team/raw\"]")
    }

    fn contract(&self, packages: &str) -> std::io::Result<()> {
        std::fs::create_dir_all(self.dataspace())?;
        std::fs::write(
            self.dataspace().join("data.toml"),
            format!("registry = \"s3://b\"\n\n[packages]\n{packages}"),
        )
    }

    fn dataspace(&self) -> PathBuf {
        self.dir.path().join("rnaseq")
    }

    fn folder(&self) -> PathBuf {
        self.dataspace().join("team/raw")
    }

    fn domain(&self) -> DomainRoot {
        DomainRoot::new(self.dir.path().join("domain"))
    }

    /// The user-wide ignore rules file, which need not exist.
    fn user_rules(&self) -> PathBuf {
        self.dir.path().join("user.quiltignore")
    }

    /// The Domain's queue, opened again by each tick as a new process would.
    async fn queue(&self) -> Result<Queue, Box<dyn std::error::Error>> {
        Ok(Queue::open(&self.dir.path().join("queue.db")).await?)
    }

    async fn sync(&self) -> Result<DataspaceSync, Box<dyn std::error::Error>> {
        let dataspace = self.dataspace();
        Ok(Box::pin(sync(
            &self.storage,
            &self.remote,
            &self.domain(),
            &self.queue().await?,
            &dataspace,
            &writer(),
            MESSAGE,
            &self.user_rules(),
        ))
        .await?)
    }

    async fn kept(&self) -> Result<Option<KeptRevision>, Box<dyn std::error::Error>> {
        // The contract's spelling of the folder, which on Windows is not
        // `folder()`'s.
        let contract = Contract::read(&self.storage, &self.dataspace(), None).await?;
        let entry = contract.entries().first().ok_or("no entry")?;
        let folder = FolderKey {
            registry: Some(entry.registry.clone()),
            package: entry.package.clone(),
            folder: entry.folder.clone(),
        };
        Ok(self.queue().await?.kept(&folder).await?)
    }

    async fn read_all(&self) -> Result<(), Box<dyn std::error::Error>> {
        let dataspace = self.dataspace();
        crate::dataspace::read_all::read_all(&self.storage, &self.remote, &dataspace, &writer())
            .await?;
        Ok(())
    }

    /// The tick's one entry.
    async fn tick(&self) -> Result<EntrySync, Box<dyn std::error::Error>> {
        let mut synced = self.sync().await?;
        assert_eq!(synced.entries.len(), 1, "{synced:?}");
        Ok(synced.entries.remove(0).1)
    }

    /// Stores `files` as revision `hash` and tags it `latest`.
    async fn publish(&self, hash: &str, files: &[(&str, &[u8])]) -> Res {
        self.publish_to("team/raw", hash, files).await
    }

    /// [`World::publish`] for `package`.
    async fn publish_to(&self, package: &str, hash: &str, files: &[(&str, &[u8])]) -> Res {
        let header = ManifestHeader::default();
        self.publish_with(package, hash, files, header, Vec::new())
            .await
    }

    /// [`World::publish_to`], under `header` and with metadata for folders.
    async fn publish_with(
        &self,
        package: &str,
        hash: &str,
        files: &[(&str, &[u8])],
        header: ManifestHeader,
        dir_meta: Vec<crate::manifest::DirMeta>,
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
            self.remote
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
            dir_meta,
        };
        self.remote
            .put_object(
                None,
                &S3Uri::try_from(format!("s3://b/.quilt/packages/{hash}").as_str())?,
                manifest.to_jsonlines().into_bytes(),
            )
            .await?;
        self.remote
            .put_object(
                None,
                &S3Uri::try_from(
                    format!("s3://b/.quilt/named_packages/{package}/latest").as_str(),
                )?,
                hash.as_bytes().to_vec(),
            )
            .await?;
        Ok(())
    }

    async fn latest(&self) -> Result<String, Box<dyn std::error::Error>> {
        let bytes = self.remote.current(LATEST).await?.ok_or("no latest")?;
        Ok(String::from_utf8(bytes)?)
    }

    /// The files of the revision `hash` names, by key, with their bytes.
    async fn revision(
        &self,
        hash: &str,
    ) -> Result<Vec<(String, String)>, Box<dyn std::error::Error>> {
        let uri = ManifestUri {
            origin: None,
            bucket: "b".to_string(),
            namespace: package(),
            hash: hash.to_string(),
        };
        let manifest = fetch_jsonl(&self.remote, &uri).await?;
        let mut files = Vec::new();
        for row in manifest.rows {
            let uri = S3Uri::try_from(row.physical_key.as_str())?;
            let bytes = self.remote.current(&uri.to_string()).await?;
            let body = match bytes {
                Some(bytes) => String::from_utf8(bytes)?,
                None => format!("<{}>", row.physical_key),
            };
            files.push((row.logical_key.to_string(), body));
        }
        files.sort();
        Ok(files)
    }

    async fn lock(&self) -> Result<Lock, Box<dyn std::error::Error>> {
        let contract = Contract::read(&self.storage, &self.dataspace(), None).await?;
        Ok(Lock::read(&self.storage, &contract)
            .await?
            .ok_or("no lock")?)
    }

    async fn locked(&self) -> Result<Option<LockEntry>, Box<dyn std::error::Error>> {
        Ok(self.lock().await?.packages.get(&key()).cloned())
    }

    fn write(&self, key: &str, body: &str) -> std::io::Result<()> {
        let path = self.folder().join(key);
        std::fs::create_dir_all(path.parent().expect("a folder"))?;
        std::fs::write(path, body)
    }

    fn text(&self, key: &str) -> Option<String> {
        std::fs::read_to_string(self.folder().join(key)).ok()
    }

    fn remove(&self, key: &str) -> std::io::Result<()> {
        std::fs::remove_file(self.folder().join(key))
    }

    /// A dataspace filled at [`FIRST`], holding `files`.
    async fn filled(files: &[(&str, &[u8])]) -> Result<Self, Box<dyn std::error::Error>> {
        let world = Self::synced()?;
        world.publish(FIRST, files).await?;
        let synced = world.sync().await?;
        assert!(synced.read_only, "{synced:?}");
        assert_eq!(
            world.locked().await?.map(|locked| locked.base).as_deref(),
            Some(FIRST)
        );
        Ok(world)
    }
}

fn package() -> Namespace {
    ("team", "raw").into()
}

fn key() -> PackageKey {
    PackageKey {
        registry: Registry::S3("b".to_string()),
        package: package(),
    }
}

fn files(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(key, body)| ((*key).to_string(), (*body).to_string()))
        .collect()
}

fn advanced(outcome: &EntrySync) -> String {
    match outcome {
        EntrySync::Ran {
            published: Some(Published::Advanced { revision, .. }),
            ..
        } => revision.clone(),
        other => panic!("expected a publish that advanced latest, got {other:?}"),
    }
}

#[test(tokio::test)]
async fn a_first_tick_fills_the_folder_and_publishes_nothing() -> Res {
    let world = World::synced()?;
    world.publish(FIRST, &[("a.txt", b"alpha")]).await?;
    world.write("mine.txt", "local")?;

    let synced = world.sync().await?;

    assert!(synced.read_only);
    assert_eq!(world.text("a.txt").as_deref(), Some("alpha"));
    assert_eq!(world.latest().await?, FIRST);
    let lock = world.lock().await?;
    assert!(lock.header.filled.is_some());
    assert_eq!(
        lock.packages.get(&key()).map(|locked| locked.base.as_str()),
        Some(FIRST)
    );
    Ok(())
}

#[test(tokio::test)]
async fn a_tick_publishes_additions_changes_and_deletions_against_the_base() -> Res {
    let world = World::filled(&[("a.txt", b"v1"), ("b.txt", b"v1"), ("c.txt", b"v1")]).await?;
    world.write("a.txt", "v2")?;
    world.remove("b.txt")?;
    world.write("new/d.txt", "added")?;

    let outcome = world.tick().await?;

    let revision = advanced(&outcome);
    assert_eq!(world.latest().await?, revision);
    assert_eq!(
        world.revision(&revision).await?,
        files(&[("a.txt", "v2"), ("c.txt", "v1"), ("new/d.txt", "added")])
    );
    let locked = world.locked().await?.ok_or("no lock entry")?;
    assert_eq!(locked.base, revision);
    assert_eq!(locked.commits, Vec::<String>::new());
    Ok(())
}

#[test(tokio::test)]
async fn a_tick_leaves_out_what_the_ignore_rules_leave_out_of_a_commit() -> Res {
    let world = World::filled(&[("a.txt", b"a"), (".env", b"old"), ("old.csv", b"o")]).await?;
    std::fs::write(world.user_rules(), "*.tmp\n")?;
    world.write(".quiltignore", "*.log\nold.csv\n")?;
    world.write(".env", "new")?;
    world.write("new.txt", "n")?;
    world.write("run.log", "l")?;
    world.write("scratch.tmp", "t")?;
    world.write("cache/.DS_Store", "d")?;

    let outcome = world.tick().await?;

    // The built-in rules spare a key the package has; the folder's own rules
    // do not.
    assert_eq!(
        world.revision(&advanced(&outcome)).await?,
        files(&[
            (".env", "new"),
            (".quiltignore", "*.log\nold.csv\n"),
            ("a.txt", "a"),
            ("new.txt", "n"),
        ])
    );
    Ok(())
}

#[test(tokio::test)]
async fn a_tick_reads_latest_then_publishes_the_local_work_kept() -> Res {
    let world = World::filled(&[("a.txt", b"v1"), ("b.txt", b"v1")]).await?;
    world
        .publish(SECOND, &[("a.txt", b"v1"), ("b.txt", b"v2")])
        .await?;
    world.write("a.txt", "mine")?;

    let outcome = world.tick().await?;

    let EntrySync::Ran {
        read: Some(read), ..
    } = &outcome
    else {
        panic!("expected a read, got {outcome:?}");
    };
    assert_eq!(read.revision, SECOND);
    let revision = advanced(&outcome);
    assert_eq!(world.text("b.txt").as_deref(), Some("v2"));
    assert_eq!(
        world.revision(&revision).await?,
        files(&[("a.txt", "mine"), ("b.txt", "v2")])
    );
    Ok(())
}

#[test(tokio::test)]
async fn a_collision_blocks_the_tick_before_anything_is_applied_or_published() -> Res {
    let world = World::filled(&[("a.txt", b"v1"), ("b.txt", b"v1")]).await?;
    world
        .publish(SECOND, &[("a.txt", b"v2"), ("b.txt", b"v2")])
        .await?;
    world.write("a.txt", "mine")?;

    let outcome = world.tick().await?;

    assert!(
        matches!(&outcome, EntrySync::Blocked { revision, conflicts }
            if revision == SECOND && *conflicts == [PathBuf::from("a.txt")]),
        "{outcome:?}"
    );
    assert_eq!(world.text("b.txt").as_deref(), Some("v1"));
    assert_eq!(world.latest().await?, SECOND);
    assert_eq!(
        world.locked().await?.map(|locked| locked.base).as_deref(),
        Some(FIRST)
    );
    Ok(())
}

#[test(tokio::test)]
async fn only_the_entrys_paths_are_read_and_published() -> Res {
    let world = World::new("sync = [{ package = \"team/raw\", path = \"data/\" }]")?;
    world
        .publish(FIRST, &[("data/a.txt", b"v1"), ("notes.txt", b"v1")])
        .await?;
    world.sync().await?;
    world
        .publish(SECOND, &[("data/a.txt", b"v1"), ("notes.txt", b"v2")])
        .await?;
    world.write("data/b.txt", "added")?;
    world.write("other.txt", "not the package's")?;

    let outcome = world.tick().await?;

    let revision = advanced(&outcome);
    assert_eq!(world.text("notes.txt"), None);
    assert_eq!(
        world.revision(&revision).await?,
        files(&[
            ("data/a.txt", "v1"),
            ("data/b.txt", "added"),
            ("notes.txt", "v2")
        ])
    );
    Ok(())
}

#[test(tokio::test)]
async fn a_folder_with_nothing_new_publishes_nothing() -> Res {
    let world = World::filled(&[("a.txt", b"v1")]).await?;

    let outcome = world.tick().await?;

    assert!(
        matches!(
            &outcome,
            EntrySync::Ran {
                read: None,
                published: None
            }
        ),
        "{outcome:?}"
    );
    assert_eq!(world.latest().await?, FIRST);
    Ok(())
}

#[test(tokio::test)]
async fn a_push_that_latest_moved_past_is_left_stored_and_the_lock_unmoved() -> Res {
    let world = World::filled(&[("a.txt", b"v1")]).await?;
    world.write("a.txt", "v2")?;
    let gate = world.remote.park(LATEST);

    let rival = async {
        // The tick's own read of `latest`, then the push's.
        gate.arrived().await;
        let push = world.remote.park(LATEST);
        gate.release();
        push.arrived().await;
        world.publish(SECOND, &[("a.txt", b"theirs")]).await?;
        push.release();
        Res::Ok(())
    };
    let (outcome, rivalled) = tokio::join!(world.tick(), rival);
    rivalled?;
    let outcome = outcome?;

    let EntrySync::Ran {
        published: Some(Published::Stored { revision, latest }),
        ..
    } = &outcome
    else {
        panic!("expected the revision stored, got {outcome:?}");
    };
    assert_eq!(latest.as_deref(), Some(SECOND));
    assert_eq!(world.revision(revision).await?, files(&[("a.txt", "v2")]));
    let locked = world.locked().await?.ok_or("no lock entry")?;
    assert_eq!(locked.base, FIRST);
    assert_eq!(locked.commits, std::slice::from_ref(revision));
    assert_eq!(
        world.kept().await?,
        Some(KeptRevision {
            hash: revision.clone(),
            base: Some(FIRST.to_string()),
            reason: KeptReason::LatestMoved,
        })
    );
    Ok(())
}

#[test(tokio::test)]
async fn unpushed_commits_and_a_latest_that_moved_are_diverged() -> Res {
    let world = World::filled(&[("a.txt", b"v1")]).await?;
    world.write("a.txt", "v2")?;
    let stopped = stopping_at("push.object", 0, world.tick()).await?;
    assert!(matches!(stopped, EntrySync::Failed(_)), "{stopped:?}");
    world.publish(SECOND, &[("a.txt", b"theirs")]).await?;

    let outcome = world.tick().await?;

    assert!(
        matches!(&outcome, EntrySync::Diverged { latest } if latest.as_deref() == Some(SECOND)),
        "{outcome:?}"
    );
    assert_eq!(world.text("a.txt").as_deref(), Some("v2"));
    Ok(())
}

#[test(tokio::test)]
async fn a_package_whose_latest_is_gone_is_diverged_and_its_lock_kept() -> Res {
    let world = World::filled(&[("a.txt", b"v1")]).await?;
    world
        .remote
        .storage
        .remove_file(LATEST.strip_prefix("s3://").ok_or("an s3 URI")?)
        .await?;

    let outcome = world.tick().await;

    assert!(
        matches!(&outcome, Ok(EntrySync::Diverged { latest: None })),
        "{outcome:?}"
    );
    assert_eq!(
        world.locked().await?.map(|locked| locked.base).as_deref(),
        Some(FIRST)
    );
    Ok(())
}

#[test(tokio::test)]
async fn a_push_squashes_the_commits_not_pushed_and_releases_them() -> Res {
    let world = World::filled(&[("a.txt", b"v1")]).await?;
    world.write("a.txt", "v2")?;
    stopping_at("push.object", 0, world.tick()).await?;
    let squashed = world.locked().await?.ok_or("no lock entry")?.commits;
    assert_eq!(squashed.len(), 1);
    world.write("b.txt", "added")?;

    let outcome = world.tick().await?;

    let revision = advanced(&outcome);
    assert_eq!(
        world.revision(&revision).await?,
        files(&[("a.txt", "v2"), ("b.txt", "added")])
    );
    let locked = world.locked().await?.ok_or("no lock entry")?;
    assert_eq!(locked.base, revision);
    assert_eq!(locked.commits, Vec::<String>::new());
    // Squashed: only the last commit was stored in the bucket.
    assert!(world.revision(&squashed[0]).await.is_err());
    Ok(())
}

#[test(tokio::test)]
async fn a_missing_or_empty_folder_is_read_back_at_the_lock_and_nothing_published() -> Res {
    let world = World::filled(&[("a.txt", b"v1"), ("b.txt", b"v1")]).await?;
    world
        .publish(SECOND, &[("a.txt", b"v2"), ("b.txt", b"v2")])
        .await?;

    for empty in [false, true] {
        std::fs::remove_dir_all(world.folder())?;
        if empty {
            std::fs::create_dir_all(world.folder())?;
        }

        let outcome = world.tick().await?;

        assert!(
            matches!(&outcome, EntrySync::ReadBack(read) if read.revision == FIRST),
            "{outcome:?}"
        );
        assert_eq!(world.text("a.txt").as_deref(), Some("v1"));
        assert_eq!(world.text("b.txt").as_deref(), Some("v1"));
        assert!(world.folder().join(METADATA_FILE).exists());
        assert_eq!(world.latest().await?, SECOND);
        let locked = world.locked().await?.ok_or("no lock entry")?;
        assert_eq!(locked.base, FIRST);
    }
    Ok(())
}

#[test(tokio::test)]
async fn a_missing_folder_is_read_back_at_the_local_commit_the_lock_names() -> Res {
    let world = World::filled(&[("a.txt", b"v1"), ("b.txt", b"v1")]).await?;
    world.write("a.txt", "v2")?;
    stopping_at("push.object", 0, world.tick()).await?;
    let commits = world.locked().await?.ok_or("no lock entry")?.commits;
    assert_eq!(commits.len(), 1);
    std::fs::remove_dir_all(world.folder())?;

    let outcome = world.tick().await?;

    assert!(
        matches!(&outcome, EntrySync::ReadBack(read) if read.revision == commits[0]),
        "{outcome:?}"
    );
    assert_eq!(world.text("a.txt").as_deref(), Some("v2"));
    assert_eq!(world.text("b.txt").as_deref(), Some("v1"));
    assert_eq!(world.latest().await?, FIRST);
    let locked = world.locked().await?.ok_or("no lock entry")?;
    assert_eq!(locked.base, FIRST);
    assert_eq!(locked.commits, commits);
    Ok(())
}

#[test(tokio::test)]
async fn after_data_toml_is_edited_a_tick_reads_the_entry_whole_first() -> Res {
    let world = World::filled(&[("a.txt", b"v1"), ("b.txt", b"v1")]).await?;
    world.remove("a.txt")?;
    world.contract("# edited\nsync = [\"team/raw\"]")?;

    let synced = world.sync().await?;

    assert!(synced.read_only, "{synced:?}");
    assert_eq!(world.text("a.txt").as_deref(), Some("v1"));
    assert_eq!(world.latest().await?, FIRST);
    let outcome = world.tick().await?;
    assert!(
        matches!(
            &outcome,
            EntrySync::Ran {
                read: None,
                published: None
            }
        ),
        "{outcome:?}"
    );
    Ok(())
}

#[test(tokio::test)]
async fn after_data_toml_is_edited_commits_not_pushed_are_pushed_before_the_read() -> Res {
    let world = World::filled(&[("a.txt", b"v1"), ("b.txt", b"v1")]).await?;
    world.write("a.txt", "v2")?;
    stopping_at("push.object", 0, world.tick()).await?;
    let commit = world.locked().await?.ok_or("no lock entry")?.commits;
    world.remove("b.txt")?;
    world.contract("# edited\nsync = [\"team/raw\"]")?;

    let synced = world.sync().await?;

    assert!(synced.read_only, "{synced:?}");
    assert_eq!(world.latest().await?, commit[0]);
    assert_eq!(
        world.revision(&commit[0]).await?,
        files(&[("a.txt", "v2"), ("b.txt", "v1")])
    );
    assert_eq!(world.text("b.txt").as_deref(), Some("v1"));
    let locked = world.locked().await?.ok_or("no lock entry")?;
    assert_eq!(locked.base, commit[0]);
    assert_eq!(locked.commits, Vec::<String>::new());
    Ok(())
}

#[test(tokio::test)]
async fn an_edit_to_data_toml_that_read_all_saw_is_still_read_whole_by_the_tick() -> Res {
    let world = World::new("read = [\"team/ref\"]\nsync = [\"team/raw\"]")?;
    world
        .publish_to("team/ref", THIRD, &[("r.txt", b"ref")])
        .await?;
    world
        .publish(FIRST, &[("a.txt", b"v1"), ("b.txt", b"v1")])
        .await?;
    world.read_all().await?;
    world.sync().await?;
    assert!(world.lock().await?.header.filled.is_some());
    world.remove("a.txt")?;
    world.contract("# edited\nread = [\"team/ref\"]\nsync = [\"team/raw\"]")?;

    world.read_all().await?;
    let synced = world.sync().await?;

    assert!(synced.read_only, "{synced:?}");
    assert_eq!(world.text("a.txt").as_deref(), Some("v1"));
    assert_eq!(world.latest().await?, FIRST);
    Ok(())
}

#[test(tokio::test)]
async fn after_data_toml_is_edited_an_entry_read_whole_keeps_its_base_while_another_is_blocked()
-> Res {
    let world = World::new("sync = [\"team/raw\", \"team/ref\"]")?;
    world.publish(FIRST, &[("a.txt", b"v1")]).await?;
    world
        .publish_to("team/ref", THIRD, &[("r.txt", b"v1")])
        .await?;
    world.sync().await?;
    world.publish(SECOND, &[("a.txt", b"v2")]).await?;
    world
        .publish_to("team/ref", FOURTH, &[("r.txt", b"v2")])
        .await?;
    std::fs::write(world.dataspace().join("team/ref/r.txt"), "mine")?;
    world.contract("# edited\nsync = [\"team/raw\", \"team/ref\"]")?;

    let synced = world.sync().await?;

    assert!(synced.read_only, "{synced:?}");
    assert_eq!(world.text("a.txt").as_deref(), Some("v2"));
    let lock = world.lock().await?;
    assert_eq!(
        lock.packages.get(&key()).map(|locked| locked.base.as_str()),
        Some(SECOND)
    );
    let contract = Contract::read(&world.storage, &world.dataspace(), None).await?;
    assert_ne!(lock.header.data_toml_hash, contract.data_toml_hash());
    Ok(())
}

#[test(tokio::test)]
async fn after_data_toml_is_edited_a_commit_whose_latest_is_gone_is_not_pushed() -> Res {
    let world = World::filled(&[("a.txt", b"v1")]).await?;
    world.write("a.txt", "v2")?;
    stopping_at("push.object", 0, world.tick()).await?;
    world
        .remote
        .storage
        .remove_file(LATEST.strip_prefix("s3://").ok_or("an s3 URI")?)
        .await?;
    world.contract("# edited\nsync = [\"team/raw\"]")?;

    let outcome = world.tick().await?;

    assert!(
        matches!(&outcome, EntrySync::Diverged { latest: None }),
        "{outcome:?}"
    );
    assert!(world.remote.current(LATEST).await?.is_none());
    Ok(())
}

#[test(tokio::test)]
async fn a_revision_read_after_a_push_finds_the_metadata_file_unchanged() -> Res {
    let world = World::filled(&[("a.txt", b"v1"), ("b.txt", b"v1")]).await?;
    world.write("a.txt", "v2")?;
    advanced(&world.tick().await?);
    let theirs = ManifestHeader {
        message: crate::manifest::HeaderMessage::Text("theirs".to_string()),
        ..ManifestHeader::default()
    };
    world
        .publish_with(
            "team/raw",
            SECOND,
            &[("a.txt", b"v2"), ("b.txt", b"v2")],
            theirs,
            Vec::new(),
        )
        .await?;

    let outcome = world.tick().await?;

    assert!(
        matches!(&outcome, EntrySync::Ran { read: Some(read), .. } if read.revision == SECOND),
        "{outcome:?}"
    );
    assert_eq!(world.text("b.txt").as_deref(), Some("v2"));
    Ok(())
}

#[test(tokio::test)]
async fn a_change_undone_before_its_push_releases_the_commits() -> Res {
    let world = World::filled(&[("a.txt", b"v1")]).await?;
    world.write("a.txt", "v2")?;
    let base = advanced(&world.tick().await?);
    world.write("a.txt", "v3")?;
    stopping_at("push.object", 0, world.tick()).await?;
    world.write("a.txt", "v2")?;

    let outcome = world.tick().await?;

    assert!(
        matches!(
            &outcome,
            EntrySync::Ran {
                published: None,
                ..
            }
        ),
        "{outcome:?}"
    );
    let locked = world.locked().await?.ok_or("no lock entry")?;
    assert_eq!(locked.base, base);
    assert_eq!(locked.commits, Vec::<String>::new());
    Ok(())
}

#[test(tokio::test)]
async fn a_folder_turned_into_a_file_loses_the_folders_metadata() -> Res {
    let world = World::synced()?;
    let folder = crate::manifest::DirMeta::new(
        LogicalKey::new("dir")?,
        serde_json::json!({"user_meta": {"units": "m"}}),
    );
    world
        .publish_with(
            "team/raw",
            FIRST,
            &[("dir/a.txt", b"v1"), ("b.txt", b"v1")],
            ManifestHeader::default(),
            vec![folder],
        )
        .await?;
    world.sync().await?;
    std::fs::remove_dir_all(world.folder().join("dir"))?;
    world.write("dir", "now a file")?;

    let outcome = world.tick().await?;

    let revision = advanced(&outcome);
    assert_eq!(
        world.revision(&revision).await?,
        files(&[("b.txt", "v1"), ("dir", "now a file")])
    );
    Ok(())
}

#[test(tokio::test)]
async fn metadata_of_a_folder_that_held_no_files_is_no_change() -> Res {
    let world = World::synced()?;
    let folder = crate::manifest::DirMeta::new(
        LogicalKey::new("empty")?,
        serde_json::json!({"user_meta": {"units": "m"}}),
    );
    world
        .publish_with(
            "team/raw",
            FIRST,
            &[("a.txt", b"v1")],
            ManifestHeader::default(),
            vec![folder],
        )
        .await?;
    world.sync().await?;

    let outcome = world.tick().await?;

    assert!(
        matches!(
            &outcome,
            EntrySync::Ran {
                read: None,
                published: None
            }
        ),
        "{outcome:?}"
    );
    assert_eq!(world.latest().await?, FIRST);
    Ok(())
}

#[test(tokio::test)]
async fn a_package_another_process_holds_is_not_filled() -> Res {
    let world = World::synced()?;
    world.publish(FIRST, &[("a.txt", b"v1")]).await?;
    let store = world.domain().registry("b")?;
    let _held = world
        .storage
        .lock_exclusive(store.package_lock(&package()))
        .await?;

    let outcome = world.tick().await?;

    assert!(matches!(outcome, EntrySync::Busy), "{outcome:?}");
    assert_eq!(world.text("a.txt"), None);
    Ok(())
}

#[test(tokio::test)]
async fn a_fill_that_does_not_read_every_entry_whole_leaves_the_dataspace_unfilled() -> Res {
    let world = World::filled(&[("a.txt", b"v1")]).await?;
    // As a lock carried from another checkout is taken up.
    let contract = Contract::read(&world.storage, &world.dataspace(), None).await?;
    let mut lock = world.lock().await?;
    lock.header.filled = None;
    lock.write(&world.storage, &contract, &writer()).await?;
    world.publish(SECOND, &[("a.txt", b"v2")]).await?;
    world.write("a.txt", "mine")?;

    let synced = world.sync().await?;

    assert!(synced.read_only, "{synced:?}");
    assert!(world.lock().await?.header.filled.is_none());
    Ok(())
}

#[test(tokio::test)]
async fn a_dataspace_is_not_filled_while_a_read_entry_is_unread() -> Res {
    let world = World::new("read = [\"team/ref\"]\nsync = [\"team/raw\"]")?;
    world
        .publish_to("team/ref", THIRD, &[("r.txt", b"ref")])
        .await?;
    world.publish(FIRST, &[("a.txt", b"v1")]).await?;

    world.sync().await?;
    world.write("a.txt", "v2")?;
    let synced = world.sync().await?;

    assert!(synced.read_only);
    assert!(world.lock().await?.header.filled.is_none());
    assert_eq!(world.latest().await?, FIRST);
    Ok(())
}

#[test(tokio::test)]
async fn a_read_entry_with_no_revision_does_not_keep_the_dataspace_unfilled() -> Res {
    let world = World::new("read = [\"team/ref\"]\nsync = [\"team/raw\"]")?;
    world.publish(FIRST, &[("a.txt", b"v1")]).await?;

    let synced = world.sync().await?;

    assert!(synced.read_only);
    assert!(world.lock().await?.header.filled.is_some());
    Ok(())
}

#[test(tokio::test)]
async fn a_pinned_read_entry_keeps_the_dataspace_unfilled_until_it_is_read() -> Res {
    let world = World::new(&format!(
        "read = [{{ package = \"team/ref\", revision = \"{THIRD}\" }}]\nsync = [\"team/raw\"]"
    ))?;
    world.publish(FIRST, &[("a.txt", b"v1")]).await?;

    world.sync().await?;

    assert!(world.lock().await?.header.filled.is_none());
    Ok(())
}

const WITH_LOCAL: &str = "sync = [\"team/raw\", \"quilt+local://lab-mac#package=me/out\"]";

#[test(tokio::test)]
async fn a_local_registrys_entry_is_refused_and_keeps_the_dataspace_unfilled() -> Res {
    let world = World::new(WITH_LOCAL)?;
    world.publish(FIRST, &[("a.txt", b"v1")]).await?;

    let synced = world.sync().await?;

    let [(_, raw), (_, local)] = synced.entries.as_slice() else {
        panic!("{synced:?}");
    };
    assert!(matches!(raw, EntrySync::Ran { .. }), "{raw:?}");
    assert!(
        matches!(local, EntrySync::Failed(EntryError::LocalRegistry)),
        "{local:?}"
    );
    assert_eq!(world.text("a.txt").as_deref(), Some("v1"));
    assert!(world.lock().await?.header.filled.is_none());
    Ok(())
}

#[test(tokio::test)]
async fn a_filled_tick_refuses_a_local_registrys_entry_and_publishes_the_rest() -> Res {
    let world = World::new(WITH_LOCAL)?;
    world.publish(FIRST, &[("a.txt", b"v1")]).await?;
    world.sync().await?;
    // As a lock carried from another checkout, filled for this contract.
    let contract = Contract::read(&world.storage, &world.dataspace(), None).await?;
    let mut lock = world.lock().await?;
    lock.header.filled = Some(Utc::now().trunc_subsecs(0));
    lock.write_for(
        &world.storage,
        &contract,
        &writer(),
        contract.data_toml_hash(),
    )
    .await?;
    world.write("a.txt", "v2")?;

    let synced = world.sync().await?;

    assert!(!synced.read_only, "{synced:?}");
    let [(_, raw), (_, local)] = synced.entries.as_slice() else {
        panic!("{synced:?}");
    };
    advanced(raw);
    assert!(
        matches!(local, EntrySync::Failed(EntryError::LocalRegistry)),
        "{local:?}"
    );
    Ok(())
}

#[test(tokio::test)]
async fn a_file_named_as_a_folder_that_held_none_takes_its_place() -> Res {
    let world = World::synced()?;
    let folder = crate::manifest::DirMeta::new(
        LogicalKey::new("empty")?,
        serde_json::json!({"user_meta": {"units": "m"}}),
    );
    world
        .publish_with(
            "team/raw",
            FIRST,
            &[("a.txt", b"v1")],
            ManifestHeader::default(),
            vec![folder],
        )
        .await?;
    world.sync().await?;
    world.write("empty", "now a file")?;

    let outcome = world.tick().await?;

    let revision = advanced(&outcome);
    assert_eq!(
        world.revision(&revision).await?,
        files(&[("a.txt", "v1"), ("empty", "now a file")])
    );
    Ok(())
}

#[test(tokio::test)]
async fn a_package_with_no_revision_is_published_once_the_dataspace_is_filled() -> Res {
    let world = World::synced()?;
    world.write("a.txt", "first")?;

    let filling = world.sync().await?;
    let outcome = world.tick().await?;

    assert!(filling.read_only);
    let revision = advanced(&outcome);
    assert_eq!(
        world.revision(&revision).await?,
        files(&[("a.txt", "first")])
    );
    assert_eq!(
        world.locked().await?.map(|locked| locked.base),
        Some(revision)
    );
    Ok(())
}

/// The publish leaves a link out, as a commit does; it deletes nothing.
#[cfg(unix)]
#[test(tokio::test)]
async fn a_link_where_a_file_was_is_no_deletion() -> Res {
    let world = World::filled(&[("a.txt", b"v1"), ("b.txt", b"v1")]).await?;
    world.remove("a.txt")?;
    std::os::unix::fs::symlink(
        world.dir.path().join("elsewhere"),
        world.folder().join("a.txt"),
    )?;
    world.write("b.txt", "v2")?;

    let outcome = world.tick().await?;

    let revision = advanced(&outcome);
    assert_eq!(
        world.revision(&revision).await?,
        files(&[("a.txt", "v1"), ("b.txt", "v2")])
    );
    Ok(())
}

#[cfg(unix)]
#[test(tokio::test)]
async fn a_link_where_a_folder_was_deletes_none_of_its_files() -> Res {
    let world = World::filled(&[("data/a.txt", b"v1"), ("b.txt", b"v1")]).await?;
    let elsewhere = world.dir.path().join("elsewhere");
    std::fs::rename(world.folder().join("data"), &elsewhere)?;
    std::os::unix::fs::symlink(&elsewhere, world.folder().join("data"))?;
    world.write("b.txt", "v2")?;

    let outcome = world.tick().await?;

    let revision = advanced(&outcome);
    assert_eq!(
        world.revision(&revision).await?,
        files(&[("b.txt", "v2"), ("data/a.txt", "v1")])
    );
    Ok(())
}

#[test(tokio::test)]
async fn a_package_with_no_revision_and_no_folder_publishes_nothing() -> Res {
    let world = World::synced()?;
    world.sync().await?;

    let outcome = world.tick().await?;

    assert!(
        matches!(
            &outcome,
            EntrySync::Ran {
                read: None,
                published: None
            }
        ),
        "{outcome:?}"
    );
    Ok(())
}

#[test(tokio::test)]
async fn a_change_the_buckets_workflow_refuses_is_stored_kept_and_not_uploaded_again() -> Res {
    let world = World::filled(&[("a.txt", b"v1")]).await?;
    gate(&world, r#"{"type": "object", "required": ["owner"]}"#).await?;
    world.write("a.txt", "v2")?;

    let outcome = world.tick().await?;

    let EntrySync::Ran {
        published: Some(Published::WorkflowBlocked { revision, .. }),
        ..
    } = &outcome
    else {
        panic!("expected the revision stored and blocked, got {outcome:?}");
    };
    assert_eq!(world.revision(revision).await?, files(&[("a.txt", "v2")]));
    assert_eq!(world.latest().await?, FIRST);
    assert_eq!(
        world.locked().await?.map(|locked| locked.base).as_deref(),
        Some(FIRST)
    );
    assert_eq!(
        world.kept().await?,
        Some(KeptRevision {
            hash: revision.clone(),
            base: Some(FIRST.to_string()),
            reason: KeptReason::WorkflowBlocked,
        })
    );

    // Overwritten in the bucket, the file would be put back by an upload.
    let stored = fetch_jsonl(
        &world.remote,
        &ManifestUri {
            origin: None,
            bucket: "b".to_string(),
            namespace: package(),
            hash: revision.clone(),
        },
    )
    .await?;
    let object = &stored.rows.first().ok_or("no row")?.physical_key;
    world
        .remote
        .put_object(
            None,
            &S3Uri::try_from(object.as_str())?,
            b"overwritten".to_vec(),
        )
        .await?;
    let again = world.tick().await?;

    assert!(
        matches!(&again, EntrySync::Ran {
            published: Some(Published::WorkflowBlocked { revision: again, .. }),
            ..
        } if again == revision),
        "{again:?}"
    );
    assert_eq!(
        world.revision(revision).await?,
        files(&[("a.txt", "overwritten")])
    );
    assert_eq!(world.latest().await?, FIRST);
    Ok(())
}

#[test(tokio::test)]
async fn a_kept_revision_the_bucket_no_longer_holds_is_stored_again() -> Res {
    let world = World::filled(&[("a.txt", b"v1")]).await?;
    world.write("a.txt", "v2")?;
    let gate = world.remote.park(LATEST);
    let rival = async {
        gate.arrived().await;
        let push = world.remote.park(LATEST);
        gate.release();
        push.arrived().await;
        world.publish(SECOND, &[("a.txt", b"theirs")]).await?;
        push.release();
        Res::Ok(())
    };
    let (outcome, rivalled) = tokio::join!(world.tick(), rival);
    rivalled?;
    let EntrySync::Ran {
        published: Some(Published::Stored { revision, .. }),
        ..
    } = &outcome?
    else {
        panic!("expected the revision stored");
    };
    let revision = revision.clone();
    let stored = format!("b/.quilt/packages/{revision}");
    world.remote.storage.remove_file(&stored).await?;
    // `latest` back on the base, so the kept commit is pushed again.
    world.publish(FIRST, &[("a.txt", b"v1")]).await?;

    let outcome = world.tick().await?;

    assert_eq!(advanced(&outcome), revision);
    assert!(world.remote.current(&stored).await?.is_some());
    assert_eq!(world.revision(&revision).await?, files(&[("a.txt", "v2")]));
    Ok(())
}

#[test(tokio::test)]
async fn a_workflow_that_cannot_be_checked_fails_the_push_before_anything_is_uploaded() -> Res {
    let world = World::filled(&[("a.txt", b"v1")]).await?;
    gate(&world, r#"{"type": "object"}"#).await?;
    world.write("a.txt", "v2")?;
    stopping_at("push.object", 0, world.tick()).await?;
    let commits = world.locked().await?.ok_or("no lock entry")?.commits;
    assert_eq!(commits.len(), 1);
    // A schema the validator does not support, not a refusal of the revision.
    gate(&world, r##"{"$ref": "#/definitions/meta"}"##).await?;

    let outcome = world.tick().await?;

    assert!(matches!(&outcome, EntrySync::Failed(_)), "{outcome:?}");
    let stored = format!("b/.quilt/packages/{}", commits[0]);
    assert!(world.remote.current(&stored).await?.is_none());
    assert_eq!(world.kept().await?, None);
    Ok(())
}

#[test(tokio::test)]
async fn a_blocked_commit_the_store_cleared_is_read_from_the_bucket() -> Res {
    let world = World::filled(&[("a.txt", b"v1")]).await?;
    gate(&world, r#"{"type": "object", "required": ["owner"]}"#).await?;
    world.write("a.txt", "v2")?;
    world.tick().await?;
    // The bucket holds the blocked revision, so the store counts it published.
    world
        .domain()
        .registry("b")?
        .clear_published(&world.storage, &world.remote, None)
        .await?;

    let outcome = world.tick().await?;

    assert!(
        matches!(
            &outcome,
            EntrySync::Ran {
                published: Some(Published::WorkflowBlocked { .. }),
                ..
            }
        ),
        "{outcome:?}"
    );
    Ok(())
}

#[test(tokio::test)]
async fn a_read_back_stopped_partway_leaves_the_folder_empty_for_the_next_tick() -> Res {
    let world = World::filled(&[("a.txt", b"v1"), ("b.txt", b"v1")]).await?;
    std::fs::remove_dir_all(world.folder())?;

    let stopped = stopping_at("sync.read_back", 0, world.tick()).await?;

    assert!(matches!(stopped, EntrySync::Failed(_)), "{stopped:?}");
    assert_eq!(world.text("a.txt"), None);
    assert_eq!(world.text("b.txt"), None);
    let outcome = world.tick().await?;
    assert!(matches!(&outcome, EntrySync::ReadBack(_)), "{outcome:?}");
    assert_eq!(world.text("b.txt").as_deref(), Some("v1"));
    assert_eq!(world.latest().await?, FIRST);
    Ok(())
}

#[test(tokio::test)]
async fn after_data_toml_is_edited_a_blocked_commit_is_read_whole_and_kept() -> Res {
    let world = World::filled(&[("a.txt", b"v1"), ("b.txt", b"v1")]).await?;
    gate(&world, r#"{"type": "object", "required": ["owner"]}"#).await?;
    world.write("a.txt", "v2")?;
    world.tick().await?;
    let commits = world.locked().await?.ok_or("no lock entry")?.commits;
    assert_eq!(commits.len(), 1);
    world.remove("b.txt")?;
    world.contract("# edited\nsync = [\"team/raw\"]")?;

    let synced = world.sync().await?;

    assert!(synced.read_only, "{synced:?}");
    assert_eq!(world.text("a.txt").as_deref(), Some("v2"));
    assert_eq!(world.text("b.txt").as_deref(), Some("v1"));
    let lock = world.lock().await?;
    let contract = Contract::read(&world.storage, &world.dataspace(), None).await?;
    assert_eq!(lock.header.data_toml_hash, contract.data_toml_hash());
    let locked = lock.packages.get(&key()).ok_or("no lock entry")?;
    assert_eq!(locked.base, FIRST);
    assert_eq!(locked.commits, commits);
    Ok(())
}

#[test(tokio::test)]
async fn a_folder_that_cannot_be_read_back_whole_is_left_empty_and_nothing_published() -> Res {
    let world = World::filled(&[("a.txt", b"v1"), ("b.txt", b"v1")]).await?;
    std::fs::remove_dir_all(world.folder())?;
    // b.txt's bytes in the bucket no longer match its row.
    world
        .remote
        .put_object(
            None,
            &S3Uri::try_from(
                format!("s3://b/team/raw/b.txt?versionId={}", hex::encode(b"v1")).as_str(),
            )?,
            b"corrupt".to_vec(),
        )
        .await?;

    let first = world.tick().await?;
    let second = world.tick().await?;

    for outcome in [first, second] {
        assert!(
            matches!(&outcome, EntrySync::ReadBack(read)
                if read.placed.is_empty() && read.hash_mismatch == [PathBuf::from("b.txt")]),
            "{outcome:?}"
        );
    }
    assert_eq!(world.text("a.txt"), None);
    assert_eq!(world.latest().await?, FIRST);
    Ok(())
}

/// Makes the bucket's default workflow `gate`, whose metadata schema is
/// `schema`.
async fn gate(world: &World, schema: &str) -> Res {
    let config = "version: \"1\"\ndefault_workflow: gate\nworkflows:\n  gate:\n    name: Gate\n    metadata_schema: meta\nschemas:\n  meta:\n    url: s3://b/schemas/meta.json\n";
    for (key, body) in [
        ("s3://b/.quilt/workflows/config.yml", config),
        ("s3://b/schemas/meta.json", schema),
    ] {
        world
            .remote
            .put_object(None, &S3Uri::try_from(key)?, body.as_bytes().to_vec())
            .await?;
    }
    Ok(())
}

#[test(tokio::test)]
async fn a_package_another_process_holds_is_left_for_the_next_tick() -> Res {
    let world = World::filled(&[("a.txt", b"v1")]).await?;
    world.write("a.txt", "v2")?;
    let store = world.domain().registry("b")?;
    let _held = world
        .storage
        .lock_exclusive(store.package_lock(&package()))
        .await?;

    let outcome = world.tick().await?;

    assert!(matches!(outcome, EntrySync::Busy), "{outcome:?}");
    assert_eq!(world.latest().await?, FIRST);
    Ok(())
}

/// Each point a tick that reads and publishes passes, and how many times.
const TRANSITIONS: &[(&str, usize)] = &[
    // Placing b.txt and c.txt, removing x.txt.
    ("read.file", 3),
    // b.txt, c.txt and d.txt into the Domain's store.
    ("record.object", 3),
    ("sync.recorded", 1),
    ("push.object", 3),
    ("push.stored", 1),
    ("sync.pushed", 1),
];

/// A filled dataspace whose `latest` moved and whose folder changed.
async fn changed_both_ways() -> Result<World, Box<dyn std::error::Error>> {
    let world = World::filled(&[("a.txt", b"v1"), ("b.txt", b"v1"), ("x.txt", b"v1")]).await?;
    world
        .publish(
            SECOND,
            &[("a.txt", b"v1"), ("b.txt", b"v2"), ("c.txt", b"v1")],
        )
        .await?;
    world.remove("a.txt")?;
    world.write("d.txt", "added")?;
    Ok(world)
}

#[test(tokio::test)]
async fn a_tick_stopped_at_any_file_manifest_or_lock_step_ends_as_one_that_was_not() -> Res {
    let uninterrupted = changed_both_ways().await?;
    let expected = advanced(&uninterrupted.tick().await?);
    let expected_files = uninterrupted.revision(&expected).await?;
    assert_eq!(
        expected_files,
        files(&[("b.txt", "v2"), ("c.txt", "v1"), ("d.txt", "added")])
    );

    for (point, passes) in TRANSITIONS {
        for pass in 0..*passes {
            let world = changed_both_ways().await?;
            let stopped = stopping_at(point, pass, world.tick()).await?;
            assert!(
                matches!(stopped, EntrySync::Failed(_)),
                "{point} #{pass} did not stop the tick: {stopped:?}"
            );

            // As many ticks as it takes, but no more than the steps left.
            for _ in 0..2 {
                world.tick().await?;
            }

            let context = format!("stopped at {point} #{pass}");
            assert_eq!(world.latest().await?, expected, "{context}");
            let locked = world.locked().await?.ok_or("no lock entry")?;
            assert_eq!(locked.base, expected, "{context}");
            assert_eq!(locked.commits, Vec::<String>::new(), "{context}");
            for (key, body) in &expected_files {
                assert_eq!(world.text(key).as_ref(), Some(body), "{context}: {key}");
            }
            assert_eq!(world.text("a.txt"), None, "{context}");
            assert_eq!(world.text("x.txt"), None, "{context}");
            assert!(world.folder().join(METADATA_FILE).exists(), "{context}");
        }
    }
    Ok(())
}
