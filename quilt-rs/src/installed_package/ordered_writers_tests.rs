//! A pull and a download of the same package, run at once with nothing
//! ordering them, as `QuiltSync`'s autopull tick and a user's download run:
//! the finished package tracks what was downloaded at the newer revision, and
//! no file nobody touched reads Modified.
//!
//! Whichever writes second finds the entry moved and redoes its part: see
//! [`super::older_revision_tests`] for each crossing pinned in turn.

use super::older_revision_tests::Rev1Installed;
use super::*;

use test_log::test;

async fn assert_clean_at_rev2(t: &Rev1Installed) -> Res {
    assert_eq!(
        t.outcome().await?,
        (
            Some(t.rev2.clone()),
            Rev1Installed::all_paths(),
            b"two".to_vec()
        )
    );
    assert_eq!(t.changed().await?, Vec::<PathBuf>::new());
    Ok(())
}

#[test(tokio::test)]
async fn a_pull_and_a_download_run_at_once_and_end_clean() -> Res {
    let t = Rev1Installed::new().await?;
    let (pulled, downloaded) = tokio::join!(t.pull(), t.download());
    pulled?;
    downloaded?;
    assert_clean_at_rev2(&t).await
}

#[test(tokio::test)]
async fn a_download_and_a_pull_run_at_once_and_end_clean() -> Res {
    let t = Rev1Installed::new().await?;
    let (downloaded, pulled) = tokio::join!(t.download(), t.pull());
    downloaded?;
    pulled?;
    assert_clean_at_rev2(&t).await
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
    assert_clean_at_rev2(&t).await
}
