use super::*;

use test_log::test;

use crate::dataspace::hooks::HookPoint::PreWrite;
use crate::dataspace::hooks::tests::METADATA_VAR;
use crate::dataspace::hooks::tests::call;
use crate::dataspace::hooks::tests::calls;
use crate::dataspace::hooks::tests::table;
use crate::dataspace::reattach::unattached;
use crate::dataspace::store::tests::publish_local;
use crate::dataspace::sync::tests::FIRST;
use crate::dataspace::sync::tests::World;
use crate::dataspace::sync::tests::advanced;
use crate::dataspace::sync::tests::files;
use crate::dataspace::sync::tests::gate;
use crate::dataspace::sync::tests::hooked;
use crate::dataspace::sync::tests::key;
use crate::dataspace::sync::tests::local_key;
use crate::dataspace::sync::tests::moving;
use crate::dataspace::sync::tests::out_key;
use crate::dataspace::sync::tests::package;
use crate::dataspace::sync::tests::read_text;
use crate::dataspace::sync::tests::record;
use crate::dataspace::sync::tests::ungate;
use crate::dataspace::sync::tests::writer;
use quilt_uri::Namespace;

type Res = Result<(), Box<dyn std::error::Error>>;

impl World {
    /// Commits `package` with `message`, as `quilt commit --dataspace` does.
    pub(crate) async fn commit_as(
        &self,
        package: &PackageKey,
        message: &str,
    ) -> Result<Committed, CommitError> {
        commit(
            &self.storage,
            &self.remote,
            &self.domain(),
            &self.dataspace(),
            &writer(),
            package,
            message,
            &self.user_rules(),
        )
        .await
    }

    /// [`World::commit_as`] for `team/raw`, which must record a commit.
    pub(crate) async fn commit(&self, message: &str) -> Result<String, Box<dyn std::error::Error>> {
        let committed = self.commit_as(&key(), message).await?;
        Ok(committed.commit.ok_or("nothing committed")?)
    }

    /// The commit `hash` of `team/raw` in the bucket's store.
    pub(crate) async fn committed(
        &self,
        hash: &str,
    ) -> Result<Manifest, Box<dyn std::error::Error>> {
        let store = self.domain().registry("b")?;
        Ok(Manifest::from_path(&self.storage, &store.installed_manifest(&package(), hash)).await?)
    }
}

/// Two commits are appended to the lock, oldest first, each with its own
/// message, and nothing is pushed until the tick pushes the last.
#[test(tokio::test)]
async fn a_commit_is_appended_to_the_lock_and_pushed_by_the_next_tick() -> Res {
    let world = World::filled(&[("a.txt", b"v1")]).await?;
    world.write("a.txt", "v2")?;

    let first = world.commit("first edit").await?;

    assert_eq!(
        world.locked().await?,
        Some(LockEntry {
            base: Some(FIRST.to_string()),
            commits: vec![first.clone()],
        })
    );
    assert_eq!(world.latest().await?, FIRST, "a commit pushes nothing");
    assert_eq!(
        world.committed(&first).await?.header.message(),
        Some("first edit")
    );
    world.write("b.txt", "new")?;

    let second = world.commit("second edit").await?;

    assert_eq!(
        world.locked().await?.map(|locked| locked.commits),
        Some(vec![first, second.clone()])
    );
    let pushed = advanced(&world.tick().await?);
    assert_eq!(pushed, second, "the push squashes onto the last commit");
    assert_eq!(world.latest().await?, second);
    assert_eq!(
        world.revision(&second).await?,
        files(&[("a.txt", "v2"), ("b.txt", "new")])
    );
    Ok(())
}

#[test(tokio::test)]
async fn a_folder_with_nothing_new_commits_nothing_and_leaves_the_lock() -> Res {
    let world = World::filled(&[("a.txt", b"v1")]).await?;
    let before = world.lock().await?;

    let committed = world.commit_as(&key(), "nothing").await?;

    assert_eq!(committed.commit, None);
    assert_eq!(world.lock().await?, before);
    Ok(())
}

/// The contract's `pre-write` hook runs before the commit, as in the tick,
/// and the commit takes what it wrote; `post-write` waits for the push.
#[test(tokio::test)]
async fn a_commit_runs_pre_write_first_and_takes_what_it_wrote() -> Res {
    let stamp = "[metadata.package]\nstamped = true\n";
    let world = hooked(|record| table(PreWrite, record, &[(METADATA_VAR, stamp)])).await?;
    world.write("a.txt", "v2")?;

    let commit = world.commit("stamped").await?;

    let header = world.committed(&commit).await?.header;
    assert_eq!(header.user_meta, Some(serde_json::json!({"stamped": true})));
    assert_eq!(header.message(), Some("stamped"));
    assert_eq!(calls(&record(&world)), [call(PreWrite, None)]);
    Ok(())
}

/// A package never published is committed with no base (25m), and its first
/// push bases the entry at the commit.
#[test(tokio::test)]
async fn a_package_never_published_is_committed_with_no_base() -> Res {
    let world = World::synced()?;
    world.write("a.txt", "first")?;

    let commit = world.commit("start").await?;

    assert_eq!(
        world.locked().await?,
        Some(LockEntry {
            base: None,
            commits: vec![commit.clone()],
        })
    );
    let synced = world.sync().await?;
    let [(_, pushed)] = synced.entries.as_slice() else {
        panic!("{synced:?}");
    };
    assert_eq!(advanced(pushed), commit);
    assert_eq!(world.locked().await?, Some(LockEntry::at(commit)));
    Ok(())
}

#[test(tokio::test)]
async fn a_package_with_no_revision_and_no_folder_is_refused() -> Res {
    let world = World::synced()?;

    let refused = world.commit_as(&key(), "start").await;

    assert!(
        matches!(refused, Err(CommitError::NoFolder(_))),
        "{refused:?}"
    );
    Ok(())
}

#[test(tokio::test)]
async fn a_folder_gone_while_the_lock_names_a_revision_is_refused() -> Res {
    let world = World::filled(&[("a.txt", b"v1")]).await?;
    world.remove("a.txt")?;

    let refused = world.commit_as(&key(), "deleted").await;

    assert!(
        matches!(refused, Err(CommitError::Entry(EntryError::FolderGone(_)))),
        "{refused:?}"
    );
    assert_eq!(world.locked().await?, Some(LockEntry::at(FIRST)));
    Ok(())
}

/// Where the tick would only read, absence cannot count as a deletion yet.
#[test(tokio::test)]
async fn a_commit_after_data_toml_is_edited_is_refused_until_a_sync_reads_it() -> Res {
    let world = World::filled(&[("a.txt", b"v1"), ("b.txt", b"v1")]).await?;
    world.contract("# edited\nsync = [\"team/raw\"]")?;
    world.remove("b.txt")?;

    let refused = world.commit_as(&key(), "edited").await;

    assert!(matches!(refused, Err(CommitError::Edited)), "{refused:?}");
    world.sync().await?;
    assert_eq!(world.text("b.txt").as_deref(), Some("v1"), "read whole");
    world.write("a.txt", "v2")?;
    assert!(world.commit_as(&key(), "edited").await?.commit.is_some());
    Ok(())
}

/// An install reads its folder whole before the lock takes the revision, so
/// a commit needs no fill of the whole dataspace after it.
#[test(tokio::test)]
async fn a_commit_after_an_install_needs_no_fill() -> Res {
    let world = World::new("")?;
    world
        .publish(FIRST, &[("a.txt", b"v1"), ("b.txt", b"v1")])
        .await?;
    let request = crate::dataspace::install::Install::new("quilt+s3://b#package=team/raw");
    crate::dataspace::install::install(
        &world.storage,
        &world.remote,
        None,
        &world.dataspace(),
        &writer(),
        &request,
    )
    .await?;
    assert_eq!(world.lock().await?.header.filled, None);
    world.remove("b.txt")?;

    let commit = world.commit("deleted b").await?;

    assert_eq!(world.committed(&commit).await?.rows.len(), 1);
    Ok(())
}

#[test(tokio::test)]
async fn only_a_sync_entry_takes_a_commit() -> Res {
    for (access, list) in [
        (Access::Write, "write"),
        (Access::Replace, "replace"),
        (Access::Read, "read"),
    ] {
        let world = World::new(&format!("{list} = [\"team/raw\"]"))?;
        world.write("a.txt", "v1")?;

        let refused = world.commit_as(&key(), "no").await;

        assert!(
            matches!(&refused, Err(CommitError::Access(refused)) if *refused == access),
            "{refused:?}"
        );
    }
    let world = World::synced()?;
    let other = "quilt+s3://b#package=team/other".parse()?;
    let refused = world.commit_as(&other, "no").await;
    assert!(
        matches!(refused, Err(CommitError::NotDeclared(_))),
        "{refused:?}"
    );
    Ok(())
}

/// A commit undone or lost from the lock stays in the store, where reattach
/// finds it.
#[test(tokio::test)]
async fn a_commit_is_kept_in_the_store_for_reattach() -> Res {
    let world = World::filled(&[("a.txt", b"v1")]).await?;
    world.write("a.txt", "v2")?;
    let commit = world.commit("kept").await?;
    std::fs::remove_file(world.dataspace().join(LOCK_FILE))?;

    let listed = unattached(
        &world.storage,
        &world.remote,
        &world.domain(),
        &world.dataspace(),
        &writer(),
        &key(),
    )
    .await?;

    assert_eq!(
        listed.iter().map(|commit| &commit.hash).collect::<Vec<_>>(),
        [&commit]
    );
    Ok(())
}

/// A package of the local registry is committed into it, under the name it
/// keeps, and the tick publishes the commit there.
#[test(tokio::test)]
async fn a_local_registry_entry_is_committed_into_the_registry() -> Res {
    let world = World::new("sync = [\"quilt+local://lab-mac#package=me/out\"]")?;
    let local = world.local_named("lab-mac").await?;
    let namespace = Namespace::try_from("me/out")?;
    let first = publish_local(&world.storage, &local, "me/out", &[("o.txt", b"o1")]).await?;
    world.sync().await?;
    let folder = world.local_folder().await?;
    std::fs::write(folder.join("o.txt"), "o2")?;

    let committed = world.commit_as(&out_key("lab-mac"), "local edit").await?;

    let commit = committed.commit.ok_or("nothing committed")?;
    assert_eq!(
        committed.locked,
        Some(LockEntry {
            base: Some(first.clone()),
            commits: vec![commit.clone()],
        })
    );
    assert_eq!(
        local.read_latest(&world.storage, &namespace).await?,
        Some(first)
    );
    let manifest = Manifest::from_path(
        &world.storage,
        &local.installed_manifest(&namespace, &commit),
    )
    .await?;
    assert_eq!(manifest.header.message(), Some("local edit"));
    assert_eq!(manifest.header.workflow, None);
    assert_eq!(
        committed.warnings,
        Vec::new(),
        "a local registry has no workflows"
    );
    let pushed = advanced(&world.tick().await?);
    assert_eq!(pushed, commit);
    assert_eq!(read_text(&folder, "o.txt").as_deref(), Some("o2"));
    Ok(())
}

#[test(tokio::test)]
async fn a_lock_written_elsewhere_with_files_here_is_refused() -> Res {
    let world = World::filled(&[("a.txt", b"v1")]).await?;
    world.write("a.txt", "v2")?;
    let elsewhere = Writer {
        host: "there".to_string(),
        ..writer()
    };

    let refused = commit(
        &world.storage,
        &world.remote,
        &world.domain(),
        &world.dataspace(),
        &elsewhere,
        &key(),
        "no",
        &world.user_rules(),
    )
    .await;

    assert!(
        matches!(&refused, Err(CommitError::Mismatch(found)) if found == &[Mismatch::Host]),
        "{refused:?}"
    );
    Ok(())
}

const CONFIG: &str = "s3://b/.quilt/workflows/config.yml";

/// The workflow id the commit `hash` of `team/raw` names, if any.
async fn workflow_of(
    world: &World,
    hash: &str,
) -> Result<Option<String>, Box<dyn std::error::Error>> {
    Ok(world
        .committed(hash)
        .await?
        .header
        .workflow
        .and_then(|workflow| workflow.id)
        .map(|id| id.id))
}

/// A commit the bucket's workflow refuses is recorded under its default
/// workflow anyway, in the lock and the store, with a warning that a push of
/// it will be refused (#297's warn rulings); nothing is pushed.
#[test(tokio::test)]
async fn a_commit_the_buckets_workflow_refuses_is_recorded_with_an_inconsistent_warning() -> Res {
    let world = World::filled(&[("a.txt", b"v1")]).await?;
    gate(&world, r#"{"type": "object", "required": ["owner"]}"#).await?;
    world.write("a.txt", "v2")?;

    let committed = world.commit_as(&key(), "refused edit").await?;

    let commit = committed.commit.clone().ok_or("nothing committed")?;
    let [WorkflowWarning::Inconsistent { bucket, reason }] = committed.warnings.as_slice() else {
        panic!("expected one inconsistent warning: {committed:?}");
    };
    assert_eq!(bucket, "b");
    assert!(reason.contains("owner"), "{reason}");
    assert_eq!(
        world.locked().await?.map(|locked| locked.commits),
        Some(vec![commit.clone()])
    );
    assert_eq!(workflow_of(&world, &commit).await?.as_deref(), Some("gate"));
    assert_eq!(world.latest().await?, FIRST, "a commit pushes nothing");
    Ok(())
}

/// A commit the bucket's workflow accepts carries it, and no warning.
#[test(tokio::test)]
async fn a_commit_the_buckets_workflow_accepts_has_no_warning() -> Res {
    let world = World::filled(&[("a.txt", b"v1")]).await?;
    gate(&world, r#"{"type": "object"}"#).await?;
    world.write("a.txt", "v2")?;

    let committed = world.commit_as(&key(), "accepted edit").await?;

    let commit = committed.commit.clone().ok_or("nothing committed")?;
    assert_eq!(committed.warnings, Vec::new(), "{committed:?}");
    assert_eq!(workflow_of(&world, &commit).await?.as_deref(), Some("gate"));
    Ok(())
}

/// A workflows config that cannot be read does not refuse the commit: it is
/// recorded with no workflow, and a warning that it went unchecked.
#[test(tokio::test)]
async fn a_commit_whose_workflows_config_cannot_be_read_is_recorded_unchecked() -> Res {
    let world = World::filled(&[("a.txt", b"v1")]).await?;
    gate(&world, r#"{"type": "object", "required": ["owner"]}"#).await?;
    world.remote.deny_reads(CONFIG);
    world.write("a.txt", "v2")?;

    let committed = world.commit_as(&key(), "unreadable").await?;

    let commit = committed.commit.clone().ok_or("nothing committed")?;
    let [WorkflowWarning::Unchecked { bucket, .. }] = committed.warnings.as_slice() else {
        panic!("expected one unchecked warning: {committed:?}");
    };
    assert_eq!(bucket, "b");
    assert_eq!(world.committed(&commit).await?.header.workflow, None);
    assert_eq!(
        world.locked().await?.map(|locked| locked.commits),
        Some(vec![commit])
    );
    Ok(())
}

/// A validator the schema misconfigures does not refuse the commit either:
/// it is recorded under the workflow, with a warning that it went unchecked.
#[test(tokio::test)]
async fn a_commit_a_misconfigured_validator_cannot_check_is_recorded_unchecked() -> Res {
    let world = World::filled(&[("a.txt", b"v1")]).await?;
    gate(&world, r##"{"$ref": "#/definitions/meta"}"##).await?;
    world.write("a.txt", "v2")?;

    let committed = world.commit_as(&key(), "misconfigured").await?;

    let commit = committed.commit.clone().ok_or("nothing committed")?;
    assert!(
        matches!(
            committed.warnings.as_slice(),
            [WorkflowWarning::Unchecked { bucket, .. }] if bucket == "b"
        ),
        "{committed:?}"
    );
    assert_eq!(workflow_of(&world, &commit).await?.as_deref(), Some("gate"));
    Ok(())
}

/// Nothing committed warns of nothing, whatever the workflows say.
#[test(tokio::test)]
async fn nothing_committed_has_no_warning() -> Res {
    let world = World::filled(&[("a.txt", b"v1")]).await?;
    gate(&world, r#"{"type": "object", "required": ["owner"]}"#).await?;

    let committed = world.commit_as(&key(), "nothing").await?;

    assert_eq!(committed.commit, None);
    assert_eq!(committed.warnings, Vec::new());
    Ok(())
}

/// A package `data.toml` moved to its bucket, whose move the bucket's
/// workflow refused, is committed in the local registry it still lives in,
/// after its last local commit, and the move then pushes that commit.
#[test(tokio::test)]
async fn a_commit_follows_a_package_moved_to_a_bucket_until_the_move_lands() -> Res {
    let (world, _, head) = moving().await?;
    world.local_named("lab-mac").await?;
    gate(&world, r#"{"type": "object"}"#).await?;
    world.tick().await?;
    world.write("a.txt", "v3")?;
    world.write("b.txt", "v1")?;

    let committed = world.commit_as(&key(), "after the move").await;

    let Ok(Committed {
        commit: Some(hash), ..
    }) = committed
    else {
        panic!("the moved package was not committed: {committed:?}");
    };
    let lock = world.lock().await?;
    assert_eq!(lock.packages.get(&key()), None, "{lock:?}");
    assert_eq!(
        lock.packages
            .get(&local_key())
            .map(|locked| locked.commits.clone()),
        Some(vec![head, hash.clone()])
    );
    ungate(&world).await?;
    world.sync().await?;
    assert_eq!(world.latest().await?, hash);
    Ok(())
}

/// The move a tick makes holds the bucket's package lock, so a commit that
/// follows the pending move takes that lock rather than the local one's.
#[test(tokio::test)]
async fn a_commit_on_a_moved_package_waits_for_the_bucket_lock() -> Res {
    let (world, _, _) = moving().await?;
    world.local_named("lab-mac").await?;
    gate(&world, r#"{"type": "object"}"#).await?;
    world.tick().await?;
    world.write("b.txt", "v1")?;
    let store = world.domain().registry("b")?;
    let _held = world
        .storage
        .lock_exclusive(store.package_lock(&package()))
        .await?;

    let committed = world.commit_as(&key(), "while the move runs").await;

    assert!(
        matches!(committed, Err(CommitError::Busy(_))),
        "{committed:?}"
    );
    Ok(())
}
