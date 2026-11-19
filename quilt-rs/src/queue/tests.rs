use super::*;

use std::path::PathBuf;
use std::time::UNIX_EPOCH;

use test_log::test;

use crate::dataspace::contract::Registry;

type Res = Result<(), Box<dyn std::error::Error>>;

/// `acme/demo` from `bucket`, in the working folder `folder`.
fn copy(bucket: &str, folder: &str) -> FolderKey {
    FolderKey {
        registry: Some(Registry::S3(bucket.to_string())),
        package: ("acme", "demo").into(),
        folder: PathBuf::from(folder),
    }
}

fn job(folder: &str, access: Access) -> JobKey {
    JobKey {
        folder: copy("bucket", folder),
        access,
    }
}

fn at(secs: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(secs)
}

fn kept(hash: &str) -> KeptRevision {
    KeptRevision {
        hash: hash.to_string(),
        base: Some("base".to_string()),
        reason: KeptReason::LatestMoved,
    }
}

fn queue_path(dir: &tempfile::TempDir) -> PathBuf {
    dir.path().join(".quilt").join("queue.db")
}

#[test(tokio::test)]
async fn a_job_waits_runs_and_leaves_the_queue() -> Res {
    let queue = Queue::in_memory().await?;
    let key = job("/one", Access::Read);

    queue.enqueue(&key).await?;
    assert_eq!(
        queue.jobs().await?,
        [Job {
            key: key.clone(),
            state: JobState::Pending
        }]
    );

    queue.start(&key).await?;
    assert_eq!(queue.jobs().await?[0].state, JobState::Running);

    queue.finish(&key).await?;
    assert_eq!(queue.jobs().await?, []);
    Ok(())
}

fn states(jobs: &[Job]) -> Vec<(&JobKey, JobState)> {
    jobs.iter().map(|job| (&job.key, job.state)).collect()
}

/// A change seen while its job runs may come too late for the run: the job
/// runs on, then waits again in the place it had.
#[test(tokio::test)]
async fn a_job_queued_again_while_it_runs_waits_in_its_place_after_the_run() -> Res {
    let queue = Queue::in_memory().await?;
    let first = job("/one", Access::Sync);
    let second = job("/two", Access::Sync);
    queue.enqueue(&first).await?;
    queue.enqueue(&second).await?;
    queue.start(&first).await?;

    queue.enqueue(&first).await?;
    assert_eq!(
        states(&queue.jobs().await?),
        [(&first, JobState::Running), (&second, JobState::Pending)]
    );

    queue.finish(&first).await?;
    assert_eq!(
        states(&queue.jobs().await?),
        [(&first, JobState::Pending), (&second, JobState::Pending)]
    );
    Ok(())
}

/// Starting a job this queue already runs changes nothing, so a change seen
/// during the run is still run after it.
#[test(tokio::test)]
async fn starting_a_running_job_again_keeps_it_queued_again() -> Res {
    let queue = Queue::in_memory().await?;
    let key = job("/one", Access::Sync);
    queue.start(&key).await?;
    queue.enqueue(&key).await?;

    queue.start(&key).await?;
    queue.finish(&key).await?;

    assert_eq!(states(&queue.jobs().await?), [(&key, JobState::Pending)]);
    Ok(())
}

#[test(tokio::test)]
async fn a_job_that_failed_waits_for_its_retry_after_the_run() -> Res {
    let queue = Queue::in_memory().await?;
    let key = job("/one", Access::Sync);
    queue.start(&key).await?;

    queue.fail(&key, "offline", at(100)).await?;
    queue.finish(&key).await?;

    assert_eq!(states(&queue.jobs().await?), [(&key, JobState::Pending)]);
    assert_eq!(
        queue.failure(&key).await?.map(|f| f.retry_at),
        Some(at(102))
    );
    Ok(())
}

/// A newer build's reason is still a kept revision, never none.
#[test(tokio::test)]
async fn a_kept_revision_for_a_reason_this_build_doesnt_know_is_an_error() -> Res {
    let dir = tempfile::tempdir()?;
    let path = queue_path(&dir);
    let folder = copy("bucket", "/one");
    let queue = Queue::open(&path).await?;
    queue.keep(&folder, kept("aaaa")).await?;
    rusqlite::Connection::open(&path)?.execute("UPDATE kept SET reason = 'quota'", [])?;

    let read = queue.kept(&folder).await;

    assert!(
        matches!(&read, Err(QueueError::UnknownKeptReason(reason)) if reason == "quota"),
        "{read:?}"
    );
    Ok(())
}

/// Opening a file already made reads it, so another process's write does not
/// hold it up.
#[test(tokio::test)]
async fn a_queue_already_made_opens_while_another_process_writes() -> Res {
    let dir = tempfile::tempdir()?;
    let path = queue_path(&dir);
    drop(Queue::open(&path).await?);
    let other = rusqlite::Connection::open(&path)?;
    other.execute_batch("BEGIN IMMEDIATE")?;

    let opened = tokio::time::timeout(Duration::from_secs(2), Queue::open(&path)).await;

    other.execute_batch("COMMIT")?;
    assert!(matches!(opened, Ok(Ok(_))), "{opened:?}");
    Ok(())
}

#[test(tokio::test)]
async fn a_queue_from_a_newer_build_is_refused_while_another_process_writes() -> Res {
    let dir = tempfile::tempdir()?;
    let path = queue_path(&dir);
    drop(Queue::open(&path).await?);
    let other = rusqlite::Connection::open(&path)?;
    other.pragma_update(None, "user_version", 2)?;
    other.execute_batch("BEGIN IMMEDIATE")?;

    let opened = tokio::time::timeout(Duration::from_secs(2), Queue::open(&path)).await;

    other.execute_batch("COMMIT")?;
    assert!(
        matches!(opened, Ok(Err(QueueError::Version(2)))),
        "{opened:?}"
    );
    Ok(())
}

/// One package in two folders, and one folder's entry under two accesses,
/// are four jobs.
#[test(tokio::test)]
async fn jobs_are_kept_apart_by_folder_and_access() -> Res {
    let queue = Queue::in_memory().await?;
    let keys = [
        job("/one", Access::Read),
        job("/one", Access::Write),
        job("/two", Access::Read),
        job("/two", Access::Write),
    ];
    for key in &keys {
        queue.enqueue(key).await?;
    }
    queue.start(&keys[0]).await?;
    queue.finish(&keys[0]).await?;

    let left: Vec<JobKey> = queue.jobs().await?.into_iter().map(|j| j.key).collect();
    assert_eq!(left, keys[1..]);
    Ok(())
}

#[test(tokio::test)]
async fn a_job_running_when_the_process_ended_is_interrupted_after_a_restart() -> Res {
    let dir = tempfile::tempdir()?;
    let path = queue_path(&dir);
    let running = job("/one", Access::Sync);
    let waiting = job("/two", Access::Read);
    {
        let queue = Queue::open(&path).await?;
        queue.enqueue(&waiting).await?;
        queue.start(&running).await?;
    }

    let queue = Queue::open(&path).await?;

    assert_eq!(
        queue.jobs().await?,
        [
            Job {
                key: waiting,
                state: JobState::Pending
            },
            Job {
                key: running.clone(),
                state: JobState::Interrupted
            },
        ]
    );
    queue.start(&running).await?;
    assert_eq!(queue.jobs().await?[1].state, JobState::Running);
    Ok(())
}

#[test(tokio::test)]
async fn each_failure_waits_twice_as_long_up_to_a_minute_and_a_bit() -> Res {
    let queue = Queue::in_memory().await?;
    let key = job("/one", Access::Sync);

    let mut waits = Vec::new();
    for n in 0..8 {
        let now = at(1_000 + n);
        let failure = queue.fail(&key, &format!("error {n}"), now).await?;
        waits.push(failure.retry_at.duration_since(now)?.as_secs());
        assert_eq!(failure.failures, u32::try_from(n)? + 1);
    }

    assert_eq!(waits, [2, 4, 8, 16, 32, 64, 64, 64]);
    assert_eq!(
        queue.failure(&key).await?.map(|f| f.error),
        Some("error 7".to_string())
    );
    Ok(())
}

#[test(tokio::test)]
async fn failures_survive_a_restart_until_the_job_works() -> Res {
    let dir = tempfile::tempdir()?;
    let path = queue_path(&dir);
    let key = job("/one", Access::Sync);
    let other = job("/two", Access::Sync);
    {
        let queue = Queue::open(&path).await?;
        queue.fail(&key, "offline", at(100)).await?;
        queue.fail(&key, "offline", at(200)).await?;
        queue.fail(&other, "denied", at(100)).await?;
    }

    let queue = Queue::open(&path).await?;
    assert_eq!(
        queue.failure(&key).await?,
        Some(Failure {
            failures: 2,
            error: "offline".to_string(),
            retry_at: at(204),
        })
    );

    queue.clear_failure(&key).await?;
    assert_eq!(queue.failure(&key).await?, None);
    assert_eq!(queue.failure(&other).await?.map(|f| f.failures), Some(1));
    Ok(())
}

#[test(tokio::test)]
async fn a_kept_revision_survives_a_restart_until_released() -> Res {
    let dir = tempfile::tempdir()?;
    let path = queue_path(&dir);
    let folder = copy("bucket", "/one");
    let first_publication = KeptRevision {
        hash: "beef".to_string(),
        base: None,
        reason: KeptReason::WorkflowBlocked,
    };
    {
        let queue = Queue::open(&path).await?;
        queue.keep(&folder, first_publication.clone()).await?;
    }

    let queue = Queue::open(&path).await?;
    assert_eq!(queue.kept(&folder).await?, Some(first_publication));

    queue.release(&folder).await?;
    assert_eq!(queue.kept(&folder).await?, None);
    Ok(())
}

/// One per folder: the copy in another folder, or from another bucket, keeps
/// its own.
#[test(tokio::test)]
async fn a_folder_keeps_only_its_latest_kept_revision() -> Res {
    let queue = Queue::in_memory().await?;
    let folder = copy("bucket", "/one");
    let elsewhere = copy("bucket", "/two");
    let other_bucket = copy("other", "/one");
    queue.keep(&folder, kept("aaaa")).await?;
    queue.keep(&elsewhere, kept("bbbb")).await?;
    queue.keep(&other_bucket, kept("cccc")).await?;

    queue.keep(&folder, kept("dddd")).await?;

    assert_eq!(queue.kept(&folder).await?, Some(kept("dddd")));
    assert_eq!(queue.kept(&elsewhere).await?, Some(kept("bbbb")));
    assert_eq!(queue.kept(&other_bucket).await?, Some(kept("cccc")));
    Ok(())
}

#[test(tokio::test)]
async fn a_queue_from_a_newer_build_is_refused_and_left_as_it_is() -> Res {
    let dir = tempfile::tempdir()?;
    let path = queue_path(&dir);
    {
        let queue = Queue::open(&path).await?;
        queue.keep(&copy("bucket", "/one"), kept("aaaa")).await?;
    }
    rusqlite::Connection::open(&path)?.pragma_update(None, "user_version", 2)?;

    let opened = Queue::open(&path).await;

    assert!(matches!(opened, Err(QueueError::Version(2))), "{opened:?}");
    let db = rusqlite::Connection::open(&path)?;
    let hashes: i64 = db.query_row("SELECT count(*) FROM kept", [], |row| row.get(0))?;
    assert_eq!(hashes, 1);
    Ok(())
}

#[test(tokio::test)]
async fn a_damaged_queue_is_refused_and_left_as_it_is() -> Res {
    let dir = tempfile::tempdir()?;
    let path = queue_path(&dir);
    std::fs::create_dir_all(path.parent().expect("a parent"))?;
    let junk = b"not a database, but somebody's only copy of it".repeat(100);
    std::fs::write(&path, &junk)?;

    let opened = Queue::open(&path).await;

    assert!(matches!(opened, Err(QueueError::Sqlite(_))), "{opened:?}");
    assert_eq!(std::fs::read(&path)?, junk);
    Ok(())
}

#[test(tokio::test)]
async fn a_job_this_build_cant_read_is_skipped_and_the_rest_listed() -> Res {
    let dir = tempfile::tempdir()?;
    let path = queue_path(&dir);
    let readable = job("/two", Access::Read);
    let queue = Queue::open(&path).await?;
    queue.enqueue(&job("/one", Access::Write)).await?;
    queue.enqueue(&readable).await?;
    queue.enqueue(&job("/three", Access::Sync)).await?;
    let db = rusqlite::Connection::open(&path)?;
    db.execute(
        "UPDATE job SET access = 'publish' WHERE access = 'write'",
        [],
    )?;
    db.execute("UPDATE job SET access = x'00' WHERE access = 'sync'", [])?;

    let listed = queue.jobs().await;

    let keys: Option<Vec<JobKey>> = listed
        .ok()
        .map(|jobs| jobs.into_iter().map(|j| j.key).collect());
    assert_eq!(keys, Some(vec![readable]));
    Ok(())
}

/// Another process holds the write lock while it creates the file's tables.
#[test(tokio::test)]
async fn a_new_queue_opened_by_two_processes_at_once_opens_in_both() -> Res {
    let dir = tempfile::tempdir()?;
    let path = queue_path(&dir);
    std::fs::create_dir_all(path.parent().expect("a parent"))?;
    let other = rusqlite::Connection::open(&path)?;
    other.execute_batch("BEGIN IMMEDIATE")?;

    let opening = tokio::spawn({
        let path = path.clone();
        async move { Queue::open(&path).await }
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    other.execute_batch(SCHEMA)?;
    other.execute_batch(&format!("PRAGMA user_version = {SCHEMA_VERSION}; COMMIT"))?;

    let opened = opening.await?;
    assert!(opened.is_ok(), "{opened:?}");
    Ok(())
}

#[test(tokio::test)]
async fn a_failure_counted_while_another_process_writes_waits_its_turn() -> Res {
    let dir = tempfile::tempdir()?;
    let path = queue_path(&dir);
    let queue = std::sync::Arc::new(Queue::open(&path).await?);
    let key = job("/one", Access::Sync);
    let other = rusqlite::Connection::open(&path)?;
    other.execute_batch("BEGIN IMMEDIATE")?;

    let failing = tokio::spawn({
        let queue = std::sync::Arc::clone(&queue);
        let key = key.clone();
        async move { queue.fail(&key, "offline", at(100)).await }
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    other.execute_batch("COMMIT")?;

    let failed = failing.await?;
    assert_eq!(failed.ok().map(|f| f.failures), Some(1));
    Ok(())
}
