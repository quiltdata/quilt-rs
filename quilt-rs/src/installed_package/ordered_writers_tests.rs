//! A pull and a download of the same package, started together, as
//! `QuiltSync`'s autopull tick and a user's download start them. Each holds
//! the package's lock from its first read to its write, so they run one after
//! the other: the finished package tracks what was downloaded at the newer
//! revision, and no file nobody touched reads Modified.

use std::time::Duration;

use super::older_revision_tests::Rev1Installed;
use super::older_revision_tests::arrives;
use super::*;

use test_log::test;

use crate::io::storage::lock::tests::OtherProcess;

#[test(tokio::test)]
async fn a_pull_then_a_download_started_together_end_clean() -> Res {
    let t = Rev1Installed::new().await?;
    let (pulled, downloaded) = tokio::join!(t.pull(), t.download());
    pulled?;
    downloaded?;
    t.assert_clean_at_rev2().await
}

#[test(tokio::test)]
async fn a_download_then_a_pull_started_together_end_clean() -> Res {
    let t = Rev1Installed::new().await?;
    let (downloaded, pulled) = tokio::join!(t.download(), t.pull());
    downloaded?;
    pulled?;
    t.assert_clean_at_rev2().await
}

/// On separate tasks, so the two really run in parallel on the runtime's
/// threads rather than taking turns in one.
#[test(tokio::test(flavor = "multi_thread", worker_threads = 4))]
async fn a_pull_and_a_download_on_separate_tasks_end_clean() -> Res {
    let t = Arc::new(Rev1Installed::new().await?);
    let pull = tokio::spawn({
        let t = Arc::clone(&t);
        async move { t.pull().await }
    });
    let download = tokio::spawn({
        let t = Arc::clone(&t);
        async move { t.download().await }
    });
    pull.await.map_err(std::io::Error::other)??;
    download.await.map_err(std::io::Error::other)??;
    t.assert_clean_at_rev2().await
}

/// The tick's side: it only tries the lock. While a download holds it the
/// try says busy at once, so the tick skips the package with no wait, and
/// the next tick takes the lock and pulls cleanly.
#[test(tokio::test)]
async fn a_tick_that_skips_during_a_download_leaves_it_intact_and_pulls_next_time() -> Res {
    let t = Rev1Installed::new().await?;
    let fetching_changes = t.park_object("changes.txt", "c1");

    let (downloaded, tried) = tokio::join!(t.download(), async {
        arrives(&fetching_changes, "the download's fetch").await;
        let tried = tokio::time::timeout(Duration::from_secs(5), t.package.try_lock()).await;
        fetching_changes.release();
        tried
    });
    downloaded?;
    let tried = tried.expect("a try must not wait")?;
    assert!(tried.is_none(), "the tick must see the download");

    assert_eq!(
        t.outcome().await?,
        (
            Some(t.rev1.clone()),
            Rev1Installed::all_paths(),
            b"one".to_vec()
        )
    );
    assert_eq!(t.changed().await?, Vec::<PathBuf>::new());

    let locked = t
        .package
        .try_lock()
        .await?
        .expect("the next tick finds the package free");
    locked
        .pull(Some(HostConfig::default()), SyncScope::IndividualFiles)
        .await?;
    drop(locked);
    t.assert_clean_at_rev2().await
}

/// Another process, the CLI beside the app, holds the package's lock: a pull
/// here waits for it, and runs once the other process lets go.
#[test(tokio::test)]
async fn a_pull_waits_for_another_process_holding_the_package() -> Res {
    let t = Rev1Installed::new().await?;
    t.download().await?;
    let other = OtherProcess::holding(&t.package.paths.package_lock(&t.package.namespace)).await?;

    let mut pull = std::pin::pin!(t.pull());
    let early = tokio::time::timeout(Duration::from_millis(300), &mut pull).await;
    assert!(early.is_err(), "the pull must wait for the other process");

    other.release()?;
    tokio::time::timeout(Duration::from_secs(30), pull)
        .await
        .expect("the pull runs once the other process lets go")?;
    t.assert_clean_at_rev2().await
}
