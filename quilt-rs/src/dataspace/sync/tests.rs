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

    async fn sync(&self) -> Result<DataspaceSync, SyncError> {
        let dataspace = self.dataspace();
        Box::pin(sync(
            &self.storage,
            &self.remote,
            &self.domain(),
            &dataspace,
            &writer(),
            MESSAGE,
        ))
        .await
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
        self.publish_with_folders(package, hash, files, Vec::new())
            .await
    }

    /// [`World::publish_to`], with metadata for folders.
    async fn publish_with_folders(
        &self,
        package: &str,
        hash: &str,
        files: &[(&str, &[u8])],
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
            header: ManifestHeader::default(),
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
        let contract = Contract::read(&self.storage, &self.dataspace()).await?;
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
async fn a_folder_that_holds_none_of_the_packages_files_publishes_nothing() -> Res {
    let world = World::filled(&[("a.txt", b"v1"), ("b.txt", b"v1")]).await?;
    std::fs::remove_dir_all(world.folder())?;

    let gone = world.tick().await?;
    std::fs::create_dir_all(world.folder())?;
    let emptied = world.tick().await?;

    for outcome in [gone, emptied] {
        assert!(
            matches!(&outcome, EntrySync::Failed(EntryError::FolderGone(_))),
            "{outcome:?}"
        );
    }
    assert_eq!(world.latest().await?, FIRST);
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
    let contract = Contract::read(&world.storage, &world.dataspace()).await?;
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
async fn a_folder_turned_into_a_file_loses_the_folders_metadata() -> Res {
    let world = World::synced()?;
    let folder = crate::manifest::DirMeta::new(
        LogicalKey::new("dir")?,
        serde_json::json!({"user_meta": {"units": "m"}}),
    );
    world
        .publish_with_folders(
            "team/raw",
            FIRST,
            &[("dir/a.txt", b"v1"), ("b.txt", b"v1")],
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
async fn a_fill_that_does_not_read_every_entry_whole_leaves_the_dataspace_unfilled() -> Res {
    let world = World::filled(&[("a.txt", b"v1")]).await?;
    // As a lock carried from another checkout is taken up.
    let contract = Contract::read(&world.storage, &world.dataspace()).await?;
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
