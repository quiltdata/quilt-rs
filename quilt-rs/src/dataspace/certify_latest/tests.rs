use super::*;

use test_log::test;

use crate::dataspace::hooks::HookPoint::PostWrite;
use crate::dataspace::hooks::tests::call;
use crate::dataspace::hooks::tests::calls;
use crate::dataspace::hooks::tests::table;
use crate::dataspace::store::tests::publish_local;
use crate::dataspace::sync::EntrySync;
use crate::dataspace::sync::tests::FIRST;
use crate::dataspace::sync::tests::SECOND;
use crate::dataspace::sync::tests::World;
use crate::dataspace::sync::tests::files;
use crate::dataspace::sync::tests::gate;
use crate::dataspace::sync::tests::hooked;
use crate::dataspace::sync::tests::key;
use crate::dataspace::sync::tests::local_key;
use crate::dataspace::sync::tests::moving;
use crate::dataspace::sync::tests::out_key;
use crate::dataspace::sync::tests::record;
use crate::dataspace::sync::tests::writer;
use quilt_uri::Namespace;

type Res = Result<(), Box<dyn std::error::Error>>;

impl World {
    /// Certifies `package`'s current revision, as `quilt certify --dataspace`
    /// does.
    async fn certify_as(&self, package: &PackageKey) -> Result<Certified, CertifyError> {
        certify_latest(
            &self.storage,
            &self.remote,
            &self.domain(),
            &self.queue().await.expect("the Domain's queue"),
            &self.dataspace(),
            &writer(),
            package,
        )
        .await
    }
}

/// A revision the bucket's workflow refused is stored and kept; certifying
/// forces `latest` to it, releases it, and bases the lock there.
#[test(tokio::test)]
async fn certify_releases_a_revision_the_workflow_blocked() -> Res {
    let world = World::filled(&[("a.txt", b"v1")]).await?;
    gate(&world, r#"{"type": "object", "required": ["owner"]}"#).await?;
    world.write("a.txt", "v2")?;
    let blocked = match world.tick().await? {
        EntrySync::Ran {
            published: Some(Published::WorkflowBlocked { revision, .. }),
            ..
        } => revision,
        other => panic!("expected a blocked revision, got {other:?}"),
    };
    assert_eq!(world.latest().await?, FIRST);

    let certified = world.certify_as(&key()).await?;

    assert_eq!(certified.revision, blocked);
    assert_eq!(certified.previous.as_deref(), Some(FIRST));
    assert_eq!(world.latest().await?, blocked);
    assert_eq!(world.locked().await?, Some(LockEntry::at(blocked.clone())));
    assert_eq!(world.kept().await?, None);
    assert_eq!(world.revision(&blocked).await?, files(&[("a.txt", "v2")]));
    let idle = world.tick().await?;
    assert!(
        matches!(
            idle,
            EntrySync::Ran {
                read: None,
                published: None
            }
        ),
        "{idle:?}"
    );
    Ok(())
}

/// A local commit on a base `latest` moved past is pushed, then made
/// `latest` over the other writer's revision: the last writer wins.
#[test(tokio::test)]
async fn certify_pushes_a_commit_and_forces_latest_past_another_writer() -> Res {
    let world = World::filled(&[("a.txt", b"v1")]).await?;
    world.write("a.txt", "mine")?;
    let commit = world.commit("mine").await?;
    world.publish(SECOND, &[("a.txt", b"theirs")]).await?;

    let certified = world.certify_as(&key()).await?;

    assert_eq!(certified.revision, commit);
    assert_eq!(certified.previous.as_deref(), Some(SECOND));
    assert!(
        matches!(certified.pushed, Some(Published::Stored { .. })),
        "{:?}",
        certified.pushed
    );
    assert_eq!(world.latest().await?, commit);
    assert_eq!(world.revision(&commit).await?, files(&[("a.txt", "mine")]));
    assert_eq!(world.locked().await?, Some(LockEntry::at(commit)));
    Ok(())
}

/// With no commit, the base is forced back over a `latest` that moved.
#[test(tokio::test)]
async fn certify_forces_latest_back_to_the_base() -> Res {
    let world = World::filled(&[("a.txt", b"v1")]).await?;
    world.publish(SECOND, &[("a.txt", b"theirs")]).await?;

    let certified = world.certify_as(&key()).await?;

    assert_eq!(certified.revision, FIRST);
    assert_eq!(certified.previous.as_deref(), Some(SECOND));
    assert!(certified.pushed.is_none());
    assert_eq!(world.latest().await?, FIRST);
    assert_eq!(world.locked().await?, Some(LockEntry::at(FIRST)));
    Ok(())
}

/// A move to a bucket its workflow refused is released by certifying it
/// (25k decision 1): the bucket's `latest` is the last local commit, and
/// the lock holds the package under the bucket's key.
#[test(tokio::test)]
async fn certify_releases_a_move_to_a_bucket_the_workflow_refused() -> Res {
    let (world, _, head) = moving().await?;
    gate(&world, r#"{"type": "object"}"#).await?;
    let refused = world.tick().await?;
    assert!(
        matches!(&refused, EntrySync::Ran {
            published: Some(Published::WorkflowBlocked { revision, .. }),
            ..
        } if *revision == head),
        "{refused:?}"
    );

    let certified = world.certify_as(&key()).await?;

    assert_eq!(certified.revision, head);
    assert_eq!(world.latest().await?, head);
    let lock = world.lock().await?;
    assert_eq!(lock.packages.get(&key()), Some(&LockEntry::at(head)));
    assert!(!lock.packages.contains_key(&local_key()), "{lock:?}");
    Ok(())
}

#[test(tokio::test)]
async fn certify_runs_post_write_once_latest_moved() -> Res {
    let world = hooked(|record| table(PostWrite, record, &[])).await?;
    world.publish(SECOND, &[("a.txt", b"theirs")]).await?;

    world.certify_as(&key()).await?;
    world.certify_as(&key()).await?;

    assert_eq!(calls(&record(&world)), [call(PostWrite, Some(FIRST))]);
    Ok(())
}

#[test(tokio::test)]
async fn a_local_registry_entry_is_certified_in_the_registry() -> Res {
    let world = World::new("sync = [\"quilt+local://lab-mac#package=me/out\"]")?;
    let local = world.local_named("lab-mac").await?;
    let namespace = Namespace::try_from("me/out")?;
    publish_local(&world.storage, &local, "me/out", &[("o.txt", b"o1")]).await?;
    world.sync().await?;
    std::fs::write(world.local_folder().await?.join("o.txt"), "o2")?;
    let commit = world
        .commit_as(&out_key("lab-mac"), "mine")
        .await?
        .commit
        .ok_or("nothing committed")?;
    let theirs = publish_local(&world.storage, &local, "me/out", &[("o.txt", b"o3")]).await?;

    let certified = world.certify_as(&out_key("lab-mac")).await?;

    assert_eq!(certified.previous, Some(theirs));
    assert_eq!(
        local.read_latest(&world.storage, &namespace).await?,
        Some(commit.clone())
    );
    assert!(world.storage.exists(local.revision(&commit)).await);
    let locked = world
        .lock()
        .await?
        .packages
        .get(&out_key("lab-mac"))
        .cloned();
    assert_eq!(locked, Some(LockEntry::at(commit)));
    Ok(())
}

#[test(tokio::test)]
async fn only_a_sync_entry_with_a_revision_is_certified() -> Res {
    let world = World::new("read = [\"team/raw\"]")?;
    world.publish(FIRST, &[("a.txt", b"v1")]).await?;
    world.read_all().await?;
    let refused = world.certify_as(&key()).await;
    assert!(
        matches!(refused, Err(CertifyError::Access(Access::Read))),
        "{refused:?}"
    );

    let world = World::synced()?;
    let refused = world.certify_as(&key()).await;
    assert!(
        matches!(refused, Err(CertifyError::NothingToCertify(_))),
        "{refused:?}"
    );
    Ok(())
}
