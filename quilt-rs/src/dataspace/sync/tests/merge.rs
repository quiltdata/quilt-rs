//! The tick's merge of local commits onto a `latest` that moved past their
//! base.

use super::*;

use test_log::test;

use crate::dataspace::conflict::ConflictMap;
use crate::dataspace::conflict::Other;
use crate::dataspace::conflict::Row;
use crate::paths::file_url;

type Error = Box<dyn std::error::Error>;

/// A dataspace filled at [`FIRST`], `a.txt` and `b.txt` at `v1`, whose edit
/// of `a.txt` to `mine` was committed and locked but never pushed. The lock
/// entry.
async fn committed() -> Result<(World, LockEntry), Error> {
    let world = World::filled(&[("a.txt", b"v1"), ("b.txt", b"v1")]).await?;
    world.write("a.txt", "mine")?;
    let stopped = stopping_at("push.object", 0, world.tick()).await?;
    assert!(matches!(stopped, EntrySync::Failed(_)), "{stopped:?}");
    let locked = world.locked().await?.ok_or("no lock entry")?;
    assert_eq!(locked.commits.len(), 1, "{locked:?}");
    Ok((world, locked))
}

/// The row `conflict.json` holds for `name` of the package `key`.
async fn row(world: &World, key: &PackageKey, name: &str) -> Result<Row, Error> {
    let map = ConflictMap::read(&world.storage, &world.dataspace()).await?;
    Ok(map
        .packages
        .get(key)
        .and_then(|rows| rows.get(&LogicalKey::new(name).ok()?))
        .ok_or(format!("no row for {name}"))?
        .clone())
}

/// Decides the row of `name` of the package `key` as `decide` says.
async fn decide(
    world: &World,
    key: &PackageKey,
    name: &str,
    decide: impl FnOnce(&Row) -> Option<String>,
) -> Res {
    let mut map = ConflictMap::read(&world.storage, &world.dataspace()).await?;
    let row = map
        .packages
        .get_mut(key)
        .and_then(|rows| rows.get_mut(&LogicalKey::new(name).ok()?))
        .ok_or(format!("no row for {name}"))?;
    row.resolved = Some(decide(row).ok_or("nothing to decide")?);
    map.write(&world.storage, &world.dataspace()).await?;
    Ok(())
}

fn read_of(outcome: &EntrySync) -> &ReadReport {
    match outcome {
        EntrySync::Ran {
            read: Some(read), ..
        } => read,
        other => panic!("expected a read, got {other:?}"),
    }
}

fn blocked_at(outcome: &EntrySync, revision: &str, keys: &[&str]) -> bool {
    let keys: Vec<PathBuf> = keys.iter().map(PathBuf::from).collect();
    matches!(outcome, EntrySync::Blocked { revision: at, conflicts }
        if at == revision && *conflicts == keys)
}

/// `latest`'s change and the local commits' touch other keys, so the tick
/// takes both, the folder's change not yet committed too, and publishes them
/// as one revision on `latest`.
#[test(tokio::test)]
async fn a_moved_latest_is_merged_with_local_commits_apart_from_it_and_published() -> Res {
    let (world, _) = committed().await?;
    world
        .publish(
            SECOND,
            &[("a.txt", b"v1"), ("b.txt", b"v2"), ("c.txt", b"new")],
        )
        .await?;
    world.write("d.txt", "not committed")?;

    let outcome = world.tick().await?;

    assert_eq!(read_of(&outcome).revision, SECOND);
    let revision = advanced(&outcome);
    assert_eq!(world.latest().await?, revision);
    assert_eq!(
        world.revision(&revision).await?,
        files(&[
            ("a.txt", "mine"),
            ("b.txt", "v2"),
            ("c.txt", "new"),
            ("d.txt", "not committed"),
        ])
    );
    assert_eq!(world.text("b.txt").as_deref(), Some("v2"));
    assert_eq!(world.text("c.txt").as_deref(), Some("new"));
    assert_eq!(world.locked().await?, Some(LockEntry::at(revision)));
    // The folder holds what was published, metadata file included.
    let again = world.tick().await?;
    assert!(
        matches!(
            again,
            EntrySync::Ran {
                read: None,
                published: None
            }
        ),
        "{again:?}"
    );
    Ok(())
}

/// The merge's push is a push like any: one that `latest` moved past again
/// is stored, and the lock holds the merge as its one commit on the `latest`
/// it was merged onto, which the next tick merges onto the newer one.
#[test(tokio::test)]
async fn a_merge_that_latest_moved_past_again_is_merged_again_by_the_next_tick() -> Res {
    let (world, _) = committed().await?;
    world
        .publish(SECOND, &[("a.txt", b"v1"), ("b.txt", b"v2")])
        .await?;
    let gate = world.remote.park(LATEST);
    let rival = async {
        // The tick's own read of `latest`, then the push's.
        gate.arrived().await;
        let push = world.remote.park(LATEST);
        gate.release();
        push.arrived().await;
        world
            .publish(
                THIRD,
                &[("a.txt", b"v1"), ("b.txt", b"v2"), ("c.txt", b"new")],
            )
            .await?;
        push.release();
        Res::Ok(())
    };
    let (outcome, rivalled) = tokio::join!(world.tick(), rival);
    rivalled?;
    let outcome = outcome?;

    let EntrySync::Ran {
        published: Some(Published::Stored { revision, latest }),
        ..
    } = &outcome
    else {
        panic!("expected the merge stored, got {outcome:?}");
    };
    assert_eq!(latest.as_deref(), Some(THIRD));
    assert_eq!(
        world.locked().await?,
        Some(LockEntry {
            base: Some(SECOND.to_string()),
            commits: vec![revision.clone()],
        })
    );

    let outcome = world.tick().await?;

    let merged = advanced(&outcome);
    assert_eq!(
        world.revision(&merged).await?,
        files(&[("a.txt", "mine"), ("b.txt", "v2"), ("c.txt", "new")])
    );
    assert_eq!(world.text("c.txt").as_deref(), Some("new"));
    assert_eq!(world.locked().await?, Some(LockEntry::at(merged)));
    Ok(())
}

/// A key the local commits and `latest` both changed blocks the merge before
/// anything is applied, committed or published: the folder and the lock stay
/// as they were, and `conflict.json` holds the publish's row.
#[test(tokio::test)]
async fn a_moved_latest_that_collides_with_local_commits_blocks_with_a_publish_row() -> Res {
    let (world, locked) = committed().await?;
    world
        .publish(SECOND, &[("a.txt", b"theirs"), ("b.txt", b"v2")])
        .await?;
    world.write("e.txt", "not committed")?;

    let outcome = world.tick().await?;

    assert!(blocked_at(&outcome, SECOND, &["a.txt"]), "{outcome:?}");
    assert_eq!(world.text("a.txt").as_deref(), Some("mine"));
    assert_eq!(world.text("b.txt").as_deref(), Some("v1"));
    assert_eq!(world.latest().await?, SECOND);
    assert_eq!(world.locked().await?, Some(locked));
    let row = row(&world, &key(), "a.txt").await?;
    let Other::Destination(Some(destination)) = &row.other else {
        panic!("expected a publish's row, got {row:?}");
    };
    assert_eq!(
        destination.physical_key.as_deref(),
        Some(format!("s3://b/team/raw/a.txt?versionId={}", hex::encode("theirs")).as_str())
    );
    let current = row.current.as_ref().ok_or("no current side")?;
    // The contract's spelling of the folder, which on Windows is not
    // `folder()`'s.
    let contract = Contract::read(&world.storage, &world.dataspace(), None).await?;
    let folder = &contract.entries().first().ok_or("no entry")?.folder;
    assert_eq!(
        current.physical_key.as_deref(),
        Some(file_url(&folder.join("a.txt"))?.as_str())
    );
    assert_eq!(current.size, Some(4));
    assert_eq!(row.resolved, None);
    let text = std::fs::read_to_string(world.dataspace().join("conflict.json"))?;
    assert!(
        text.contains("\"destination\"") && !text.contains("\"incoming\""),
        "{text}"
    );
    Ok(())
}

/// The revision a tick that merged leaves `latest` at: the merge it
/// published, or `latest` itself where that held the merge already.
fn merged(outcome: &EntrySync, latest: &str) -> String {
    match outcome {
        EntrySync::Ran {
            read: Some(_),
            published: None,
        } => latest.to_string(),
        outcome => advanced(outcome),
    }
}

/// A later tick applies the decision recorded on a publish's row: `latest`'s
/// object, which leaves nothing to publish but `latest` itself, or the
/// folder's own file, or `""`, either published as the merge.
#[test(tokio::test)]
async fn a_decided_collision_is_applied_by_the_next_tick_and_published() -> Res {
    type Choice = fn(&Row) -> Option<String>;
    let cases: [(&str, Choice, Option<&str>); 3] = [
        (
            "latest's",
            |row| row.others_key().map(str::to_string),
            Some("theirs"),
        ),
        (
            "the folder's",
            |row| row.current.as_ref()?.physical_key.clone(),
            Some("mine"),
        ),
        ("none", |_| Some(String::new()), None),
    ];
    for (taking, choice, a) in cases {
        let (world, _) = committed().await?;
        world
            .publish(SECOND, &[("a.txt", b"theirs"), ("b.txt", b"v2")])
            .await?;
        assert!(blocked_at(&world.tick().await?, SECOND, &["a.txt"]));
        decide(&world, &key(), "a.txt", choice).await?;

        let outcome = world.tick().await?;

        let revision = merged(&outcome, SECOND);
        assert_eq!(revision == SECOND, taking == "latest's", "{outcome:?}");
        assert_eq!(world.latest().await?, revision, "taking {taking}");
        let mut expected = vec![("b.txt", "v2")];
        expected.extend(a.map(|a| ("a.txt", a)));
        expected.sort_unstable();
        assert_eq!(
            world.revision(&revision).await?,
            files(&expected),
            "taking {taking}"
        );
        assert_eq!(world.text("a.txt").as_deref(), a, "taking {taking}");
        assert_eq!(
            world.text("b.txt").as_deref(),
            Some("v2"),
            "taking {taking}"
        );
        assert!(
            !world.dataspace().join("conflict.json").exists(),
            "taking {taking}"
        );
        assert_eq!(world.locked().await?, Some(LockEntry::at(revision)));
    }
    Ok(())
}

/// `metadata.toml` is compared like any file: the package's metadata both
/// sides changed is one collision row, and `""` takes `latest`'s.
#[test(tokio::test)]
async fn metadata_both_sides_changed_collides_as_metadata_toml_and_is_decided() -> Res {
    let world = World::filled(&[("a.txt", b"v1")]).await?;
    world.write(
        METADATA_FILE,
        "version = \"v0\"\n\n[metadata.package]\nk = \"mine\"\n",
    )?;
    world.write("b.txt", "mine")?;
    stopping_at("push.object", 0, world.tick()).await?;
    let header = ManifestHeader {
        user_meta: Some(serde_json::json!({"k": "theirs"})),
        ..ManifestHeader::default()
    };
    world
        .publish_with("team/raw", SECOND, &[("a.txt", b"v2")], header, Vec::new())
        .await?;

    let outcome = world.tick().await?;

    assert!(
        blocked_at(&outcome, SECOND, &[METADATA_FILE]),
        "{outcome:?}"
    );
    let row = row(&world, &key(), METADATA_FILE).await?;
    assert!(
        matches!(&row.other, Other::Destination(Some(side)) if side.physical_key.is_none()),
        "{row:?}"
    );
    decide(&world, &key(), METADATA_FILE, |_| Some(String::new())).await?;

    let outcome = world.tick().await?;

    let revision = advanced(&outcome);
    assert_eq!(
        world.revision(&revision).await?,
        files(&[("a.txt", "v2"), ("b.txt", "mine")])
    );
    let uri = ManifestUri {
        origin: None,
        bucket: "b".to_string(),
        namespace: package(),
        hash: revision,
    };
    let published = fetch_jsonl(&world.remote, &uri).await?;
    assert_eq!(
        published.header.user_meta,
        Some(serde_json::json!({"k": "theirs"}))
    );
    assert!(
        world
            .text(METADATA_FILE)
            .is_some_and(|text| text.contains("theirs")),
        "{:?}",
        world.text(METADATA_FILE)
    );
    Ok(())
}

/// A package never published whose commit is locked with no base, and whose
/// bucket gained a `latest` it did not make, is merged onto it with the empty
/// revision as base: `latest`'s files are its change, and the local commit
/// loses nothing.
#[test(tokio::test)]
async fn a_package_never_published_whose_bucket_gained_a_latest_is_merged_onto_it() -> Res {
    let world = unpublished().await?;
    stopping_at("push.object", 0, world.tick()).await?;
    assert!(
        world
            .locked()
            .await?
            .is_some_and(|locked| locked.base.is_none())
    );
    world.publish(SECOND, &[("b.txt", b"theirs")]).await?;

    let outcome = world.tick().await?;

    let revision = advanced(&outcome);
    assert_eq!(
        world.revision(&revision).await?,
        files(&[("a.txt", "first"), ("b.txt", "theirs")])
    );
    assert_eq!(world.text("b.txt").as_deref(), Some("theirs"));
    assert_eq!(world.locked().await?, Some(LockEntry::at(revision)));
    Ok(())
}

const LOCAL_OUT: &str = "sync = [\"quilt+local://lab-mac#package=me/out\"]";

/// Records `files` in the local registry as a commit of `me/out` on `base`,
/// and puts it in the lock after the commits it names, as an import, or a
/// publish that was lost, leaves it.
async fn commit_out(
    world: &World,
    base: Option<&str>,
    files: &[(&'static str, &'static [u8])],
) -> Result<LockEntry, Error> {
    let mut rows = Vec::new();
    let mut content = BTreeMap::new();
    for (key, body) in files {
        let key = LogicalKey::new(*key)?;
        rows.push(changed(key.clone(), None));
        content.insert(key, RowContent::Bytes(bytes::Bytes::from_static(body)));
    }
    let lineage = PackageLineage {
        base_hash: base.unwrap_or_default().to_string(),
        ..PackageLineage::default()
    };
    let manifest = Manifest {
        rows,
        ..Manifest::default()
    };
    let (_, recorded) = crate::flow::record(
        &world.domain().local(),
        &world.storage,
        &world.remote,
        WorkflowCheck::Warn { host: None },
        &Namespace::try_from("me/out")?,
        lineage,
        manifest,
        &content,
    )
    .await?;
    let locked = LockEntry {
        base: base.map(str::to_string),
        commits: vec![recorded.hash().ok_or("nothing recorded")?.to_string()],
    };
    let here = Writer {
        local_registry: "lab-mac".to_string(),
        ..writer()
    };
    let contract = Contract::read(&world.storage, &world.dataspace(), None).await?;
    let mut lock = world.lock().await?;
    lock.packages.insert(out_key("lab-mac"), locked.clone());
    lock.write(&world.storage, &contract, &here).await?;
    Ok(locked)
}

/// A dataspace syncing `me/out` of the local registry `lab-mac`, filled at a
/// revision holding `o.txt` and `p.txt` at `1`, whose edit of `o.txt` to
/// `mine` is a local commit the lock names. The registry, the working folder
/// and the lock entry.
async fn local_committed() -> Result<(World, LocalRegistry, PathBuf, LockEntry), Error> {
    let world = World::new(LOCAL_OUT)?;
    let local = world.local_named("lab-mac").await?;
    let files: &[(&str, &[u8])] = &[("o.txt", b"o1"), ("p.txt", b"p1")];
    let first = publish_local(&world.storage, &local, "me/out", files).await?;
    assert!(world.sync().await?.read_only);
    let folder = world.local_folder().await?;
    std::fs::write(folder.join("o.txt"), "mine")?;
    let locked = commit_out(
        &world,
        Some(&first),
        &[("o.txt", b"mine"), ("p.txt", b"p1")],
    )
    .await?;
    Ok((world, local, folder, locked))
}

async fn out_locked(world: &World) -> Result<Option<LockEntry>, Error> {
    Ok(world
        .lock()
        .await?
        .packages
        .get(&out_key("lab-mac"))
        .cloned())
}

/// [`a_moved_latest_is_merged_with_local_commits_apart_from_it_and_published`]
/// for a package of the local registry.
#[test(tokio::test)]
async fn a_local_entrys_commits_are_merged_onto_a_moved_latest_and_published() -> Res {
    let (world, local, folder, _) = local_committed().await?;
    publish_local(
        &world.storage,
        &local,
        "me/out",
        &[("o.txt", b"o1"), ("p.txt", b"p2")],
    )
    .await?;

    let outcome = world.tick().await?;

    let revision = advanced(&outcome);
    let namespace = Namespace::try_from("me/out")?;
    assert_eq!(
        local.read_latest(&world.storage, &namespace).await?,
        Some(revision.clone())
    );
    assert_eq!(
        local_bytes(&world.storage, &local, &revision, "o.txt").await?,
        b"mine"
    );
    assert_eq!(
        local_bytes(&world.storage, &local, &revision, "p.txt").await?,
        b"p2"
    );
    assert_eq!(read_text(&folder, "p.txt").as_deref(), Some("p2"));
    assert_eq!(out_locked(&world).await?, Some(LockEntry::at(revision)));
    Ok(())
}

/// A local package's collision is blocked with its row under the registry's
/// key, and the decision keeping the folder's file is applied by the next
/// tick and published there.
#[test(tokio::test)]
async fn a_local_entrys_collision_is_blocked_then_its_decision_applied() -> Res {
    let (world, local, folder, locked) = local_committed().await?;
    let second = publish_local(
        &world.storage,
        &local,
        "me/out",
        &[("o.txt", b"o3"), ("p.txt", b"p2")],
    )
    .await?;

    let outcome = world.tick().await?;

    assert!(blocked_at(&outcome, &second, &["o.txt"]), "{outcome:?}");
    assert_eq!(read_text(&folder, "p.txt").as_deref(), Some("p1"));
    assert_eq!(out_locked(&world).await?, Some(locked));
    let row = row(&world, &out_key("lab-mac"), "o.txt").await?;
    assert!(matches!(&row.other, Other::Destination(Some(_))), "{row:?}");
    decide(&world, &out_key("lab-mac"), "o.txt", |row| {
        row.current.as_ref()?.physical_key.clone()
    })
    .await?;

    let outcome = world.tick().await?;

    let revision = advanced(&outcome);
    assert_eq!(
        local_bytes(&world.storage, &local, &revision, "o.txt").await?,
        b"mine"
    );
    assert_eq!(
        local_bytes(&world.storage, &local, &revision, "p.txt").await?,
        b"p2"
    );
    assert_eq!(read_text(&folder, "o.txt").as_deref(), Some("mine"));
    assert_eq!(read_text(&folder, "p.txt").as_deref(), Some("p2"));
    assert_eq!(out_locked(&world).await?, Some(LockEntry::at(revision)));
    Ok(())
}

/// [`a_package_never_published_whose_bucket_gained_a_latest_is_merged_onto_it`]
/// for a package of the local registry.
#[test(tokio::test)]
async fn a_local_package_never_published_whose_registry_gained_a_latest_is_merged() -> Res {
    let world = World::new(LOCAL_OUT)?;
    let local = world.local_named("lab-mac").await?;
    assert!(world.sync().await?.read_only);
    let folder = world.local_folder().await?;
    std::fs::create_dir_all(&folder)?;
    std::fs::write(folder.join("o.txt"), "first")?;
    commit_out(&world, None, &[("o.txt", b"first")]).await?;
    publish_local(&world.storage, &local, "me/out", &[("q.txt", b"theirs")]).await?;

    let outcome = world.tick().await?;

    let revision = advanced(&outcome);
    assert_eq!(
        local_bytes(&world.storage, &local, &revision, "o.txt").await?,
        b"first"
    );
    assert_eq!(
        local_bytes(&world.storage, &local, &revision, "q.txt").await?,
        b"theirs"
    );
    assert_eq!(read_text(&folder, "q.txt").as_deref(), Some("theirs"));
    assert_eq!(out_locked(&world).await?, Some(LockEntry::at(revision)));
    Ok(())
}

/// [`committed`], with `data.toml` edited since, so the next sync only
/// fills.
async fn committed_then_edited() -> Result<(World, LockEntry), Error> {
    let (world, locked) = committed().await?;
    world.contract("sync = [\"team/raw\"]\n# edited")?;
    Ok((world, locked))
}

fn filled_entry(synced: &DataspaceSync) -> &EntrySync {
    assert!(synced.read_only, "{synced:?}");
    assert_eq!(synced.entries.len(), 1, "{synced:?}");
    &synced.entries[0].1
}

/// A fill merges the lock's commits alone onto a moved `latest`, and reads
/// the merge whole, the folder's edits not yet committed kept as local work:
/// the dataspace is filled, and the next tick publishes the merge with them.
#[test(tokio::test)]
async fn a_fill_merges_commits_onto_a_moved_latest_and_reads_the_merge_whole() -> Res {
    let (world, _) = committed_then_edited().await?;
    world
        .publish(
            SECOND,
            &[("a.txt", b"v1"), ("b.txt", b"v2"), ("c.txt", b"new")],
        )
        .await?;
    world.write("a.txt", "mine, then more")?;
    world.write("d.txt", "not committed")?;

    let synced = world.sync().await?;

    let outcome = filled_entry(&synced);
    assert!(
        matches!(
            outcome,
            EntrySync::Ran {
                read: Some(_),
                published: None
            }
        ),
        "{outcome:?}"
    );
    assert_eq!(world.latest().await?, SECOND);
    assert_eq!(world.text("a.txt").as_deref(), Some("mine, then more"));
    assert_eq!(world.text("b.txt").as_deref(), Some("v2"));
    assert_eq!(world.text("c.txt").as_deref(), Some("new"));
    let locked = world.locked().await?.ok_or("no lock entry")?;
    assert_eq!(locked.base.as_deref(), Some(SECOND));
    assert_eq!(locked.commits.len(), 1, "{locked:?}");
    let contract = Contract::read(&world.storage, &world.dataspace(), None).await?;
    assert_eq!(
        world.lock().await?.header.data_toml_hash,
        contract.data_toml_hash()
    );

    let outcome = world.tick().await?;

    let revision = advanced(&outcome);
    assert_eq!(
        world.revision(&revision).await?,
        files(&[
            ("a.txt", "mine, then more"),
            ("b.txt", "v2"),
            ("c.txt", "new"),
            ("d.txt", "not committed"),
        ])
    );
    assert_eq!(world.locked().await?, Some(LockEntry::at(revision)));
    Ok(())
}

/// A fill whose commits collide with a moved `latest` blocks with the
/// publish's rows, applying nothing, and the dataspace stays unfilled.
#[test(tokio::test)]
async fn a_fill_whose_commits_collide_with_a_moved_latest_blocks_with_a_publish_row() -> Res {
    let (world, locked) = committed_then_edited().await?;
    world
        .publish(SECOND, &[("a.txt", b"theirs"), ("b.txt", b"v2")])
        .await?;

    let synced = world.sync().await?;

    let outcome = filled_entry(&synced);
    assert!(blocked_at(outcome, SECOND, &["a.txt"]), "{outcome:?}");
    assert_eq!(world.text("a.txt").as_deref(), Some("mine"));
    assert_eq!(world.text("b.txt").as_deref(), Some("v1"));
    assert_eq!(world.locked().await?, Some(locked));
    let row = row(&world, &key(), "a.txt").await?;
    assert!(matches!(&row.other, Other::Destination(Some(_))), "{row:?}");
    assert!(world.sync().await?.read_only, "still unfilled");
    Ok(())
}

/// A fill applies the decision on a publish's row to the merge: here the
/// folder's own file, which the merge takes and the next tick publishes.
#[test(tokio::test)]
async fn a_fill_applies_a_decision_on_a_publish_row_to_the_merge() -> Res {
    let (world, _) = committed_then_edited().await?;
    world
        .publish(SECOND, &[("a.txt", b"theirs"), ("b.txt", b"v2")])
        .await?;
    assert!(blocked_at(
        filled_entry(&world.sync().await?),
        SECOND,
        &["a.txt"]
    ));
    decide(&world, &key(), "a.txt", |row| {
        row.current.as_ref()?.physical_key.clone()
    })
    .await?;

    let synced = world.sync().await?;

    let outcome = filled_entry(&synced);
    assert!(
        matches!(outcome, EntrySync::Ran { read: Some(_), .. }),
        "{outcome:?}"
    );
    assert_eq!(world.text("a.txt").as_deref(), Some("mine"));
    assert_eq!(world.text("b.txt").as_deref(), Some("v2"));
    assert!(!world.dataspace().join("conflict.json").exists());
    let revision = advanced(&world.tick().await?);
    assert_eq!(
        world.revision(&revision).await?,
        files(&[("a.txt", "mine"), ("b.txt", "v2")])
    );
    Ok(())
}
