use super::*;

use test_log::test;

use crate::dataspace::lock::LockEntry;
use crate::dataspace::reset_to_latest::reset_to_latest;
use crate::dataspace::store::tests::publish_local;
use crate::dataspace::sync::EntrySync;
use crate::dataspace::sync::tests::FIRST;
use crate::dataspace::sync::tests::LATEST;
use crate::dataspace::sync::tests::SECOND;
use crate::dataspace::sync::tests::World;
use crate::dataspace::sync::tests::key;
use crate::dataspace::sync::tests::out_key;
use crate::dataspace::sync::tests::package;
use crate::dataspace::sync::tests::read_text;
use crate::dataspace::sync::tests::writer;
use crate::flow::PackageStore;
use quilt_uri::Namespace;

type Res = Result<(), Box<dyn std::error::Error>>;

const TAGS: &str = "s3://b/.quilt/named_packages/team/raw";

impl World {
    /// Removes `revisions` of `package`, as `quilt remove-revisions
    /// --dataspace` does.
    async fn remove_as(
        &self,
        package: &PackageKey,
        revisions: &[&str],
    ) -> Result<Removed, RemoveError> {
        let revisions: Vec<String> = revisions.iter().map(ToString::to_string).collect();
        remove_revisions(
            &self.storage,
            &self.remote,
            Some(&self.domain()),
            &self.dataspace(),
            &writer(),
            package,
            &revisions,
        )
        .await
    }

    /// Tags `hash` with the timestamp `name`, as each push does.
    async fn tag(&self, name: &str, hash: &str) -> Res {
        self.remote
            .put_object(
                None,
                &S3Uri::try_from(format!("{TAGS}/{name}").as_str())?,
                hash.as_bytes().to_vec(),
            )
            .await?;
        Ok(())
    }

    async fn holds(&self, uri: &str) -> Result<bool, Box<dyn std::error::Error>> {
        Ok(self.remote.current(uri).await?.is_some())
    }
}

/// A dataspace filled at [`FIRST`], with [`SECOND`] published after it and
/// `latest`; each has a timestamp tag.
async fn two_revisions() -> Result<World, Box<dyn std::error::Error>> {
    let world = World::filled(&[("a.txt", b"v1")]).await?;
    world.tag("1000", FIRST).await?;
    world.publish(SECOND, &[("a.txt", b"v2")]).await?;
    world.tag("2000", SECOND).await?;
    Ok(world)
}

/// Removing the revision `latest` names takes its tags, moves `latest` to
/// the newest revision left, and leaves the manifest and the data.
#[test(tokio::test)]
async fn removing_latest_moves_it_to_the_newest_left_and_keeps_the_data() -> Res {
    let world = two_revisions().await?;
    let object = "s3://b/team/raw/a.txt?versionId=7632";

    let removed = world.remove_as(&key(), &["2222"]).await?;

    assert_eq!(removed.revisions, [SECOND]);
    assert_eq!(removed.latest, LatestAfter::Moved(FIRST.to_string()));
    assert_eq!(removed.locked, Vec::<String>::new());
    assert_eq!(world.latest().await?, FIRST);
    assert!(!world.holds(&format!("{TAGS}/2000")).await?);
    assert!(world.holds(&format!("{TAGS}/1000")).await?);
    assert!(
        world
            .holds(&format!("s3://b/.quilt/packages/{SECOND}"))
            .await?
    );
    assert!(world.holds(object).await?, "the data stays");
    let index = world.domain().registry("b")?;
    assert_eq!(
        index
            .read_latest(&world.storage, &package())
            .await?
            .as_deref(),
        Some(FIRST)
    );
    Ok(())
}

/// The lock's base is no more protected than in quilt3: it is removed, and
/// the lock left naming it, which the result says.
#[test(tokio::test)]
async fn removing_the_locks_base_leaves_the_lock_and_says_so() -> Res {
    let world = two_revisions().await?;

    let removed = world.remove_as(&key(), &[FIRST]).await?;

    assert_eq!(removed.latest, LatestAfter::Kept);
    assert_eq!(removed.locked, [FIRST]);
    assert_eq!(world.locked().await?, Some(LockEntry::at(FIRST)));
    assert!(!world.holds(&format!("{TAGS}/1000")).await?);
    assert_eq!(world.latest().await?, SECOND);
    // The bucket still holds the manifest, so the next tick reads `latest`.
    let read = world.tick().await?;
    assert!(
        matches!(read, EntrySync::Ran { read: Some(_), .. }),
        "{read:?}"
    );
    assert_eq!(world.base().await?.as_deref(), Some(SECOND));
    Ok(())
}

#[test(tokio::test)]
async fn removing_every_revision_removes_latest() -> Res {
    let world = two_revisions().await?;

    let removed = world.remove_as(&key(), &[FIRST, SECOND, FIRST]).await?;

    assert_eq!(removed.revisions, [FIRST, SECOND]);
    assert_eq!(removed.latest, LatestAfter::Removed);
    assert!(!world.holds(LATEST).await?);
    assert!(!world.holds(&format!("{TAGS}/1000")).await?);
    Ok(())
}

#[test(tokio::test)]
async fn a_revision_the_registry_lacks_or_a_shared_prefix_is_refused() -> Res {
    let world = two_revisions().await?;
    world.tag("3000", &format!("22a{}", "2".repeat(61))).await?;

    for (wanted, expected) in [
        ("3333", "not found"),
        ("22", "ambiguous"),
        ("", "not found"),
    ] {
        let refused = world.remove_as(&key(), &[wanted]).await;
        let matched = match &refused {
            Err(RemoveError::NotFound(_)) => "not found",
            Err(RemoveError::Ambiguous(_)) => "ambiguous",
            _ => "other",
        };
        assert_eq!(matched, expected, "{wanted:?}: {refused:?}");
    }
    let refused = world.remove_as(&key(), &[]).await;
    assert!(
        matches!(refused, Err(RemoveError::NoneNamed)),
        "{refused:?}"
    );
    assert_eq!(world.latest().await?, SECOND, "nothing was removed");
    Ok(())
}

/// In the local registry the revision's manifests go and its objects stay;
/// a lock based at it then finds it missing, and a reset is the way on.
#[test(tokio::test)]
async fn a_local_registry_revision_is_removed_and_the_lock_finds_it_missing() -> Res {
    let world = World::new("sync = [\"quilt+local://lab-mac#package=me/out\"]")?;
    let local = world.local_named("lab-mac").await?;
    let namespace = Namespace::try_from("me/out")?;
    let first = publish_local(&world.storage, &local, "me/out", &[("o.txt", b"o1")]).await?;
    let second = publish_local(&world.storage, &local, "me/out", &[("o.txt", b"o2")]).await?;
    world.sync().await?;
    let objects: Vec<_> = crate::flow::list_files(&world.storage, &local.objects_dir())
        .await?
        .into_iter()
        .map(|(name, _, _)| name)
        .collect();

    let removed = world.remove_as(&out_key("lab-mac"), &[&second]).await?;

    assert_eq!(removed.latest, LatestAfter::Moved(first.clone()));
    assert_eq!(removed.locked, std::slice::from_ref(&second));
    assert_eq!(
        local.read_latest(&world.storage, &namespace).await?,
        Some(first.clone())
    );
    assert!(!world.storage.exists(local.revision(&second)).await);
    assert!(
        !world
            .storage
            .exists(local.installed_manifest(&namespace, &second))
            .await
    );
    for object in &objects {
        assert!(world.storage.exists(local.objects_dir().join(object)).await);
    }

    let missing = world.tick().await?;
    assert!(matches!(missing, EntrySync::Failed(_)), "{missing:?}");
    let reset = reset_to_latest(
        &world.storage,
        &world.remote,
        &world.domain(),
        &world.dataspace(),
        &writer(),
        &out_key("lab-mac"),
    )
    .await?;
    assert_eq!(reset.revision, first);
    let folder = world.local_folder().await?;
    assert_eq!(read_text(&folder, "o.txt").as_deref(), Some("o1"));
    Ok(())
}
