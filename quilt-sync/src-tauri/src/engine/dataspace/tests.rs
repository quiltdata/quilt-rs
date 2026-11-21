use std::time::Duration;

use quilt_rs::dataspace::create::Create;
use quilt_rs::io::remote::mocks::MockRemote;
use quilt_rs::lineage::PackageState;
use tempfile::TempDir;

use super::*;

pub(crate) type Res = Result<(), Box<dyn std::error::Error>>;

/// The package every test makes.
pub(crate) const URI: &str = "quilt+s3://b#package=team/raw";

/// An app folder, a Domain and a home folder beside each other, and bucket
/// `b` on a mock remote.
pub(crate) struct World {
    pub(crate) dir: TempDir,
    pub(crate) remote: Arc<MockRemote>,
}

impl World {
    pub(crate) fn new() -> std::io::Result<Self> {
        let world = Self {
            dir: TempDir::new()?,
            remote: Arc::new(MockRemote::default()),
        };
        std::fs::create_dir_all(world.home())?;
        Ok(world)
    }

    /// Another machine on the same bucket.
    pub(crate) fn elsewhere(&self) -> std::io::Result<Self> {
        let world = Self {
            dir: TempDir::new()?,
            remote: Arc::clone(&self.remote),
        };
        std::fs::create_dir_all(world.home())?;
        Ok(world)
    }

    pub(crate) fn home(&self) -> PathBuf {
        self.dir.path().join("home")
    }

    pub(crate) async fn open(&self) -> Result<Dataspace<MockRemote>, Error> {
        self.open_with(&self.home()).await
    }

    async fn open_with(&self, fallback: &Path) -> Result<Dataspace<MockRemote>, Error> {
        Dataspace::open(
            Arc::clone(&self.remote),
            self.dir.path().join("domain"),
            self.dir.path().join("app"),
            fallback,
            self.dir.path().join("no-user-rules"),
        )
        .await
    }

    pub(crate) fn folder(&self) -> PathBuf {
        self.home().join("b/team/raw")
    }

    pub(crate) fn write(&self, key: &str, body: &str) -> std::io::Result<()> {
        let path = self.folder().join(key);
        std::fs::create_dir_all(path.parent().expect("a parent"))?;
        std::fs::write(path, body)
    }

    pub(crate) fn text(&self, key: &str) -> Option<String> {
        std::fs::read_to_string(self.folder().join(key)).ok()
    }

    /// `team/raw` created under `sync` with `a.txt`, and pushed to `b`.
    pub(crate) async fn published(
        &self,
    ) -> Result<Dataspace<MockRemote>, Box<dyn std::error::Error>> {
        let ds = self.open().await?;
        self.write("a.txt", "v1")?;
        ds.create(&Create::new(URI)).await?;
        ds.sync("first").await?;
        Ok(ds)
    }
}

pub(crate) fn raw() -> Namespace {
    Namespace::from(("team", "raw"))
}

/// With no default dataspace chosen, the ref path's home folder becomes
/// it, and stays it once chosen.
#[tokio::test]
async fn the_default_dataspace_is_chosen_once() -> Res {
    let world = World::new()?;
    let ds = world.open().await?;
    assert_eq!(ds.dataspace(), world.home());
    assert!(world.home().join("data.toml").exists());

    let other = world.dir.path().join("other");
    std::fs::create_dir_all(&other)?;
    let again = world.open_with(&other).await?;
    assert_eq!(again.dataspace(), world.home());
    Ok(())
}

/// A package is named by the namespace the pages give, as the entry the
/// contract declares it in.
#[tokio::test]
async fn a_package_is_named_by_its_namespace() -> Res {
    let world = World::new()?;
    let ds = world.published().await?;

    let entry = ds.entry(&raw()).await?;
    assert_eq!(entry.registry, Registry::S3("b".to_string()));
    let missing = ds.entry(&Namespace::from(("team", "none"))).await;
    assert!(
        matches!(&missing, Err(Error::Dataspace(text)) if text.contains("team/none")),
        "{missing:?}"
    );
    Ok(())
}

/// Commit, status and undo go through the dataspace's lock: a commit is a
/// pending one in status, and undo puts the folder back.
#[tokio::test]
async fn a_commit_shows_in_status_and_undo_takes_it_back() -> Res {
    let world = World::new()?;
    let ds = world.published().await?;
    world.write("a.txt", "v2")?;
    assert_eq!(
        ds.entry_status(&raw(), false).await?.state(),
        PackageState::PendingChanges { files: 1 }
    );

    let committed = ds.commit(&raw(), "second").await?;
    assert!(committed.commit.is_some(), "{committed:?}");
    assert_eq!(
        ds.entry_status(&raw(), false).await?.state(),
        PackageState::PendingCommit
    );
    world.write("a.txt", "v3")?;
    ds.commit(&raw(), "third").await?;

    // One step back, to the commit before, not to `latest`.
    let undone = ds.undo(&raw()).await;
    assert!(undone.is_ok(), "{undone:?}");
    assert_eq!(world.text("a.txt").as_deref(), Some("v2"));
    assert_eq!(
        ds.entry_status(&raw(), false).await?.state(),
        PackageState::PendingCommit
    );
    ds.undo(&raw()).await?;
    assert_eq!(world.text("a.txt").as_deref(), Some("v1"));
    assert_eq!(
        ds.entry_status(&raw(), false).await?.state(),
        PackageState::Latest
    );
    Ok(())
}

/// A commit undone stays in the store, where reattach finds it and puts it
/// back on the lock's base, to be pushed.
#[tokio::test]
async fn an_undone_commit_is_reattached() -> Res {
    let world = World::new()?;
    let ds = world.published().await?;
    let base = ds.entry_status(&raw(), false).await?.hashes.base;
    world.write("a.txt", "v2")?;
    let hash = ds
        .commit(&raw(), "second")
        .await?
        .commit
        .ok_or("nothing committed")?;
    ds.undo(&raw()).await?;

    let listed = ds.unattached(&raw()).await?;
    assert_eq!(
        listed.iter().map(|commit| &commit.hash).collect::<Vec<_>>(),
        [&hash]
    );
    ds.reattach(&raw(), &hash).await?;

    let status = ds.entry_status(&raw(), false).await?;
    assert_eq!(status.unpushed, [hash]);
    assert!(base.is_some(), "the published package has a base");
    assert_eq!(status.hashes.base, base, "reattached on another base");
    assert_eq!(ds.unattached(&raw()).await?.len(), 0);
    Ok(())
}

/// Uninstall holds the package's lock in its store, so it waits for a tick
/// holding it, and the tick never acts on a package half uninstalled
/// (25q known limit).
#[tokio::test(flavor = "multi_thread")]
async fn uninstall_waits_for_a_tick_holding_the_package() -> Res {
    let world = World::new()?;
    let ds = Arc::new(world.published().await?);
    let key = ds.entry(&raw()).await?.key();
    let held = ds.storage().lock_exclusive(ds.package_lock(&key)?).await?;

    let uninstalling = tokio::spawn({
        let ds = Arc::clone(&ds);
        async move { ds.uninstall(&raw(), false).await.map(|_| ()) }
    });
    tokio::time::sleep(Duration::from_millis(300)).await;

    assert!(!uninstalling.is_finished(), "uninstall did not wait");
    assert!(ds.entry(&raw()).await.is_ok(), "the declaration went");
    drop(held);
    uninstalling.await??;
    assert!(ds.entry(&raw()).await.is_err());
    assert!(!world.folder().exists());
    Ok(())
}

/// The watcher and status take the contract's `[ignore]` table.
#[tokio::test]
async fn the_ignore_rules_take_the_contracts_table() -> Res {
    let world = World::new()?;
    let ds = world.published().await?;
    let contract = std::fs::read_to_string(world.home().join("data.toml"))?;
    std::fs::write(
        world.home().join("data.toml"),
        format!("{contract}\n[ignore]\nextend_patterns = [\"*.tmp\"]\n"),
    )?;

    let rules = ds.ignore_rules(&world.folder()).await?;

    assert!(
        rules
            .file_ignored_by(Path::new("scratch.tmp"), false)
            .is_some()
    );
    assert!(rules.file_ignored_by(Path::new("a.txt"), false).is_none());
    Ok(())
}
