//! A pull and a download of the same package, started together, as
//! `QuiltSync`'s autopull tick and a user's download start them. Each holds
//! the package's lock from its first read to its write, so they run one after
//! the other: the finished package tracks what was downloaded at the newer
//! revision, and no file nobody touched reads Modified.

use super::older_revision_tests::Rev1Installed;
use super::*;

use test_log::test;

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
