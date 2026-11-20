use super::*;

use tempfile::TempDir;
use test_log::test;

use crate::dataspace::contract::Contract;
use crate::dataspace::lock::Lock;
use crate::dataspace::lock::LockEntry;
use crate::dataspace::sync::sync;
use crate::io::remote::mocks::MockRemote;
use crate::io::storage::LocalStorage;
use crate::queue::Queue;
use quilt_uri::Namespace;

type Res = Result<(), Box<dyn std::error::Error>>;

const LATEST: &str = "s3://b/.quilt/named_packages/team/new/latest";

/// A dataspace with `b` as its default registry, its Domain beside it.
struct World {
    dir: TempDir,
    storage: LocalStorage,
    remote: MockRemote,
}

impl World {
    fn new() -> std::io::Result<Self> {
        let world = Self {
            dir: TempDir::new()?,
            storage: LocalStorage::new(),
            remote: MockRemote::default(),
        };
        std::fs::create_dir_all(world.dataspace())?;
        std::fs::write(
            world.dataspace().join(CONTRACT_FILE),
            "registry = \"s3://b\"\n",
        )?;
        Ok(world)
    }

    fn dataspace(&self) -> PathBuf {
        self.dir.path().join("rnaseq")
    }

    fn domain(&self) -> DomainRoot {
        DomainRoot::new(self.dir.path().join("domain"))
    }

    fn user_rules(&self) -> PathBuf {
        self.dir.path().join("user.quiltignore")
    }

    /// This machine's lock writer, its local registry named `lab-mac`.
    async fn writer(&self) -> Result<Writer, Box<dyn std::error::Error>> {
        self.domain()
            .local()
            .rename(&self.storage, "lab-mac")
            .await?;
        Ok(Writer {
            host: "here".to_string(),
            user: "me".to_string(),
            local_registry: "lab-mac".to_string(),
        })
    }

    async fn create(&self, request: &Create) -> Result<Created, CreateError> {
        let writer = self.writer().await.expect("a writer");
        create(
            &self.storage,
            &self.remote,
            &self.domain(),
            &self.dataspace(),
            &writer,
            request,
            &self.user_rules(),
        )
        .await
    }

    async fn contract(&self) -> Result<Contract, ContractError> {
        Contract::read(&self.storage, &self.dataspace(), Some("lab-mac")).await
    }

    async fn lock(&self) -> Result<Lock, Box<dyn std::error::Error>> {
        Ok(Lock::read(&self.storage, &self.contract().await?)
            .await?
            .ok_or("no lock")?)
    }
}

fn key(registry: Registry, package: &str) -> PackageKey {
    PackageKey {
        registry,
        package: Namespace::try_from(package).expect("package"),
    }
}

/// A package created in a bucket from the files in its folder is declared
/// under `sync` with no revision, and its first commit is in the lock with
/// no base (25m), kept in the bucket's store; the next sync pushes it as a
/// first publication, expecting no `latest` (25.9), and bases the lock on
/// it.
#[test(tokio::test)]
async fn a_created_package_commits_with_no_base_and_its_first_push_expects_no_latest() -> Res {
    let world = World::new()?;
    let folder = world.dataspace().join("team/new");
    std::fs::create_dir_all(&folder)?;
    std::fs::write(folder.join("a.txt"), b"alpha")?;

    let created = world
        .create(&Create::new("quilt+s3://b#package=team/new"))
        .await?;

    let commit = created.commit.clone().ok_or("no commit")?;
    assert_eq!(created.entry.access, Access::Sync);
    assert_eq!(created.entry.revision, RevisionPointer::Tag(Tag::Latest));
    let text = std::fs::read_to_string(world.dataspace().join(CONTRACT_FILE))?;
    assert!(
        text.contains("sync = [{ package = \"team/new\" }]"),
        "{text}"
    );
    let new = key(Registry::S3("b".to_string()), "team/new");
    let lock = world.lock().await?;
    assert_eq!(
        lock.packages[&new],
        LockEntry {
            base: None,
            commits: vec![commit.clone()],
        }
    );
    assert_eq!(
        lock.header.data_toml_hash,
        world.contract().await?.data_toml_hash()
    );
    let store = world.domain().registry("b")?;
    let namespace = Namespace::try_from("team/new")?;
    assert!(store.installed_manifest(&namespace, &commit).exists());
    let manifest = Manifest::from_path(
        &world.storage,
        &store.installed_manifest(&namespace, &commit),
    )
    .await?;
    assert_eq!(
        manifest.header.message,
        crate::manifest::HeaderMessage::Text(DEFAULT_MESSAGE.to_string())
    );

    let queue = Queue::in_memory().await?;
    let writer = world.writer().await?;
    Box::pin(sync(
        &world.storage,
        &world.remote,
        &world.domain(),
        &queue,
        &world.dataspace(),
        &writer,
        "autosync",
        &world.user_rules(),
    ))
    .await?;

    let latest = world.remote.current(LATEST).await?.ok_or("no latest")?;
    assert_eq!(String::from_utf8(latest)?, commit);
    assert_eq!(world.lock().await?.packages[&new], LockEntry::at(commit));
    Ok(())
}

/// A folder outside the dataspace is packaged where it is, into the local
/// registry: the entry names it, and nothing is copied.
#[test(tokio::test)]
async fn a_folder_outside_the_dataspace_is_created_in_the_local_registry() -> Res {
    let world = World::new()?;
    let folder = world.dir.path().join("analysis");
    std::fs::create_dir_all(&folder)?;
    std::fs::write(folder.join("result.csv"), b"1,2")?;
    let mut request = Create::new("quilt+local://#package=me/analysis");
    request.folder = Some(folder.clone());
    request.message = Some("first look".to_string());

    let created = world.create(&request).await?;

    let commit = created.commit.ok_or("no commit")?;
    assert_eq!(created.entry.folder, folder);
    let local = world.domain().local();
    let namespace = Namespace::try_from("me/analysis")?;
    assert!(local.installed_manifest(&namespace, &commit).exists());
    assert_eq!(local.read_latest(&world.storage, &namespace).await?, None);
    let text = std::fs::read_to_string(world.dataspace().join(CONTRACT_FILE))?;
    assert!(
        text.contains("registry = \"quilt+local://lab-mac\""),
        "{text}"
    );
    assert!(!world.dataspace().join("me").exists());
    let lock = world.lock().await?;
    let new = key(Registry::Local("lab-mac".to_string()), "me/analysis");
    assert_eq!(lock.packages[&new].base, None);
    Ok(())
}

/// An empty folder, or none, is declared and left out of the lock, for the
/// next sync; `write` and `replace` are declared and never committed.
#[test(tokio::test)]
async fn nothing_to_commit_declares_the_package_only() -> Res {
    let world = World::new()?;

    let created = world
        .create(&Create::new("quilt+s3://b#package=team/new"))
        .await?;
    let mut written = Create::new("quilt+s3://b#package=team/out");
    written.access = Access::Write;
    std::fs::create_dir_all(world.dataspace().join("team/out"))?;
    std::fs::write(world.dataspace().join("team/out/a.txt"), b"alpha")?;
    let out = world.create(&written).await?;

    assert_eq!(created.commit, None);
    assert_eq!(out.commit, None);
    assert_eq!(out.entry.access, Access::Write);
    assert!(world.lock().await?.packages.is_empty());
    assert_eq!(world.contract().await?.entries().len(), 2);
    Ok(())
}

/// A folder that is a link to the Domain holds it as much as the Domain's own
/// path does.
#[cfg(unix)]
#[test(tokio::test)]
async fn a_link_to_the_domain_is_refused() -> Res {
    let world = World::new()?;
    std::fs::create_dir_all(world.domain().root())?;
    let link = world.dir.path().join("link");
    std::os::unix::fs::symlink(world.domain().root(), &link)?;
    let mut request = Create::new("quilt+s3://b#package=team/home");
    request.folder = Some(link);
    let refused = world.create(&request).await;
    assert!(
        matches!(refused, Err(CreateError::HoldsDomain(_))),
        "{refused:?}"
    );
    Ok(())
}

/// What cannot be a new package is refused before `data.toml` changes: one
/// the contract declares, one whose registry has a `latest`, a name outside
/// quilt3's rule, a revision, `read`, and a folder that holds the Domain.
#[test(tokio::test)]
async fn what_is_not_a_new_package_is_refused() -> Res {
    let world = World::new()?;
    std::fs::write(
        world.dataspace().join(CONTRACT_FILE),
        "registry = \"s3://b\"\n[packages]\nsync = [\"team/raw\"]\n",
    )?;
    let before = std::fs::read(world.dataspace().join(CONTRACT_FILE))?;
    crate::dataspace::store::tests::publish_local(
        &world.storage,
        &world.domain().local(),
        "me/old",
        &[("a.txt", b"alpha")],
    )
    .await?;
    let mut domain = Create::new("quilt+s3://b#package=team/home");
    domain.folder = Some(world.domain().root().to_path_buf());
    let mut read = Create::new("quilt+s3://b#package=team/fresh");
    read.access = Access::Read;

    let cases = [
        (Create::new("quilt+s3://b#package=team/raw"), "declared"),
        (Create::new("quilt+local://#package=me/old"), "exists"),
        (Create::new("quilt+s3://b#package=team/my.data"), "named"),
        (
            Create::new(format!(
                "quilt+s3://b#package=team/fresh@{}",
                "1".repeat(64)
            )),
            "named",
        ),
        (read, "read"),
        (domain, "domain"),
    ];
    for (request, why) in cases {
        let refused = world.create(&request).await;
        let matched = matches!(
            (&refused, why),
            (Err(CreateError::Declared(_)), "declared")
                | (Err(CreateError::Exists(_)), "exists")
                | (Err(CreateError::Named { .. }), "named")
                | (Err(CreateError::Read), "read")
                | (Err(CreateError::HoldsDomain(_)), "domain")
        );
        assert!(matched, "{why}: {refused:?}");
        assert_eq!(
            std::fs::read(world.dataspace().join(CONTRACT_FILE))?,
            before,
            "{why}"
        );
    }
    Ok(())
}
