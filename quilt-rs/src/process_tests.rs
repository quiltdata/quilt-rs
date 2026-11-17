//! Package operations run in a child process, on a domain and a remote both
//! kept on disk, so a test can kill one with nothing unwinding and look at
//! what it left.

use std::path::Path;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

use aws_sdk_s3::primitives::ByteStream;
use tempfile::TempDir;

use crate::Res;
use crate::checksum::calculate_hash;
use crate::flow::UserMeta;
use crate::installed_package::InstalledPackage;
use crate::io::manifest::tag_latest;
use crate::io::remote::HostConfig;
use crate::io::remote::Remote;
use crate::io::remote::mocks::MockRemote;
use crate::io::storage::LocalStorage;
use crate::lineage::DomainLineageIo;
use crate::lineage::LineageStore;
use crate::local_domain::LocalDomain;
use crate::manifest::Manifest;
use crate::manifest::ManifestRow;
use crate::manifest::TopHasher;
use crate::paths::DomainPaths;
use quilt_uri::ManifestUri;
use quilt_uri::Namespace;
use quilt_uri::S3Uri;

mod killed;

const BUCKET: &str = "b";

/// The package every operation acts on.
fn target() -> Namespace {
    ("acme", "data").into()
}

/// A package no operation touches, under another owner, so installing the
/// target creates its owner's folder.
fn bystander() -> Namespace {
    ("other", "notes").into()
}

type Domain = LocalDomain<LocalStorage, MockRemote>;

/// A domain under `root/domain`, its home `root/home`, and the remote it
/// pulls from and pushes to, kept in `root/remote`.
struct World {
    root: PathBuf,
    store: LineageStore,
}

impl World {
    /// A domain on `store` holding [`bystander`], created locally.
    async fn new(dir: &TempDir, store: LineageStore) -> Res<Self> {
        let world = Self {
            root: dir.path().to_path_buf(),
            store,
        };
        let domain = world.domain();
        domain.set_home(world.root.join("home")).await?;
        let source = world.root.join("source");
        std::fs::create_dir_all(&source)?;
        std::fs::write(source.join("notes.txt"), b"notes")?;
        domain
            .create_package(bystander(), Some(source), None, UserMeta::Clear)
            .await?;
        Ok(world)
    }

    fn domain(&self) -> Domain {
        let paths = DomainPaths::new(self.root.join("domain"));
        let lineage = DomainLineageIo::new(paths.lineage());
        LocalDomain::with_parts(paths, lineage, LocalStorage::new(), Arc::new(self.remote()))
            .with_lineage_store(self.store)
    }

    fn remote(&self) -> MockRemote {
        MockRemote::at(self.root.join("remote"))
    }

    fn package(&self) -> InstalledPackage<LocalStorage, MockRemote> {
        self.domain().create_installed_package(target())
    }

    fn working_file(&self, key: &str) -> PathBuf {
        self.root.join("home").join(target().to_string()).join(key)
    }

    /// Publishes `rows` as [`target`]'s `latest` and returns its URI.
    async fn publish(&self, rows: &[(&str, &[u8])]) -> Res<ManifestUri> {
        let remote = self.remote();
        let storage = LocalStorage::new();
        let mut manifest = Manifest::default();
        for (key, body) in rows {
            let scratch = self.root.join("scratch").join(hex::encode(body));
            std::fs::create_dir_all(scratch.parent().expect("a folder"))?;
            std::fs::write(&scratch, body)?;
            let row =
                calculate_hash(&storage, &scratch, Path::new(key), &HostConfig::default()).await?;
            let version = hex::encode(body);
            let physical_key = format!("s3://{BUCKET}/{}/{key}?versionId={version}", target());
            remote
                .put_object(None, &S3Uri::from_str(&physical_key)?, body.to_vec())
                .await?;
            manifest.rows.push(ManifestRow {
                physical_key,
                ..row
            });
        }
        let mut hasher = TopHasher::new();
        hasher.append_header(&manifest.header)?;
        for row in &manifest.rows {
            hasher.append(row)?;
        }
        let uri = revision(hasher.finalize());
        remote
            .put_object(
                None,
                &S3Uri::from_str(&format!("s3://{BUCKET}/.quilt/packages/{}", uri.hash))?,
                ByteStream::from(&manifest),
            )
            .await?;
        tag_latest(&remote, &uri).await?;
        Ok(uri)
    }
}

/// [`target`]'s revision `hash` in the remote.
fn revision(hash: String) -> ManifestUri {
    ManifestUri {
        origin: None,
        bucket: BUCKET.to_string(),
        namespace: target(),
        hash,
    }
}

const CHILD_ROOT: &str = "QUILT_RS_TEST_PROCESS_ROOT";
const CHILD_STORE: &str = "QUILT_RS_TEST_PROCESS_STORE";
const CHILD_OPERATION: &str = "QUILT_RS_TEST_PROCESS_OPERATION";
const CHILD_ARG: &str = "QUILT_RS_TEST_PROCESS_ARG";

/// An operation a child runs on [`target`], as the CLI's command of that
/// name does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Operation {
    Commit,
    Pull,
    /// Installs the revision whose hash is the argument.
    Install,
    Push,
    Uninstall,
}

impl Operation {
    fn name(self) -> &'static str {
        match self {
            Operation::Commit => "commit",
            Operation::Pull => "pull",
            Operation::Install => "install",
            Operation::Push => "push",
            Operation::Uninstall => "uninstall",
        }
    }

    fn named(name: &str) -> Self {
        [
            Operation::Commit,
            Operation::Pull,
            Operation::Install,
            Operation::Push,
            Operation::Uninstall,
        ]
        .into_iter()
        .find(|operation| operation.name() == name)
        .unwrap_or_else(|| panic!("no operation {name}"))
    }

    async fn run(self, world: &World, arg: &str) -> Res {
        let package = world.package();
        let host_config = Some(HostConfig::default());
        match self {
            Operation::Commit => {
                package
                    .commit("edit".to_string(), UserMeta::Keep, None, host_config)
                    .await?;
            }
            // As `quilt pull` does: the stored scope, read under the lock.
            Operation::Pull => {
                let locked = package.lock().await?;
                let scope = package.lineage().await?.sync_scope;
                Box::pin(locked.pull(host_config, scope)).await?;
            }
            Operation::Install => {
                world
                    .domain()
                    .install_package(&revision(arg.to_string()))
                    .await?;
            }
            Operation::Push => {
                package.push(host_config).await?;
            }
            Operation::Uninstall => world.domain().uninstall_package(target()).await?,
        }
        Ok(())
    }
}

/// Not a test on its own: the child half of the tests in this module. It
/// runs the operation `QUILT_RS_TEST_PROCESS_OPERATION` names on the world
/// at `QUILT_RS_TEST_PROCESS_ROOT`.
#[test]
#[ignore = "run only as the child of a process test"]
fn child_runs_an_operation() -> Res {
    let (Ok(root), Ok(store), Ok(operation)) = (
        std::env::var(CHILD_ROOT),
        std::env::var(CHILD_STORE),
        std::env::var(CHILD_OPERATION),
    ) else {
        return Ok(());
    };
    let world = World {
        root: PathBuf::from(root),
        store: if store == "refs" {
            LineageStore::Refs
        } else {
            LineageStore::DataJson
        },
    };
    let arg = std::env::var(CHILD_ARG).unwrap_or_default();
    runtime()?.block_on(Operation::named(&operation).run(&world, &arg))
}

fn runtime() -> Res<tokio::runtime::Runtime> {
    Ok(tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?)
}

/// `operation` on `world` in a child process.
fn spawn(world: &World, operation: Operation, arg: &str) -> Res<std::process::Child> {
    Ok(command(world, operation, arg)?.spawn()?)
}

fn command(world: &World, operation: Operation, arg: &str) -> Res<std::process::Command> {
    let mut child = std::process::Command::new(std::env::current_exe()?);
    child
        .args([
            "--exact",
            "process_tests::child_runs_an_operation",
            "--ignored",
            "--nocapture",
        ])
        .env(CHILD_ROOT, &world.root)
        .env(
            CHILD_STORE,
            match world.store {
                LineageStore::DataJson => "data.json",
                LineageStore::Refs => "refs",
            },
        )
        .env(CHILD_OPERATION, operation.name())
        .env(CHILD_ARG, arg);
    Ok(child)
}

/// Runs `operation` in a child and kills it once it reaches the failpoint
/// `name`. Returns whether it did: a child that ends first is not killed.
fn killed_at(world: &World, operation: Operation, arg: &str, name: &str) -> Res<bool> {
    let signals = TempDir::new()?;
    // Its output stays out of the test's: a killed child has nothing to
    // report, and nextest saw the test's own output kept open past its end.
    let mut child = command(world, operation, arg)?
        .env(crate::failpoint::NAME_VAR, name)
        .env(crate::failpoint::DIR_VAR, signals.path())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    let reached = signals.path().join(format!("{name}.reached"));
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut exited = None;
    while !reached.exists() && exited.is_none() && Instant::now() < deadline {
        // Not `?`: the child is killed below on every path.
        exited = child.try_wait().unwrap_or(None);
        std::thread::sleep(Duration::from_millis(5));
    }
    let _ = child.kill();
    child.wait()?;
    assert!(
        reached.exists() || exited.is_some(),
        "the child neither reached {name} nor ended"
    );
    Ok(reached.exists())
}
