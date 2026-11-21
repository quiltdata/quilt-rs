use super::*;

use std::path::PathBuf;
use std::time::SystemTime;

use test_log::test;

use crate::dataspace::contract::Access;
use crate::dataspace::contract::Registry;
use crate::io::storage::LocalStorage;
use crate::queue::JobKey;
use crate::queue::KeptReason;
use crate::queue::KeptRevision;
use crate::state_store::FolderKey;

type Res = Result<(), Box<dyn std::error::Error>>;

/// `me/analysis` from the local registry under `name`, in `folder`.
fn local(name: &str, folder: &str) -> FolderKey {
    FolderKey {
        registry: Some(Registry::Local(name.to_string())),
        package: ("me", "analysis").into(),
        folder: PathBuf::from(folder),
    }
}

fn job(name: &str, folder: &str) -> JobKey {
    JobKey {
        folder: local(name, folder),
        access: Access::Sync,
    }
}

fn kept(hash: &str) -> KeptRevision {
    KeptRevision {
        hash: hash.to_string(),
        base: None,
        reason: KeptReason::LatestMoved,
    }
}

/// A rename moves the queue's jobs, failures and kept revisions from the
/// old name to the new one, and the old name still names the registry; a
/// bucket's are left as they are.
#[test(tokio::test)]
async fn a_rename_moves_the_queue_and_keeps_the_old_name_as_an_alias() -> Res {
    let dir = tempfile::tempdir()?;
    let storage = LocalStorage::new();
    let domain = DomainRoot::new(dir.path());
    domain.local().rename(&storage, "old-mac").await?;
    let queue = Queue::in_memory().await?;
    let waiting = job("old-mac", "/one");
    queue.enqueue(&waiting).await?;
    queue
        .fail(&waiting, "offline", SystemTime::UNIX_EPOCH)
        .await?;
    queue.keep(&local("old-mac", "/one"), kept("aaaa")).await?;
    let bucket = JobKey {
        folder: FolderKey {
            registry: Some(Registry::S3("b".to_string())),
            ..local("x", "/two")
        },
        access: Access::Sync,
    };
    queue.enqueue(&bucket).await?;

    let renamed = rename(&storage, &domain, &queue, "new-mac").await?;

    assert_eq!(renamed.names.name, "new-mac");
    assert!(renamed.names.answers_to("old-mac"));
    assert_eq!(domain.local().names(&storage).await?, renamed.names);
    assert_eq!(renamed.moved, 3);
    let jobs: Vec<JobKey> = queue.jobs().await?.into_iter().map(|job| job.key).collect();
    assert_eq!(jobs, [job("new-mac", "/one"), bucket]);
    let moved = job("new-mac", "/one");
    assert_eq!(
        queue.failure(&moved).await?.map(|failure| failure.error),
        Some("offline".to_string())
    );
    assert_eq!(queue.failure(&waiting).await?, None);
    assert_eq!(
        queue.kept(&local("new-mac", "/one")).await?,
        Some(kept("aaaa"))
    );
    assert_eq!(queue.kept(&local("old-mac", "/one")).await?, None);
    Ok(())
}

/// Work queued under an old name after the rename, as by a worker that read
/// the name before it, is moved by renaming again to the same name; where
/// the new name already holds a row for the same folder, it wins.
#[test(tokio::test)]
async fn renaming_again_moves_what_an_alias_still_holds() -> Res {
    let dir = tempfile::tempdir()?;
    let storage = LocalStorage::new();
    let domain = DomainRoot::new(dir.path());
    domain.local().rename(&storage, "old-mac").await?;
    let queue = Queue::in_memory().await?;
    rename(&storage, &domain, &queue, "new-mac").await?;
    queue.enqueue(&job("old-mac", "/late")).await?;
    queue.keep(&local("old-mac", "/one"), kept("old")).await?;
    queue.keep(&local("new-mac", "/one"), kept("new")).await?;

    let renamed = rename(&storage, &domain, &queue, "new-mac").await?;

    assert_eq!(renamed.moved, 1);
    let jobs: Vec<JobKey> = queue.jobs().await?.into_iter().map(|job| job.key).collect();
    assert_eq!(jobs, [job("new-mac", "/late")]);
    assert_eq!(
        queue.kept(&local("new-mac", "/one")).await?,
        Some(kept("new"))
    );
    assert_eq!(queue.kept(&local("old-mac", "/one")).await?, None);
    Ok(())
}

/// A name a local registry may not keep changes nothing.
#[test(tokio::test)]
async fn a_name_the_registry_cannot_keep_is_refused() -> Res {
    let dir = tempfile::tempdir()?;
    let storage = LocalStorage::new();
    let domain = DomainRoot::new(dir.path());
    domain.local().rename(&storage, "old-mac").await?;
    let queue = Queue::in_memory().await?;
    queue.enqueue(&job("old-mac", "/one")).await?;

    let refused = rename(&storage, &domain, &queue, "my mac").await;

    assert!(matches!(refused, Err(RenameError::Name(_))), "{refused:?}");
    assert_eq!(domain.local().names(&storage).await?.name, "old-mac");
    assert_eq!(queue.jobs().await?[0].key, job("old-mac", "/one"));
    Ok(())
}
