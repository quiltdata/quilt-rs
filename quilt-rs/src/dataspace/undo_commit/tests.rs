use super::*;

use test_log::test;

use crate::dataspace::reattach::unattached;
use crate::dataspace::store::tests::publish_local;
use crate::dataspace::sync::tests::FIRST;
use crate::dataspace::sync::tests::World;
use crate::dataspace::sync::tests::advanced;
use crate::dataspace::sync::tests::gate;
use crate::dataspace::sync::tests::key;
use crate::dataspace::sync::tests::local_key;
use crate::dataspace::sync::tests::moving;
use crate::dataspace::sync::tests::out_key;
use crate::dataspace::sync::tests::read_text;
use crate::dataspace::sync::tests::writer;
use crate::failpoint::stopping_at;

type Res = Result<(), Box<dyn std::error::Error>>;

impl World {
    /// Undoes `package`'s last commit, as `quilt undo-commit --dataspace` does.
    async fn undo_as(&self, package: &PackageKey) -> Result<Undone, UndoError> {
        Box::pin(undo_commit(
            &self.storage,
            &self.remote,
            &self.domain(),
            &self.dataspace(),
            &writer(),
            package,
        ))
        .await
    }

    /// The local commits of `team/raw` that the lock does not name.
    async fn unattached(&self) -> Result<Vec<String>, Box<dyn std::error::Error>> {
        let listed = unattached(
            &self.storage,
            &self.remote,
            &self.domain(),
            &self.dataspace(),
            &writer(),
            &key(),
        )
        .await?;
        Ok(listed.into_iter().map(|commit| commit.hash).collect())
    }
}

/// Undo drops the last commit and restores the folder to the commit before
/// it: a file it changed takes back its bytes, one it added goes with the
/// folder it leaves empty, and one it deleted comes back. The commit stays
/// in the store.
#[test(tokio::test)]
async fn undo_restores_the_revision_before_the_commit() -> Res {
    let world = World::filled(&[("a.txt", b"v1"), ("b.txt", b"v1")]).await?;
    world.write("a.txt", "v2")?;
    let first = world.commit("first").await?;
    world.write("a.txt", "v3")?;
    world.write("new/c.txt", "added")?;
    world.remove("b.txt")?;
    let second = world.commit("second").await?;
    // A file neither revision holds is not touched.
    world.write("mine.txt", "never committed")?;

    let undone = world.undo_as(&key()).await?;

    assert_eq!(undone.commit, second);
    let expected = LockEntry {
        base: Some(FIRST.to_string()),
        commits: vec![first],
    };
    assert_eq!(undone.locked.as_ref(), Some(&expected));
    assert_eq!(world.locked().await?, Some(expected));
    assert_eq!(world.text("a.txt").as_deref(), Some("v2"));
    assert_eq!(world.text("b.txt").as_deref(), Some("v1"));
    assert_eq!(world.text("new/c.txt"), None);
    assert!(!world.folder().join("new").exists(), "the emptied folder");
    assert_eq!(world.text("mine.txt").as_deref(), Some("never committed"));
    assert_eq!(world.unattached().await?, [second]);
    Ok(())
}

/// Undoing the only commit restores the base, fetched from the bucket.
#[test(tokio::test)]
async fn undoing_the_only_commit_restores_the_base() -> Res {
    let world = World::filled(&[("a.txt", b"v1"), ("b.txt", b"v1")]).await?;
    world.write("a.txt", "v2")?;
    world.remove("b.txt")?;
    world.commit("only").await?;

    let undone = world.undo_as(&key()).await?;

    assert_eq!(undone.locked, Some(LockEntry::at(FIRST)));
    assert_eq!(world.text("a.txt").as_deref(), Some("v1"));
    assert_eq!(world.text("b.txt").as_deref(), Some("v1"));
    let idle = world.tick().await?;
    assert!(
        matches!(
            idle,
            crate::dataspace::sync::EntrySync::Ran {
                read: None,
                published: None
            }
        ),
        "{idle:?}"
    );
    Ok(())
}

/// A path the undo would write over that holds work not committed refuses
/// it, and nothing changes.
#[test(tokio::test)]
async fn undo_refuses_when_the_folder_holds_changes_not_committed() -> Res {
    let world = World::filled(&[("a.txt", b"v1"), ("b.txt", b"v1")]).await?;
    world.write("a.txt", "v2")?;
    world.write("c.txt", "added")?;
    let commit = world.commit("edit").await?;

    for (name, change) in [("a.txt", Some("edited")), ("c.txt", None)] {
        let was = world.text(name);
        match change {
            Some(text) => world.write(name, text)?,
            None => world.remove(name)?,
        }

        let refused = world.undo_as(&key()).await;

        assert!(
            matches!(&refused, Err(UndoError::Uncommitted(paths)) if *paths == [PathBuf::from(name)]),
            "{name}: {refused:?}"
        );
        assert_eq!(
            world.locked().await?.map(|locked| locked.commits),
            Some(vec![commit.clone()])
        );
        assert_eq!(world.text(name).as_deref(), change);
        world.write(name, &was.ok_or("no file")?)?;
    }
    std::fs::write(world.folder().join(METADATA_FILE), "[metadata]\n")?;
    let refused = world.undo_as(&key()).await;
    assert!(
        matches!(&refused, Err(UndoError::Uncommitted(paths)) if *paths == [PathBuf::from(METADATA_FILE)]),
        "{refused:?}"
    );
    Ok(())
}

/// A push releases the commits it squashed, so undo cannot step past it.
#[test(tokio::test)]
async fn undo_does_not_step_back_past_a_push() -> Res {
    let world = World::filled(&[("a.txt", b"v1")]).await?;
    world.write("a.txt", "v2")?;
    let commit = world.commit("pushed").await?;
    assert_eq!(advanced(&world.tick().await?), commit);

    let refused = world.undo_as(&key()).await;

    assert!(
        matches!(refused, Err(UndoError::NothingToUndo(_))),
        "{refused:?}"
    );
    assert_eq!(world.locked().await?, Some(LockEntry::at(commit)));
    assert_eq!(world.text("a.txt").as_deref(), Some("v2"));
    Ok(())
}

/// Undoing the only commit of a package never published empties the folder
/// of what it committed, and takes the lock entry with it; the commit stays
/// in the store for reattach.
#[test(tokio::test)]
async fn undoing_a_first_commit_empties_the_folder_and_the_lock_entry() -> Res {
    let world = World::synced()?;
    world.write("a.txt", "first")?;
    let commit = world.commit("start").await?;
    assert!(world.folder().join(METADATA_FILE).exists());

    let undone = world.undo_as(&key()).await?;

    assert_eq!(undone.commit, commit);
    assert_eq!(undone.locked, None);
    assert_eq!(world.locked().await?, None);
    assert_eq!(world.text("a.txt"), None);
    assert!(!world.folder().join(METADATA_FILE).exists());
    assert_eq!(world.unattached().await?, [commit]);
    Ok(())
}

/// An undo stopped part-way puts every file back and leaves the lock.
#[test(tokio::test)]
async fn an_undo_stopped_part_way_puts_the_folder_back() -> Res {
    let world = World::filled(&[("a.txt", b"v1"), ("b.txt", b"v1")]).await?;
    world.write("a.txt", "v2")?;
    world.write("b.txt", "v2")?;
    world.commit("both").await?;
    let before = world.lock().await?;

    let stopped = stopping_at("replace.placed", 0, world.undo_as(&key())).await;

    assert!(stopped.is_err(), "{stopped:?}");
    assert_eq!(world.lock().await?, before);
    assert_eq!(world.text("a.txt").as_deref(), Some("v2"));
    assert_eq!(world.text("b.txt").as_deref(), Some("v2"));
    Ok(())
}

#[test(tokio::test)]
async fn a_package_with_no_commit_has_nothing_to_undo() -> Res {
    let world = World::filled(&[("a.txt", b"v1")]).await?;

    let refused = world.undo_as(&key()).await;

    assert!(
        matches!(refused, Err(UndoError::NothingToUndo(_))),
        "{refused:?}"
    );
    Ok(())
}

/// A package of the local registry is undone as a bucket's is, its base
/// read from the registry.
#[test(tokio::test)]
async fn a_local_registry_entry_is_restored_to_its_base() -> Res {
    let world = World::new("sync = [\"quilt+local://lab-mac#package=me/out\"]")?;
    let local = world.local_named("lab-mac").await?;
    let first = publish_local(&world.storage, &local, "me/out", &[("o.txt", b"o1")]).await?;
    world.sync().await?;
    let folder = world.local_folder().await?;
    std::fs::write(folder.join("o.txt"), "o2")?;
    std::fs::write(folder.join("p.txt"), "p")?;
    let commit = world
        .commit_as(&out_key("lab-mac"), "local edit")
        .await?
        .commit
        .ok_or("nothing committed")?;

    let undone = world.undo_as(&out_key("lab-mac")).await?;

    assert_eq!(undone.commit, commit);
    assert_eq!(undone.locked, Some(LockEntry::at(first)));
    assert_eq!(read_text(&folder, "o.txt").as_deref(), Some("o1"));
    assert_eq!(read_text(&folder, "p.txt"), None);
    Ok(())
}

/// Undo follows a package `data.toml` moved to its bucket, whose move has
/// not landed: it drops the last local commit in the local registry and
/// puts the folder back to the revision before it.
#[test(tokio::test)]
async fn undo_follows_a_package_moved_to_a_bucket_until_the_move_lands() -> Res {
    let (world, first, head) = moving().await?;
    world.local_named("lab-mac").await?;
    gate(&world, r#"{"type": "object"}"#).await?;
    world.tick().await?;
    world.write("a.txt", "v2")?;
    world.write("b.txt", "v1")?;

    let undone = world.undo_as(&key()).await;

    assert!(
        matches!(&undone, Ok(undone) if undone.commit == head),
        "{undone:?}"
    );
    let lock = world.lock().await?;
    assert_eq!(lock.packages.get(&key()), None, "{lock:?}");
    assert_eq!(lock.packages.get(&local_key()), Some(&LockEntry::at(first)));
    assert_eq!(world.text("a.txt").as_deref(), Some("v1"));
    Ok(())
}
