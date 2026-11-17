//! The move of a domain's lineage from `data.json` to refs, run in a child.

use std::collections::BTreeMap;
use std::collections::BTreeSet;

use test_log::test;

use super::*;
use crate::io::storage::Storage;
use crate::lineage::refs::RefStore;
use crate::lineage::refs::migrate;
use crate::lineage::refs::state;

/// What [`super::Operation::MigrateUp`] runs on `world`'s domain. It records
/// the packages it wrote in `moved/<pid>.json` under the world's root.
pub(super) async fn up(world: &World) -> Res {
    let migrated = migrate::up(&LocalStorage::new(), &paths(world)).await?;
    let written: Vec<String> = migrated.written.iter().map(ToString::to_string).collect();
    let records = world.root.join("moved");
    std::fs::create_dir_all(&records)?;
    std::fs::write(
        records.join(format!("{}.json", std::process::id())),
        serde_json::to_vec(&written)?,
    )?;
    Ok(())
}

fn paths(world: &World) -> DomainPaths {
    DomainPaths::new(world.root.join("domain"))
}

/// A domain on `data.json` holding [`bystander`] and [`target`], both
/// created locally.
async fn legacy(dir: &TempDir) -> Res<World> {
    let world = World::new(dir, LineageStore::DataJson).await?;
    let source = world.root.join("target-source");
    std::fs::create_dir_all(&source)?;
    std::fs::write(source.join("data.csv"), b"a,b")?;
    world
        .domain()
        .create_package(target(), Some(source), None, UserMeta::Clear)
        .await?;
    Ok(world)
}

/// The bytes of each ref, by namespace, after checking every ref reads.
async fn refs(world: &World) -> Res<BTreeMap<Namespace, Vec<u8>>> {
    let paths = paths(world);
    let store = RefStore::new(paths.clone());
    let storage = LocalStorage::new();
    let mut refs = BTreeMap::new();
    for namespace in store.listed(&storage).await? {
        store.read(&storage, &namespace).await?;
        refs.insert(
            namespace.clone(),
            std::fs::read(paths.package_ref(&namespace))?,
        );
    }
    Ok(refs)
}

/// The packages each run of [`up`] recorded writing.
fn moved(world: &World) -> Res<Vec<BTreeSet<String>>> {
    let mut runs = Vec::new();
    for entry in std::fs::read_dir(world.root.join("moved"))? {
        runs.push(serde_json::from_slice(&std::fs::read(entry?.path())?)?);
    }
    Ok(runs)
}

/// An older build installs between a move stopped before the stub and its
/// resume: on a fresh domain it finds no `data.json`, sets the home as the
/// CLI does, and installs. Returns the installed revision's hash and the
/// `data.json` the install left.
async fn install_by_an_older_build(world: &World) -> Res<(String, Vec<u8>)> {
    world
        .domain_on(LineageStore::DataJson)
        .set_home(world.root.join("home"))
        .await?;
    let revision = world.publish(&[("a.txt", b"one")]).await?;
    Operation::LegacyInstall.run(world, &revision.hash).await?;
    Ok((revision.hash, std::fs::read(paths(world).lineage())?))
}

/// The install is in the refs, and `data.json.pre-refs` is the `data.json`
/// the resumed move found, not the copy the stopped one froze.
async fn assert_installed(world: &World, (hash, moved): (String, Vec<u8>)) -> Res {
    let paths = paths(world);
    assert_eq!(std::fs::read(paths.lineage_pre_refs())?, moved);
    let store = RefStore::new(paths.clone());
    let storage = LocalStorage::new();
    let lineage = store.read(&storage, &target()).await?;
    assert_eq!(lineage.remote_uri.map(|remote| remote.hash), Some(hash));
    assert_eq!(
        store.home(&storage).await?.as_ref(),
        &world.root.join("home")
    );
    assert_eq!(
        state::detect(&storage, &paths).await?,
        state::DomainState::Refs
    );
    Ok(())
}

/// [`killed::an_install_by_an_older_build_after_a_kill_before_the_stub_is_moved`]
/// from the state the kill leaves, built directly.
#[test(tokio::test)]
async fn an_install_by_an_older_build_after_a_move_stopped_before_the_stub_is_moved() -> Res {
    let dir = TempDir::new()?;
    let world = World::empty(&dir, LineageStore::DataJson);
    let paths = paths(&world);
    std::fs::create_dir_all(paths.dot_quilt_dir())?;
    std::fs::write(
        paths.lineage_migrating(),
        serde_json::to_vec(&crate::lineage::DomainLineage::default())?,
    )?;
    let installed = install_by_an_older_build(&world).await?;

    up(&world).await?;

    assert_installed(&world, installed).await
}

/// Two processes wait on the lock together: one moves every package, and
/// the other, reading the domain once it has the lock, moves none.
#[test(tokio::test)]
async fn two_processes_started_together_migrate_once() -> Res {
    let dir = TempDir::new()?;
    let world = legacy(&dir).await?;
    let held = LocalStorage::new()
        .lock_exclusive(paths(&world).migrate_lock())
        .await?;
    let mut movers = [
        spawn(&world, Operation::MigrateUp, "")?,
        spawn(&world, Operation::MigrateUp, "")?,
    ];
    std::thread::sleep(Duration::from_millis(500));
    for mover in &mut movers {
        assert!(mover.try_wait()?.is_none(), "a mover did not wait");
    }
    drop(held);
    let deadline = Instant::now() + LIMIT;
    for mover in &mut movers {
        let status = loop {
            if let Some(status) = mover.try_wait()? {
                break status;
            }
            assert!(Instant::now() < deadline, "a mover never ended");
            std::thread::sleep(Duration::from_millis(10));
        };
        assert!(status.success(), "a mover failed: {status}");
    }

    let mut runs = moved(&world)?;
    runs.sort();
    let both: BTreeSet<String> = [bystander(), target()]
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(runs, [BTreeSet::new(), both]);
    assert_eq!(refs(&world).await?.len(), 2);
    Ok(())
}

/// Validation item 5's kills. Each has a twin in `lineage::refs::migrate`
/// that builds the state the kill leaves directly, for the OSes with no kill.
#[cfg(unix)]
mod killed {
    use test_log::test;

    use super::*;

    fn copy(world: &World, dir: &TempDir) -> Res<World> {
        copy_dir(&world.root, dir.path())?;
        Ok(World::empty(dir, LineageStore::DataJson))
    }

    fn copy_dir(from: &Path, to: &Path) -> Res {
        std::fs::create_dir_all(to)?;
        for entry in std::fs::read_dir(from)? {
            let entry = entry?;
            let target = to.join(entry.file_name());
            if entry.file_type()?.is_dir() {
                copy_dir(&entry.path(), &target)?;
            } else {
                std::fs::copy(entry.path(), target)?;
            }
        }
        Ok(())
    }

    /// Checks the domain is done moving: refs as `expected`, the stub in
    /// place of `data.json`, and no working copy left.
    async fn assert_moved(world: &World, expected: &BTreeMap<Namespace, Vec<u8>>) -> Res {
        let paths = paths(world);
        assert_eq!(&refs(world).await?, expected);
        assert_eq!(
            state::detect(&LocalStorage::new(), &paths).await?,
            state::DomainState::Refs
        );
        assert!(state::is_stub(&std::fs::read(paths.lineage())?));
        assert!(!paths.lineage_migrating().exists());
        Ok(())
    }

    /// A move killed at each step, on a domain with packages and on a fresh
    /// one, resumes to the refs and the `data.json.pre-refs` an uninterrupted
    /// move leaves, and no kill leaves a ref that does not read.
    #[test(tokio::test)]
    async fn a_move_killed_at_each_step_resumes_to_the_same_refs() -> Res {
        let with_packages = TempDir::new()?;
        let fresh = TempDir::new()?;
        for (start, has_packages) in [
            (legacy(&with_packages).await?, true),
            (World::empty(&fresh, LineageStore::DataJson), false),
        ] {
            let whole_dir = TempDir::new()?;
            let whole = copy(&start, &whole_dir)?;
            up(&whole).await?;
            let expected = refs(&whole).await?;
            let pre_refs = std::fs::read(paths(&whole).lineage_pre_refs())?;
            assert_eq!(expected.is_empty(), !has_packages);
            for step in [
                "after-freeze",
                "after-stub",
                "after-config",
                "after-ref",
                "before-done",
                "after-done",
            ] {
                let dir = TempDir::new()?;
                let world = copy(&start, &dir)?;
                let name = format!("migrate.up.{step}");
                let killed = killed_at(&world, Operation::MigrateUp, "", &name)?;
                // A domain with no package writes no ref.
                assert_eq!(killed, has_packages || step != "after-ref", "{name}");
                refs(&world).await?;

                up(&world).await?;

                assert_moved(&world, &expected).await?;
                assert_eq!(
                    std::fs::read(paths(&world).lineage_pre_refs())?,
                    pre_refs,
                    "{name}"
                );
            }
        }
        Ok(())
    }

    /// A ref a killed move wrote is kept as it is, not written again.
    #[test(tokio::test)]
    async fn a_ref_written_by_a_killed_move_is_not_rewritten() -> Res {
        let dir = TempDir::new()?;
        let world = legacy(&dir).await?;
        assert!(killed_at(
            &world,
            Operation::MigrateUp,
            "",
            "migrate.up.after-ref"
        )?);
        let paths = paths(&world);
        let [namespace]: [Namespace; 1] = refs(&world)
            .await?
            .into_keys()
            .collect::<Vec<_>>()
            .try_into()
            .expect("one ref");
        let mut marked: serde_json::Value =
            serde_json::from_slice(&std::fs::read(paths.package_ref(&namespace))?)?;
        marked["marker"] = serde_json::json!("written before the kill");
        let marked = serde_json::to_vec_pretty(&marked)?;
        std::fs::write(paths.package_ref(&namespace), &marked)?;

        up(&world).await?;

        assert_eq!(std::fs::read(paths.package_ref(&namespace))?, marked);
        assert_eq!(refs(&world).await?.len(), 2);
        Ok(())
    }

    #[test(tokio::test)]
    async fn an_install_by_an_older_build_after_a_kill_before_the_stub_is_moved() -> Res {
        let dir = TempDir::new()?;
        let world = World::empty(&dir, LineageStore::DataJson);
        assert!(killed_at(
            &world,
            Operation::MigrateUp,
            "",
            "migrate.up.after-freeze"
        )?);
        let installed = install_by_an_older_build(&world).await?;

        up(&world).await?;

        assert_installed(&world, installed).await
    }

    /// After a move and a rollback, a second move killed once it froze its
    /// own copy resumes from that copy, keeps the first move's byte for
    /// byte, and removes its own once done.
    #[test(tokio::test)]
    async fn a_second_move_killed_resumes_from_its_own_copy() -> Res {
        let dir = TempDir::new()?;
        let world = legacy(&dir).await?;
        let paths = paths(&world);
        let mut second: serde_json::Value =
            serde_json::from_slice(&std::fs::read(paths.lineage())?)?;
        second["packages"]
            .as_object_mut()
            .expect("packages")
            .remove(&target().to_string());
        let second = serde_json::to_vec(&second)?;
        up(&world).await?;
        let first = std::fs::read(paths.lineage_pre_refs())?;
        let bystander_ref = std::fs::read(paths.package_ref(&bystander()))?;
        // Rolled back as the down-migration leaves a domain, after which an
        // older build uninstalled the target.
        std::fs::rename(paths.refs_dir(), paths.refs_archive("refs.rolled-back-x"))?;
        std::fs::remove_file(paths.domain_config())?;
        std::fs::write(paths.lineage(), &second)?;

        assert!(killed_at(
            &world,
            Operation::MigrateUp,
            "",
            "migrate.up.after-config"
        )?);
        assert_eq!(std::fs::read(paths.lineage_migrating())?, second);
        up(&world).await?;

        assert_moved(&world, &BTreeMap::from([(bystander(), bystander_ref)])).await?;
        assert_eq!(std::fs::read(paths.lineage_pre_refs())?, first);
        Ok(())
    }
}
