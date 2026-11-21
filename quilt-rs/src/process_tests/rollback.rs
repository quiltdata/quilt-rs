//! The move of a domain's lineage from refs back to `data.json`, run in a
//! child.

use std::collections::BTreeMap;

use test_log::test;

use super::*;
use crate::Error;
use crate::error::LineageError;
use crate::lineage::PackageLineage;
use crate::lineage::PathState;
use crate::lineage::refs::state::STUB;

/// What [`super::Operation::MigrateDown`] runs on `world`'s domain.
pub(super) async fn down(world: &World) -> Res {
    rolled_back(world).await?;
    Ok(())
}

async fn rolled_back(world: &World) -> Res<crate::lineage::refs::rollback::RolledBack> {
    crate::lineage::refs::rollback::down(&LocalStorage::new(), &world.paths()).await
}

const PRE_REFS: &[u8] = br#"{"home": "/before", "packages": {}}"#;

impl World {
    fn paths(&self) -> DomainPaths {
        DomainPaths::new(self.root.join("domain"))
    }

    /// A domain on refs as the cut-over leaves it, holding [`target`] at a
    /// first revision with its files, unless `install` is false. Returns the
    /// revision.
    async fn migrated(dir: &TempDir, install: bool) -> Res<(Self, ManifestUri)> {
        let world = Self::new(dir, LineageStore::Refs).await?;
        std::fs::write(world.paths().lineage(), STUB)?;
        std::fs::write(world.paths().lineage_pre_refs(), PRE_REFS)?;
        let rev1 = world
            .publish(&[("a.txt", b"one"), ("b.txt", b"same")])
            .await?;
        if install {
            world.domain().install_package(&rev1).await?;
            world
                .package()
                .install_paths(&["a.txt".into(), "b.txt".into()])
                .await?;
        }
        Ok((world, rev1))
    }

    /// Each package's lineage as the refs hold it.
    async fn on_refs(&self) -> Res<BTreeMap<Namespace, PackageLineage>> {
        self.domain()
            .read_packages()
            .await?
            .into_iter()
            .map(|(namespace, lineage)| Ok((namespace, lineage?)))
            .collect()
    }

    /// Each package's lineage as `data.json` holds it, without the times,
    /// which the refs do not keep.
    async fn on_data_json(&self) -> Res<BTreeMap<Namespace, PackageLineage>> {
        self.domain_on(LineageStore::DataJson)
            .read_packages()
            .await?
            .into_iter()
            .map(|(namespace, lineage)| {
                let mut lineage = lineage?;
                lineage.paths.values_mut().for_each(|path| {
                    *path = PathState::default();
                });
                Ok((namespace, lineage))
            })
            .collect()
    }

    /// Authority is back with `data.json`, holding `expected`, and the
    /// rollback left nothing of its own but the archive.
    async fn assert_rolled_back(&self, expected: &BTreeMap<Namespace, PackageLineage>) -> Res {
        let paths = self.paths();
        assert!(!paths.domain_config().exists());
        assert!(!paths.lineage_rollback().exists());
        assert!(!paths.refs_dir().exists());
        assert_eq!(std::fs::read(paths.lineage_pre_refs())?, PRE_REFS);
        assert_eq!(&self.on_data_json().await?, expected);
        Ok(())
    }
}

/// A child running an operation, stopped at a failpoint until let go on.
struct Parked {
    child: Running,
    resume: PathBuf,
    _signals: TempDir,
}

impl Parked {
    fn start(world: &World, operation: Operation, arg: &str, name: &str) -> Res<Self> {
        let signals = TempDir::new()?;
        let mut child = Running(
            command(world, operation, arg)?
                .env(crate::failpoint::NAME_VAR, name)
                .env(crate::failpoint::DIR_VAR, signals.path())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()?,
        );
        let reached = signals.path().join(format!("{name}.reached"));
        child.until(&format!("{} reached {name}", operation.name()), || {
            reached.exists()
        })?;
        Ok(Self {
            child,
            resume: signals.path().join(format!("{name}.resume")),
            _signals: signals,
        })
    }

    /// Lets the child go on, and returns whether it succeeded.
    fn finish(self) -> Res<bool> {
        std::fs::write(&self.resume, b"")?;
        self.child.finish()
    }
}

/// A child, killed if the test ends before it does.
struct Running(std::process::Child);

impl Running {
    /// Waits for `done`, failing if the child ends first.
    fn until(&mut self, what: &str, done: impl Fn() -> bool) -> Res {
        let deadline = Instant::now() + LIMIT;
        while !done() {
            assert!(self.0.try_wait()?.is_none(), "it ended before {what}");
            assert!(Instant::now() < deadline, "never {what}");
            std::thread::sleep(Duration::from_millis(5));
        }
        Ok(())
    }

    fn finish(mut self) -> Res<bool> {
        let deadline = Instant::now() + LIMIT;
        loop {
            if let Some(status) = self.0.try_wait()? {
                return Ok(status.success());
            }
            assert!(Instant::now() < deadline, "it did not end");
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Runs `operation`, parked at `failpoint`, while a rollback in another
/// child takes step 1, then lets it go on before the rollback does. Returns
/// whether it succeeded.
fn across_step_one(world: &World, operation: Operation, arg: &str, failpoint: &str) -> Res<bool> {
    let parked = Parked::start(world, operation, arg, failpoint)?;
    let rollback = Parked::start(
        world,
        Operation::MigrateDown,
        "",
        "migrate.down.after-state",
    )?;
    let succeeded = parked.finish()?;
    assert!(rollback.finish()?, "the rollback completes");
    Ok(succeeded)
}

#[test(tokio::test)]
async fn an_operation_parked_before_its_lineage_when_the_rollback_begins_writes_none() -> Res {
    for operation in [Operation::Commit, Operation::Pull, Operation::Uninstall] {
        let dir = TempDir::new()?;
        let (world, _) = World::migrated(&dir, true).await?;
        match operation {
            Operation::Commit => std::fs::write(world.working_file("a.txt"), b"edit")?,
            Operation::Pull => {
                world
                    .publish(&[("a.txt", b"two"), ("b.txt", b"same")])
                    .await?;
            }
            _ => {}
        }
        let before = world.on_refs().await?;
        let failpoint = format!("flow.{}.before-lineage", operation.name());

        let succeeded = across_step_one(&world, operation, "", &failpoint)?;

        assert!(!succeeded, "{}", operation.name());
        world.assert_rolled_back(&before).await?;
    }
    Ok(())
}

#[test(tokio::test)]
async fn an_install_parked_when_the_rollback_begins_removes_what_it_placed() -> Res {
    let dir = TempDir::new()?;
    let (world, rev1) = World::migrated(&dir, false).await?;
    let before = world.on_refs().await?;

    let succeeded = across_step_one(
        &world,
        Operation::Install,
        &rev1.hash,
        "flow.install.before-create",
    )?;

    assert!(!succeeded);
    world.assert_rolled_back(&before).await?;
    let manifest = world.paths().installed_manifest(&target(), &rev1.hash);
    assert!(!manifest.exists());
    assert!(!world.working_file("").exists());
    Ok(())
}

/// The rollback is not held after step 1: step 2 waits on the commit's lock.
#[test(tokio::test)]
async fn a_commit_past_its_check_when_the_rollback_begins_lands_first() -> Res {
    let dir = TempDir::new()?;
    let (world, _) = World::migrated(&dir, true).await?;
    std::fs::write(world.working_file("a.txt"), b"edit")?;
    let commit = Parked::start(&world, Operation::Commit, "", "ref.before-temp")?;
    let mut rollback = Running(
        command(&world, Operation::MigrateDown, "")?
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()?,
    );
    let config = world.paths().domain_config();
    rollback.until("took step 1", || {
        std::fs::read(&config).is_ok_and(|bytes| {
            serde_json::from_slice::<serde_json::Value>(&bytes)
                .is_ok_and(|config| config["migration"] != "done")
        })
    })?;
    // Time for a step 2 that did not wait to read the ref.
    std::thread::sleep(Duration::from_millis(200));

    assert!(commit.finish()?);
    assert!(rollback.finish()?, "the rollback completes");
    let rolled = world.on_data_json().await?;
    assert!(rolled[&target()].commit.is_some(), "{rolled:?}");
    Ok(())
}

#[test(tokio::test)]
async fn a_home_change_after_the_rollback_begins_is_refused() -> Res {
    let dir = TempDir::new()?;
    let (world, _) = World::migrated(&dir, true).await?;
    let before = world.on_refs().await?;
    // Open before the rollback begins, as a running app is.
    let domain = world.domain();
    domain.get_home().await?;
    let rollback = Parked::start(
        &world,
        Operation::MigrateDown,
        "",
        "migrate.down.after-state",
    )?;

    let moved = domain.set_home(world.root.join("elsewhere")).await;

    assert!(
        matches!(
            moved,
            Err(Error::Lineage(LineageError::RefsNotWritable { .. }))
        ),
        "{moved:?}"
    );
    assert!(rollback.finish()?, "the rollback completes");
    world.assert_rolled_back(&before).await?;
    let legacy: serde_json::Value =
        serde_json::from_slice(&std::fs::read(world.paths().lineage())?)?;
    assert_eq!(legacy["home"], serde_json::json!(world.root.join("home")));
    Ok(())
}

/// A rollback finished but for removing the domain config, as a kill after
/// the publish leaves it.
#[test(tokio::test)]
async fn a_legacy_write_after_the_publish_survives_the_resume() -> Res {
    let dir = TempDir::new()?;
    let (world, _) = World::migrated(&dir, true).await?;
    let rolled = rolled_back(&world).await?;
    let name = rolled
        .archive
        .file_name()
        .expect("a name")
        .to_string_lossy();
    let config = serde_json::json!({
        "format": 1,
        "home": world.root.join("home"),
        "migration": {"rolling_back": {"archive": name}},
    });
    std::fs::write(world.paths().domain_config(), serde_json::to_vec(&config)?)?;
    std::fs::write(world.working_file("a.txt"), b"edit")?;
    Operation::LegacyCommit.run(&world, "").await?;
    let written = world.on_data_json().await?;

    down(&world).await?;

    assert!(written[&target()].commit.is_some());
    world.assert_rolled_back(&written).await
}

#[cfg(unix)]
mod killed {
    use test_log::test;

    use super::*;

    const STEPS: [&str; 4] = [
        "migrate.down.after-state",
        "migrate.down.after-staged",
        "migrate.down.after-archive",
        "migrate.down.after-publish",
    ];

    #[test(tokio::test)]
    async fn a_rollback_killed_at_each_step_resumes_to_the_refs_lineage() -> Res {
        for step in STEPS {
            let dir = TempDir::new()?;
            let (world, _) = World::migrated(&dir, true).await?;
            std::fs::write(world.working_file("a.txt"), b"edit")?;
            Operation::Commit.run(&world, "").await?;
            let before = world.on_refs().await?;

            assert!(
                killed_at(&world, Operation::MigrateDown, "", step)?,
                "{step}"
            );
            down(&world).await?;

            world.assert_rolled_back(&before).await?;
        }
        Ok(())
    }

    #[test(tokio::test)]
    async fn a_legacy_write_after_a_killed_publish_survives_the_resume() -> Res {
        let dir = TempDir::new()?;
        let (world, _) = World::migrated(&dir, true).await?;

        assert!(killed_at(
            &world,
            Operation::MigrateDown,
            "",
            "migrate.down.after-publish"
        )?);
        std::fs::write(world.working_file("a.txt"), b"edit")?;
        Operation::LegacyCommit.run(&world, "").await?;
        let written = world.on_data_json().await?;
        down(&world).await?;

        assert!(written[&target()].commit.is_some());
        world.assert_rolled_back(&written).await
    }
}
