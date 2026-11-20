use super::*;

use std::time::Duration;

use tempfile::TempDir;
use test_log::test;

use crate::LogicalKey;
use crate::io::remote::mocks::MockRemote;
use crate::io::storage::LocalStorage;
use crate::manifest::Manifest;
use crate::manifest::ManifestRow;
use crate::manifest::RawNumbers;
use crate::queue::Failure;
use crate::state_store::FolderKey;
use quilt_uri::S3Uri;

type Res = Result<(), Box<dyn std::error::Error>>;
type Got<T> = Result<T, Box<dyn std::error::Error>>;

const LATEST: &str = "s3://b/.quilt/named_packages/team/raw/latest";
const FIRST: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const SECOND: &str = "2222222222222222222222222222222222222222222222222222222222222222";
const THIRD: &str = "3333333333333333333333333333333333333333333333333333333333333333";

/// A dataspace on autosync holding `team/raw` of bucket `b`, its Domain and
/// queue beside it, and the bucket on a mock remote.
struct World {
    dir: TempDir,
    remote: MockRemote,
    storage: LocalStorage,
}

impl World {
    /// A dataspace whose `[packages]` table is `packages`, on autosync.
    async fn new(packages: &str) -> Got<Self> {
        let world = Self {
            dir: TempDir::new()?,
            remote: MockRemote::default(),
            storage: LocalStorage::new(),
        };
        std::fs::create_dir_all(world.dataspace())?;
        std::fs::write(
            world.dataspace().join("data.toml"),
            format!("registry = \"s3://b\"\n\n[packages]\n{packages}"),
        )?;
        DomainRoot::new(world.dir.path().join("domain"))
            .set_autosync(&world.storage, &world.dataspace(), true)
            .await?;
        Ok(world)
    }

    fn dataspace(&self) -> PathBuf {
        self.dir.path().join("rnaseq")
    }

    fn folder(&self) -> PathBuf {
        self.dataspace().join("team/raw")
    }

    /// The runner, opened again for each tick as a new process would.
    async fn runner(&self) -> Got<Runner> {
        let settings = Settings {
            domain: DomainRoot::new(self.dir.path().join("domain")),
            writer: Writer {
                host: "here".to_string(),
                user: "me".to_string(),
                local_registry: String::new(),
            },
            user_rules: self.dir.path().join("user.quiltignore"),
            message: "autosync".to_string(),
        };
        Ok(Runner::open(&self.dir.path().join("queue.db"), settings).await?)
    }

    /// One tick at `now`, in the one dataspace.
    async fn tick_at(&self, now: SystemTime) -> Got<DataspaceTick> {
        let runner = self.runner().await?;
        let mut tick = runner.tick(&self.storage, &self.remote, now).await?;
        assert_eq!(tick.dataspaces.len(), 1, "{tick:?}");
        Ok(tick.dataspaces.remove(0))
    }

    async fn tick(&self) -> Got<DataspaceTick> {
        self.tick_at(SystemTime::now()).await
    }

    /// What the watcher would report: `key` in the folder changed.
    async fn changed(&self, key: &str) -> Got<Vec<JobKey>> {
        let runner = self.runner().await?;
        Ok(runner
            .changed(&self.storage, &self.contract_folder().await?.join(key))
            .await?)
    }

    /// The folder as the contract spells it, which on Windows is not
    /// `folder()`'s.
    async fn contract_folder(&self) -> Got<PathBuf> {
        let contract = Contract::read(&self.storage, &self.dataspace(), None).await?;
        Ok(contract.entries().first().ok_or("no entry")?.folder.clone())
    }

    async fn folder_key(&self) -> Got<FolderKey> {
        let contract = Contract::read(&self.storage, &self.dataspace(), None).await?;
        Ok(folder_key(contract.entries().first().ok_or("no entry")?))
    }

    async fn jobs(&self) -> Got<usize> {
        Ok(self.runner().await?.queue().jobs().await?.len())
    }

    async fn kept(&self) -> Got<Option<KeptRevision>> {
        let folder = self.folder_key().await?;
        Ok(self.runner().await?.queue().kept(&folder).await?)
    }

    async fn keep(&self, revision: KeptRevision) -> Res {
        let folder = self.folder_key().await?;
        self.runner().await?.queue().keep(&folder, revision).await?;
        Ok(())
    }

    async fn failure(&self, access: Access) -> Got<Option<Failure>> {
        let key = JobKey {
            folder: self.folder_key().await?,
            access,
        };
        Ok(self.runner().await?.queue().failure(&key).await?)
    }

    /// Stores `files` as revision `hash` of `team/raw` and tags it `latest`.
    async fn publish(&self, hash: &str, files: &[(&str, &[u8])]) -> Res {
        self.publish_to("team/raw", hash, files).await
    }

    /// [`World::publish`] for `package`.
    async fn publish_to(&self, package: &str, hash: &str, files: &[(&str, &[u8])]) -> Res {
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
        let manifest = Manifest {
            rows,
            ..Manifest::default()
        };
        let stored = format!("s3://b/.quilt/packages/{hash}");
        self.put(&stored, manifest.to_jsonlines().into_bytes())
            .await?;
        let latest = format!("s3://b/.quilt/named_packages/{package}/latest");
        self.put(&latest, hash.as_bytes().to_vec()).await
    }

    async fn filled(&self) -> Got<bool> {
        let contract = Contract::read(&self.storage, &self.dataspace(), None).await?;
        let lock = Lock::read(&self.storage, &contract).await?;
        Ok(lock.ok_or("no lock")?.header.filled.is_some())
    }

    async fn put(&self, uri: &str, body: Vec<u8>) -> Res {
        self.remote
            .put_object(None, &S3Uri::try_from(uri)?, body)
            .await?;
        Ok(())
    }

    async fn latest(&self) -> Got<Option<String>> {
        let bytes = self.remote.current(LATEST).await?;
        Ok(bytes.map(String::from_utf8).transpose()?)
    }

    async fn base(&self) -> Got<Option<String>> {
        let contract = Contract::read(&self.storage, &self.dataspace(), None).await?;
        let Some(lock) = Lock::read(&self.storage, &contract).await? else {
            return Ok(None);
        };
        let entry = contract.entries().first().ok_or("no entry")?;
        Ok(lock.packages.get(&entry.key()).and_then(|l| l.base.clone()))
    }

    fn write(&self, key: &str, body: &str) -> std::io::Result<()> {
        let path = self.folder().join(key);
        std::fs::create_dir_all(path.parent().expect("a folder"))?;
        std::fs::write(path, body)
    }

    fn text(&self, key: &str) -> Option<String> {
        std::fs::read_to_string(self.folder().join(key)).ok()
    }
}

/// The tick's one entry.
fn only(tick: &DataspaceTick) -> &EntryTick {
    assert!(tick.errors.is_empty(), "{tick:?}");
    assert_eq!(tick.entries.len(), 1, "{tick:?}");
    &tick.entries[0].1
}

fn advanced(outcome: &EntryTick) -> String {
    match outcome {
        EntryTick::Published(EntryPublish::Ran(Published::Advanced { revision, .. }))
        | EntryTick::Synced(EntrySync::Ran {
            published: Some(Published::Advanced { revision, .. }),
            ..
        }) => revision.clone(),
        other => panic!("expected a publish that advanced latest, got {other:?}"),
    }
}

#[test(tokio::test)]
async fn a_read_entry_is_read_once_latest_moves_off_the_lock_base() -> Res {
    let world = World::new("read = [\"team/raw\"]").await?;
    world.publish(FIRST, &[("a.txt", b"v1")]).await?;

    let first = world.tick().await?;
    assert!(
        matches!(only(&first), EntryTick::Read(read) if read.completed()),
        "{first:?}"
    );
    assert_eq!(world.text("a.txt").as_deref(), Some("v1"));
    assert_eq!(world.base().await?.as_deref(), Some(FIRST));
    assert_eq!(world.jobs().await?, 0);

    let unmoved = world.tick().await?;
    assert!(matches!(only(&unmoved), EntryTick::Idle), "{unmoved:?}");

    world.publish(SECOND, &[("a.txt", b"v2")]).await?;
    let moved = world.tick().await?;
    assert!(
        matches!(only(&moved), EntryTick::Read(read) if read.completed()),
        "{moved:?}"
    );
    assert_eq!(world.text("a.txt").as_deref(), Some("v2"));
    assert_eq!(world.base().await?.as_deref(), Some(SECOND));
    Ok(())
}

#[test(tokio::test)]
async fn a_dataspace_is_filled_only_once_every_read_entry_is_read() -> Res {
    let world = World::new("read = [\"team/raw\", \"team/ref\"]").await?;
    world.publish(FIRST, &[("a.txt", b"v1")]).await?;

    let raw = world.tick().await?;
    assert!(
        matches!(
            &raw.entries[..],
            [(_, EntryTick::Read(_)), (_, EntryTick::Idle)]
        ),
        "{raw:?}"
    );
    assert!(!world.filled().await?);

    world
        .publish_to("team/ref", SECOND, &[("b.txt", b"v1")])
        .await?;
    let reference = world.tick().await?;
    assert!(
        matches!(
            &reference.entries[..],
            [(_, EntryTick::Idle), (_, EntryTick::Read(_))]
        ),
        "{reference:?}"
    );
    assert!(world.filled().await?);
    Ok(())
}

#[test(tokio::test)]
async fn a_write_entry_publishes_what_the_watcher_reports_changed() -> Res {
    let world = World::new("write = [\"team/raw\"]").await?;
    world.write("a.txt", "mine")?;

    let unreported = world.tick().await?;
    assert!(
        matches!(only(&unreported), EntryTick::Idle),
        "{unreported:?}"
    );
    assert_eq!(world.latest().await?, None);

    assert_eq!(world.changed("a.txt").await?.len(), 1);
    let reported = world.tick().await?;

    let revision = advanced(only(&reported));
    assert_eq!(world.latest().await?.as_deref(), Some(revision.as_str()));
    assert_eq!(world.base().await?, Some(revision));
    assert_eq!(world.jobs().await?, 0);
    Ok(())
}

#[test(tokio::test)]
async fn a_sync_entry_runs_the_tick_on_the_runners_queue() -> Res {
    let world = World::new("sync = [\"team/raw\"]").await?;
    world.publish(FIRST, &[("a.txt", b"v1")]).await?;

    let filled = world.tick().await?;
    assert!(
        matches!(
            only(&filled),
            EntryTick::Synced(EntrySync::Ran {
                read: Some(_),
                published: None
            })
        ),
        "{filled:?}"
    );
    assert_eq!(world.text("a.txt").as_deref(), Some("v1"));

    world.write("a.txt", "v2")?;
    world.changed("a.txt").await?;
    let synced = world.tick().await?;

    let revision = advanced(only(&synced));
    assert_eq!(world.latest().await?.as_deref(), Some(revision.as_str()));
    assert_eq!(world.jobs().await?, 0);
    Ok(())
}

#[test(tokio::test)]
async fn a_failed_job_waits_out_its_retry_delay_across_restarts() -> Res {
    let world = World::new("read = [\"team/raw\"]").await?;
    // `latest` names a revision the bucket does not hold.
    world.put(LATEST, FIRST.as_bytes().to_vec()).await?;
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(2_000_000_000);

    let failed = world.tick_at(now).await?;
    assert!(
        matches!(only(&failed), EntryTick::Read(EntryRead::Failed(_))),
        "{failed:?}"
    );
    let failure = world.failure(Access::Read).await?.ok_or("no failure")?;
    assert_eq!(failure.failures, 1);
    assert_eq!(failure.retry_at, now + Duration::from_secs(2));

    let waiting = world.tick_at(now + Duration::from_secs(1)).await?;
    assert!(
        matches!(only(&waiting), EntryTick::Waiting { retry_at } if *retry_at == failure.retry_at),
        "{waiting:?}"
    );

    let retried = world.tick_at(now + Duration::from_secs(3)).await?;
    assert!(
        matches!(only(&retried), EntryTick::Read(EntryRead::Failed(_))),
        "{retried:?}"
    );
    let failure = world.failure(Access::Read).await?.ok_or("no failure")?;
    assert_eq!(failure.failures, 2);
    assert_eq!(world.jobs().await?, 1);
    Ok(())
}

#[test(tokio::test)]
async fn a_package_under_read_and_write_is_read_and_not_published() -> Res {
    let world = World::new(
        "read = [{ package = \"team/raw\", path = \"in/\" }]\nwrite = [{ package = \"team/raw\", path = \"out/\" }]\n",
    )
    .await?;
    world.publish(FIRST, &[("in/a.txt", b"v1")]).await?;
    world.write("out/b.txt", "mine")?;

    assert_eq!(world.changed("out/b.txt").await?, []);
    let tick = world.tick().await?;

    assert!(tick.errors.is_empty(), "{tick:?}");
    let [(read, read_tick), (write, write_tick)] = &tick.entries[..] else {
        panic!("expected two entries, got {tick:?}");
    };
    assert_eq!((read.access, write.access), (Access::Read, Access::Write));
    assert!(
        matches!(read_tick, EntryTick::Read(read) if read.completed()),
        "{tick:?}"
    );
    assert!(
        matches!(write_tick, EntryTick::Skipped(Skip::ReadAndWrite)),
        "{tick:?}"
    );
    assert_eq!(world.text("in/a.txt").as_deref(), Some("v1"));
    assert_eq!(world.latest().await?.as_deref(), Some(FIRST));
    assert_eq!(world.jobs().await?, 0);
    Ok(())
}

#[test(tokio::test)]
async fn a_kept_revision_is_released_once_latest_names_it() -> Res {
    let world = World::new("write = [\"team/raw\"]").await?;
    world.publish(FIRST, &[("a.txt", b"v1")]).await?;
    world.write("a.txt", "v1")?;
    let kept = KeptRevision {
        hash: SECOND.to_string(),
        base: None,
        reason: KeptReason::LatestMoved,
    };
    world.keep(kept.clone()).await?;

    let unnamed = world.tick().await?;
    assert!(unnamed.released.is_empty(), "{unnamed:?}");
    assert_eq!(world.kept().await?.as_ref(), Some(&kept));

    world.publish(SECOND, &[("a.txt", b"v1")]).await?;
    let named = world.tick().await?;

    let released: Vec<&KeptRevision> = named.released.iter().map(|(_, kept)| kept).collect();
    assert_eq!(released, [&kept]);
    assert_eq!(world.kept().await?, None);
    Ok(())
}

#[test(tokio::test)]
async fn a_kept_revision_is_released_once_the_folder_moves_past_its_base() -> Res {
    let world = World::new("sync = [\"team/raw\"]").await?;
    world.publish(FIRST, &[("a.txt", b"v1")]).await?;
    world.tick().await?;
    let kept = KeptRevision {
        hash: THIRD.to_string(),
        base: Some(FIRST.to_string()),
        reason: KeptReason::LatestMoved,
    };
    world.keep(kept.clone()).await?;

    world.publish(SECOND, &[("a.txt", b"v2")]).await?;
    let read = world.tick().await?;
    assert!(read.released.is_empty(), "{read:?}");
    assert_eq!(world.base().await?.as_deref(), Some(SECOND));

    let moved = world.tick().await?;
    let released: Vec<&KeptRevision> = moved.released.iter().map(|(_, kept)| kept).collect();
    assert_eq!(released, [&kept]);
    assert_eq!(world.kept().await?, None);
    Ok(())
}

#[test(tokio::test)]
async fn a_publish_latest_moved_past_is_kept_and_its_job_done() -> Res {
    let world = World::new("replace = [\"team/raw\"]").await?;
    world.publish(FIRST, &[("a.txt", b"v1")]).await?;
    world.write("mine.txt", "mine")?;
    world.changed("mine.txt").await?;

    // The runner's look at `latest`, the publish's read of it, then its read
    // before the tag write, where a rival moves it.
    let observed = world.remote.park(LATEST);
    let rival = async {
        observed.arrived().await;
        let read = world.remote.park(LATEST);
        observed.release();
        read.arrived().await;
        let tagging = world.remote.park(LATEST);
        read.release();
        tagging.arrived().await;
        world.publish(SECOND, &[("theirs.txt", b"t")]).await?;
        tagging.release();
        Res::Ok(())
    };
    let (tick, rivalled) = tokio::join!(world.tick(), rival);
    rivalled?;
    let tick = tick?;

    let EntryTick::Published(EntryPublish::Ran(Published::Stored { revision, latest })) =
        only(&tick)
    else {
        panic!("expected the revision stored, got {tick:?}");
    };
    assert_eq!(latest.as_deref(), Some(SECOND));
    assert_eq!(
        world.kept().await?,
        Some(KeptRevision {
            hash: revision.clone(),
            base: None,
            reason: KeptReason::LatestMoved,
        })
    );
    assert_eq!(world.jobs().await?, 0);
    assert_eq!(world.failure(Access::Replace).await?, None);

    let again = world.tick().await?;
    assert!(matches!(only(&again), EntryTick::Idle), "{again:?}");
    assert_eq!(world.jobs().await?, 0);
    assert!(world.kept().await?.is_some());
    Ok(())
}

#[test(tokio::test)]
async fn an_edit_made_before_the_dataspace_is_filled_is_published_once_it_is() -> Res {
    let world = World::new("sync = [\"team/raw\"]").await?;
    world.write("b.txt", "mine")?;
    world.changed("b.txt").await?;

    let filling = world.tick().await?;
    assert!(
        matches!(
            only(&filling),
            EntryTick::Synced(EntrySync::Ran {
                published: None,
                ..
            })
        ),
        "{filling:?}"
    );
    assert_eq!(world.jobs().await?, 1);

    let synced = world.tick().await?;
    advanced(only(&synced));
    assert_eq!(world.jobs().await?, 0);
    Ok(())
}

/// Runs the sync tick outside the runner's jobs, as a tick another entry's
/// job ran, or one run by hand, would.
async fn sync_outside(world: &World) -> Got<crate::dataspace::sync::DataspaceSync> {
    let runner = world.runner().await?;
    let settings = Settings {
        domain: DomainRoot::new(world.dir.path().join("domain")),
        writer: Writer {
            host: "here".to_string(),
            user: "me".to_string(),
            local_registry: String::new(),
        },
        user_rules: world.dir.path().join("user.quiltignore"),
        message: "autosync".to_string(),
    };
    Ok(Box::pin(sync(
        &world.storage,
        &world.remote,
        &settings.domain,
        runner.queue(),
        &world.dataspace(),
        &settings.writer,
        &settings.message,
        &settings.user_rules,
    ))
    .await?)
}

/// A first push stopped once the lock holds its commit leaves no job behind;
/// the next tick queues the entry for the commit alone, with `latest`
/// unmoved and the folder unchanged, and publishes it.
#[test(tokio::test)]
async fn a_push_stopped_after_the_lock_holds_its_commit_is_published_on_the_next_tick() -> Res {
    let world = World::new("sync = [\"team/raw\"]").await?;
    world.write("a.txt", "first")?;
    assert!(sync_outside(&world).await?.read_only);
    assert!(world.filled().await?);
    let stopped = crate::failpoint::stopping_at("push.object", 0, sync_outside(&world)).await?;
    assert!(
        matches!(&stopped.entries[..], [(_, EntrySync::Failed(_))]),
        "{stopped:?}"
    );
    assert_eq!(world.latest().await?, None);
    assert_eq!(world.base().await?, None);
    assert_eq!(world.jobs().await?, 0);

    let tick = world.tick().await?;

    let revision = advanced(only(&tick));
    assert_eq!(world.latest().await?.as_deref(), Some(revision.as_str()));
    assert_eq!(world.base().await?.as_deref(), Some(revision.as_str()));
    assert_eq!(world.text("a.txt").as_deref(), Some("first"));
    assert_eq!(world.jobs().await?, 0);
    Ok(())
}

/// A commit the bucket's workflow refused stays in the lock, and is not
/// queued again for it while its kept revision holds.
#[test(tokio::test)]
async fn a_commit_the_workflow_refused_is_not_queued_again_for_its_commits() -> Res {
    let world = World::new("sync = [\"team/raw\"]").await?;
    world.publish(FIRST, &[("a.txt", b"v1")]).await?;
    world.tick().await?;
    let config = "version: \"1\"\ndefault_workflow: gate\nworkflows:\n  gate:\n    name: Gate\n    metadata_schema: meta\nschemas:\n  meta:\n    url: s3://b/schemas/meta.json\n";
    world
        .put("s3://b/.quilt/workflows/config.yml", config.into())
        .await?;
    let schema = r#"{"type": "object", "required": ["owner"]}"#;
    world.put("s3://b/schemas/meta.json", schema.into()).await?;
    world.write("a.txt", "v2")?;
    world.changed("a.txt").await?;
    let blocked = world.tick().await?;
    assert!(
        matches!(
            only(&blocked),
            EntryTick::Synced(EntrySync::Ran {
                published: Some(Published::WorkflowBlocked { .. }),
                ..
            })
        ),
        "{blocked:?}"
    );
    assert!(world.kept().await?.is_some());
    assert_eq!(world.jobs().await?, 0);

    let tick = world.tick().await?;

    assert!(matches!(only(&tick), EntryTick::Idle), "{tick:?}");
    assert_eq!(world.jobs().await?, 0);
    Ok(())
}

#[test(tokio::test)]
async fn a_folder_that_cannot_be_read_back_is_retried() -> Res {
    let world = World::new("sync = [\"team/raw\"]").await?;
    world.publish(FIRST, &[("a.txt", b"v1")]).await?;
    world.tick().await?;
    std::fs::remove_dir_all(world.folder())?;
    // The bucket's object no longer matches the manifest's hash.
    world
        .put(
            &format!("s3://b/team/raw/a.txt?versionId={}", hex::encode(b"v1")),
            b"zz".to_vec(),
        )
        .await?;
    world.changed("a.txt").await?;

    let tick = world.tick().await?;
    assert!(
        matches!(only(&tick), EntryTick::Synced(EntrySync::ReadBack(_))),
        "{tick:?}"
    );
    assert!(world.failure(Access::Sync).await?.is_some());
    assert_eq!(world.jobs().await?, 1);
    Ok(())
}
