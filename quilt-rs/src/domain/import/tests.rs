use super::*;

use bytes::Bytes;
use tempfile::TempDir;
use test_log::test;

use crate::LogicalKey;
use crate::dataspace::contract::Access;
use crate::dataspace::contract::Entry;
use crate::dataspace::contract::PackagePath;
use crate::dataspace::contract::Paths;
use crate::flow::WorkflowCheck;
use crate::flow::record;
use crate::io::manifest::RowContent;
use crate::io::remote::mocks::MockRemote;
use crate::io::storage::LocalStorage;
use crate::lineage::Home;
use crate::lineage::PathState;
use crate::manifest::ManifestRow;
use crate::package_lock::PackageLock;
use quilt_uri::ManifestUri;

type Res<T = ()> = Result<T, Box<dyn std::error::Error>>;

const CATALOG: &str = "catalog.example.com";

/// A domain as a build with refs keeps it, in `app/`, its home `home/`, and
/// the Domain to import it into, `domain/`.
struct Old {
    dir: TempDir,
    paths: DomainPaths,
    storage: LocalStorage,
}

impl Old {
    fn new() -> Res<Self> {
        let dir = TempDir::new()?;
        let paths = DomainPaths::new(dir.path().join("app"));
        std::fs::create_dir_all(paths.dot_quilt_dir())?;
        let config = serde_json::json!({
            "format": 1,
            "home": dir.path().join("home"),
            "migration": "done",
        });
        std::fs::write(paths.domain_config(), serde_json::to_vec(&config)?)?;
        Ok(Self {
            dir,
            paths,
            storage: LocalStorage::new(),
        })
    }

    fn app(&self) -> PathBuf {
        self.dir.path().join("app")
    }

    fn home(&self) -> PathBuf {
        self.dir.path().join("home")
    }

    fn domain(&self) -> DomainRoot {
        DomainRoot::new(self.dir.path().join("domain"))
    }

    async fn import(&self) -> Result<Imported, ImportError> {
        import(&self.storage, &self.app(), &self.domain()).await
    }

    /// Commits `files` on top of `lineage` into the old store, as a build
    /// with refs does; returns the lineage with the commit pending.
    async fn commit(
        &self,
        namespace: &Namespace,
        lineage: PackageLineage,
        files: &[(&str, &'static [u8])],
    ) -> Res<PackageLineage> {
        let mut rows = Vec::new();
        let mut content = BTreeMap::new();
        for (name, bytes) in files {
            let key = LogicalKey::new(*name)?;
            rows.push(ManifestRow {
                logical_key: key.clone(),
                ..ManifestRow::default()
            });
            content.insert(key, RowContent::Bytes(Bytes::from_static(bytes)));
        }
        let manifest = Manifest {
            rows,
            ..Manifest::default()
        };
        let (lineage, _) = record(
            &self.paths,
            &self.storage,
            &MockRemote::default(),
            WorkflowCheck::Warn { host: None },
            namespace,
            lineage,
            manifest,
            &content,
        )
        .await?;
        Ok(lineage)
    }

    /// Writes `lineage` as `namespace`'s ref.
    async fn install(&self, namespace: &Namespace, lineage: &PackageLineage) -> Res {
        let held = PackageLock::take(&self.storage, &self.paths, namespace).await?;
        RefStore::new(self.paths.clone())
            .create(&self.storage, &held, lineage, &Home::from(self.home()))
            .await?;
        Ok(())
    }

    /// Every file under the old folder, with its bytes.
    fn snapshot(&self) -> Res<BTreeMap<PathBuf, Vec<u8>>> {
        let mut files = BTreeMap::new();
        let mut folders = vec![self.app()];
        while let Some(folder) = folders.pop() {
            for entry in std::fs::read_dir(folder)? {
                let path = entry?.path();
                if path.is_dir() {
                    folders.push(path);
                } else {
                    files.insert(path.clone(), std::fs::read(&path)?);
                }
            }
        }
        Ok(files)
    }

    async fn local_name(&self) -> Res<String> {
        Ok(self.domain().local().names(&self.storage).await?.name)
    }

    /// The default dataspace's contract and lock.
    async fn dataspace(&self) -> Res<(Contract, Lock)> {
        let local_name = self.local_name().await?;
        let contract = Contract::read(&self.storage, &self.home(), Some(&local_name))
            .await
            .map_err(std::io::Error::other)?;
        let lock = Lock::read(&self.storage, &contract)
            .await
            .map_err(std::io::Error::other)?
            .expect("a lock");
        Ok((contract, lock))
    }
}

fn remote(bucket: &str, namespace: &Namespace, hash: &str) -> ManifestUri {
    ManifestUri {
        origin: Some(CATALOG.parse().expect("a host")),
        bucket: bucket.to_string(),
        namespace: namespace.clone(),
        hash: hash.to_string(),
    }
}

fn tracked(keys: &[&str]) -> crate::lineage::LineagePaths {
    keys.iter()
        .map(|key| (PathBuf::from(key), PathState::default()))
        .collect()
}

/// A package installed from `bucket` at a base, with two local commits on
/// it: its hashes, base first.
async fn pushed_with_commits(old: &Old, namespace: &Namespace, bucket: &str) -> Res<Vec<String>> {
    let published = old
        .commit(namespace, PackageLineage::default(), &[("a.txt", b"base")])
        .await?;
    let base = published.commit.expect("a commit").hash;
    let mut lineage = PackageLineage {
        base_hash: base.clone(),
        latest_hash: base.clone(),
        sync_scope: SyncScope::EntirePackage,
        paths: tracked(&["a.txt"]),
        ..PackageLineage::default()
    };
    lineage = old.commit(namespace, lineage, &[("a.txt", b"one")]).await?;
    lineage = old
        .commit(namespace, lineage, &[("a.txt", b"two"), ("b.txt", b"new")])
        .await?;
    let commit = lineage.commit.clone().expect("a commit");
    lineage.remote_uri = Some(remote(bucket, namespace, &base));
    old.install(namespace, &lineage).await?;
    Ok(vec![
        base,
        commit.prev_hashes[0].clone(),
        commit.hash.clone(),
    ])
}

/// One step a [`Recording`] storage saw, by the path it named.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Step {
    Write(PathBuf),
    SyncFile(PathBuf),
    SyncDir(PathBuf),
    Rename { from: PathBuf, to: PathBuf },
}

/// [`LocalStorage`] that records the writes, syncs and renames it is asked
/// for, in order, and that adds a byte to the file `changing`, if any, each
/// time it is opened: a ref another process rewrites under the import.
#[derive(Default)]
struct Recording {
    inner: LocalStorage,
    steps: std::sync::Mutex<Vec<Step>>,
    changing: Option<PathBuf>,
}

impl Recording {
    fn record(&self, step: Step) {
        self.steps.lock().expect("not poisoned").push(step);
    }

    fn steps(&self) -> Vec<Step> {
        self.steps.lock().expect("not poisoned").clone()
    }
}

impl Storage for Recording {
    async fn copy(
        &self,
        from: impl AsRef<Path> + Send,
        to: impl AsRef<Path> + Send,
    ) -> crate::Res<u64> {
        self.inner.copy(from, to).await
    }
    async fn create_dir_all(&self, path: impl AsRef<Path> + Send) -> crate::Res {
        self.inner.create_dir_all(path).await
    }
    async fn create_file(&self, path: impl AsRef<Path>) -> crate::Res<tokio::fs::File> {
        self.inner.create_file(path).await
    }
    async fn exists(&self, path: impl AsRef<Path>) -> bool {
        self.inner.exists(path).await
    }
    async fn is_file(&self, path: impl AsRef<Path>) -> bool {
        self.inner.is_file(path).await
    }
    async fn modified_timestamp(
        &self,
        path: impl AsRef<Path>,
    ) -> crate::Res<chrono::DateTime<chrono::Utc>> {
        self.inner.modified_timestamp(path).await
    }
    async fn open_file(&self, path: impl AsRef<Path> + Send) -> crate::Res<tokio::fs::File> {
        if self.changing.as_deref() == Some(path.as_ref()) {
            let mut bytes = std::fs::read(path.as_ref())?;
            bytes.push(b' ');
            std::fs::write(path.as_ref(), bytes)?;
        }
        self.inner.open_file(path).await
    }
    async fn read_byte_stream(
        &self,
        path: impl AsRef<Path> + Send + Sync,
    ) -> crate::Res<ByteStream> {
        self.inner.read_byte_stream(path).await
    }
    async fn read_dir(
        &self,
        path: impl AsRef<Path> + Send + Sync,
    ) -> crate::Res<tokio::fs::ReadDir> {
        self.inner.read_dir(path).await
    }
    async fn remove_dir(&self, path: impl AsRef<Path> + Send) -> crate::Res {
        self.inner.remove_dir(path).await
    }
    async fn remove_dir_all(&self, path: impl AsRef<Path> + Send) -> crate::Res {
        self.inner.remove_dir_all(path).await
    }
    async fn remove_file(&self, path: impl AsRef<Path> + Send) -> std::io::Result<()> {
        self.inner.remove_file(path).await
    }
    async fn rename(
        &self,
        from: impl AsRef<Path> + Send,
        to: impl AsRef<Path> + Send,
    ) -> crate::Res {
        self.record(Step::Rename {
            from: from.as_ref().to_path_buf(),
            to: to.as_ref().to_path_buf(),
        });
        self.inner.rename(from, to).await
    }
    async fn write_byte_stream(
        &self,
        path: impl AsRef<Path> + Send + Sync,
        body: ByteStream,
    ) -> crate::Res {
        self.record(Step::Write(path.as_ref().to_path_buf()));
        self.inner.write_byte_stream(path, body).await
    }
    async fn write_byte_stream_via(
        &self,
        path: impl AsRef<Path> + Send + Sync,
        temp_name: impl AsRef<Path> + Send + Sync,
        body: ByteStream,
        tag: Option<&'static str>,
    ) -> crate::Res {
        self.record(Step::Write(path.as_ref().to_path_buf()));
        self.inner
            .write_byte_stream_via(path, temp_name, body, tag)
            .await
    }
    async fn sync_dir(&self, path: impl AsRef<Path> + Send) {
        self.record(Step::SyncDir(path.as_ref().to_path_buf()));
        self.inner.sync_dir(path).await;
    }
    async fn sync_file(&self, path: impl AsRef<Path> + Send) -> crate::Res {
        self.record(Step::SyncFile(path.as_ref().to_path_buf()));
        self.inner.sync_file(path).await
    }
    async fn create_private_dir(&self, path: impl AsRef<Path> + Send) -> crate::Res {
        self.inner.create_private_dir(path).await
    }
    async fn write_private_byte_stream(
        &self,
        path: impl AsRef<Path> + Send + Sync,
        body: ByteStream,
    ) -> crate::Res {
        self.record(Step::Write(path.as_ref().to_path_buf()));
        self.inner.write_private_byte_stream(path, body).await
    }
    async fn lock_exclusive(
        &self,
        path: impl AsRef<Path> + Send,
    ) -> crate::Res<crate::io::storage::LockGuard> {
        self.inner.lock_exclusive(path).await
    }
    async fn try_lock_exclusive(
        &self,
        path: impl AsRef<Path> + Send,
    ) -> crate::Res<Option<crate::io::storage::LockGuard>> {
        self.inner.try_lock_exclusive(path).await
    }
}

/// Whether the file system holding `dir` keeps two names differing only in
/// case apart, counted by listing a folder rather than by asking for a name.
fn keeps_case(dir: &Path) -> std::io::Result<bool> {
    let probe = dir.join("probe");
    std::fs::create_dir(&probe)?;
    std::fs::write(probe.join("a"), "")?;
    std::fs::write(probe.join("A"), "")?;
    let kept = std::fs::read_dir(&probe)?.count() == 2;
    std::fs::remove_dir_all(&probe)?;
    Ok(kept)
}

fn key(registry: Registry, namespace: &Namespace) -> PackageKey {
    PackageKey {
        registry,
        package: namespace.clone(),
    }
}

fn entry_of<'a>(contract: &'a Contract, namespace: &Namespace) -> &'a Entry {
    contract
        .entries()
        .iter()
        .find(|entry| entry.package == *namespace)
        .expect("an entry")
}

/// A bucket package with unpushed commits keeps its base and its commits in
/// the lock, its catalog in the contract, its folder where it was, and its
/// manifests and objects in its bucket's store, the rows that named the old
/// store's objects naming the copies. The old folder is left as it was.
#[test(tokio::test)]
async fn a_package_with_unpushed_commits_is_imported_whole() -> Res {
    let old = Old::new()?;
    let namespace = Namespace::from(("acme", "demo"));
    let hashes = pushed_with_commits(&old, &namespace, "bucket").await?;
    let before = old.snapshot()?;

    let imported = old.import().await?;

    let key = key(Registry::S3("bucket".to_string()), &namespace);
    assert_eq!(
        imported.packages,
        BTreeMap::from([(
            namespace.clone(),
            PackageImport::Locked {
                key: key.clone(),
                base: hashes[0].clone(),
                commits: hashes[1..].to_vec(),
            }
        )])
    );
    assert_eq!(imported.dataspace, old.home());
    let (contract, lock) = old.dataspace().await?;
    let entry = entry_of(&contract, &namespace);
    assert_eq!(entry.access, Access::Sync);
    assert_eq!(entry.catalog, Some(CATALOG.parse()?));
    assert_eq!(entry.paths, Paths::Whole);
    assert_eq!(entry.folder, old.home().join("acme").join("demo"));
    assert_eq!(
        lock.packages,
        BTreeMap::from([(
            key,
            LockEntry {
                base: hashes[0].clone(),
                commits: hashes[1..].to_vec(),
            }
        )])
    );
    assert_eq!(lock.header.filled, None, "the first tick reads it in full");

    let store = old.domain().registry("bucket")?;
    for hash in &hashes {
        let manifest =
            Manifest::from_path(&old.storage, &store.installed_manifest(&namespace, hash)).await?;
        assert_eq!(top_hash(&manifest)?, *hash);
        for row in &manifest.rows {
            let object = url::Url::parse(&row.physical_key)?
                .to_file_path()
                .expect("a file");
            assert!(object.starts_with(store.objects_dir()), "{object:?}");
            assert!(object.is_file());
        }
    }
    assert_eq!(old.snapshot()?, before, "the old folder is left as it was");
    assert_eq!(
        old.domain().read_file(&old.storage).await?,
        Some(DomainFile {
            default_dataspace: Some(old.home()),
            autosync: Vec::new(),
            import: Some(ImportRecord {
                from: old.app(),
                done: true,
            }),
        })
    );
    Ok(())
}

/// A package with no remote goes to the local registry: its oldest revision
/// becomes its first there, and `latest`, and the lock's base, and the rest
/// stay unpushed commits.
#[test(tokio::test)]
async fn a_package_with_no_remote_goes_to_the_local_registry() -> Res {
    let old = Old::new()?;
    let namespace = Namespace::from(("me", "analysis"));
    let first = old
        .commit(&namespace, PackageLineage::default(), &[("a.txt", b"one")])
        .await?;
    let mut lineage = old.commit(&namespace, first, &[("a.txt", b"two")]).await?;
    lineage.sync_scope = SyncScope::EntirePackage;
    let commit = lineage.commit.clone().expect("a commit");
    old.install(&namespace, &lineage).await?;

    let imported = old.import().await?;

    let local_name = old.local_name().await?;
    let key = key(Registry::Local(local_name), &namespace);
    let oldest = commit.prev_hashes[0].clone();
    assert_eq!(
        imported.packages[&namespace],
        PackageImport::Locked {
            key: key.clone(),
            base: oldest.clone(),
            commits: vec![commit.hash.clone()],
        }
    );
    let local = old.domain().local();
    assert_eq!(
        local.read_latest(&old.storage, &namespace).await?,
        Some(oldest.clone())
    );
    assert!(local.revision(&oldest).is_file());
    assert!(local.installed_manifest(&namespace, &commit.hash).is_file());
    let (contract, lock) = old.dataspace().await?;
    assert_eq!(entry_of(&contract, &namespace).registry, key.registry);
    assert_eq!(
        lock.packages[&key],
        LockEntry {
            base: oldest,
            commits: vec![commit.hash],
        }
    );
    Ok(())
}

/// What a package keeps becomes its entry's paths: the files it tracks, or
/// its folders and the files it tracks outside them. One that keeps only the
/// files it tracks, and tracks none, is left out.
#[test(tokio::test)]
async fn what_a_package_keeps_becomes_its_paths() -> Res {
    let old = Old::new()?;
    let [files, folders, none] =
        ["files", "folders", "none"].map(|name| Namespace::from(("acme", name)));
    for (namespace, scope, keys) in [
        (
            &files,
            SyncScope::IndividualFiles,
            &["a.txt", "sub/b.txt"][..],
        ),
        (
            &folders,
            SyncScope::folders(["data"])?,
            &["data/x.csv", "top.txt"][..],
        ),
        (&none, SyncScope::IndividualFiles, &[][..]),
    ] {
        let committed = old
            .commit(namespace, PackageLineage::default(), &[("a.txt", b"a")])
            .await?;
        let base = committed.commit.expect("a commit").hash;
        let lineage = PackageLineage {
            remote_uri: Some(remote("bucket", namespace, &base)),
            base_hash: base.clone(),
            latest_hash: base,
            paths: tracked(keys),
            sync_scope: scope,
            ..PackageLineage::default()
        };
        old.install(namespace, &lineage).await?;
    }

    let imported = old.import().await?;

    let (contract, _) = old.dataspace().await?;
    let file = |key: &str| PackagePath::File(LogicalKey::new(key).unwrap());
    let folder = |key: &str| PackagePath::Folder(LogicalKey::new(key).unwrap());
    assert_eq!(
        entry_of(&contract, &files).paths,
        Paths::Only(vec![file("a.txt"), file("sub/b.txt")])
    );
    assert_eq!(
        entry_of(&contract, &folders).paths,
        Paths::Only(vec![folder("data"), file("top.txt")])
    );
    assert!(contract.entries().iter().all(|entry| entry.package != none));
    assert_eq!(
        imported.packages[&none],
        PackageImport::NothingTracked {
            key: key(Registry::S3("bucket".to_string()), &none)
        }
    );
    Ok(())
}

/// A package with a remote that was never published has no base, so the
/// lock can't hold its commits: it is declared, and its commits are kept in
/// its bucket's store, where a later reattach finds them.
#[test(tokio::test)]
async fn a_package_never_published_is_declared_and_its_commits_kept() -> Res {
    let old = Old::new()?;
    let namespace = Namespace::from(("acme", "new"));
    let mut lineage = old
        .commit(&namespace, PackageLineage::default(), &[("a.txt", b"a")])
        .await?;
    lineage.sync_scope = SyncScope::EntirePackage;
    let commit = lineage.commit.clone().expect("a commit");
    lineage.remote_uri = Some(remote("bucket", &namespace, ""));
    old.install(&namespace, &lineage).await?;

    let imported = old.import().await?;

    let key = key(Registry::S3("bucket".to_string()), &namespace);
    assert_eq!(
        imported.packages[&namespace],
        PackageImport::Unpublished {
            key: key.clone(),
            commits: vec![commit.hash.clone()],
        }
    );
    let (contract, lock) = old.dataspace().await?;
    assert_eq!(entry_of(&contract, &namespace).access, Access::Sync);
    assert!(!lock.packages.contains_key(&key));
    let store = old.domain().registry("bucket")?;
    assert!(store.installed_manifest(&namespace, &commit.hash).is_file());
    Ok(())
}

/// A bucket whose name the store refuses has no store, so its package is
/// left out and the import says why; the others are imported.
#[test(tokio::test)]
async fn a_package_whose_bucket_has_no_store_is_left_out() -> Res {
    let old = Old::new()?;
    let refused = Namespace::from(("acme", "legacy"));
    pushed_with_commits(&old, &refused, "LegacyBucket").await?;
    let kept = Namespace::from(("acme", "kept"));
    pushed_with_commits(&old, &kept, "bucket").await?;

    let imported = old.import().await?;

    assert!(
        matches!(
            &imported.packages[&refused],
            PackageImport::NoStore { bucket, .. } if bucket == "LegacyBucket"
        ),
        "{:?}",
        imported.packages[&refused]
    );
    assert!(matches!(
        imported.packages[&kept],
        PackageImport::Locked { .. }
    ));
    let (contract, _) = old.dataspace().await?;
    assert!(
        contract
            .entries()
            .iter()
            .all(|entry| entry.package != refused)
    );
    assert!(!old.domain().root().join("registries/LegacyBucket").exists());
    Ok(())
}

/// A ref that can't be read costs its package alone, and a package whose
/// name the local registry already holds at another revision is left out
/// rather than published over; the import says why for each.
#[test(tokio::test)]
async fn a_package_that_cant_be_placed_costs_that_package_alone() -> Res {
    let old = Old::new()?;
    let damaged = Namespace::from(("acme", "damaged"));
    let taken = Namespace::from(("me", "taken"));
    let kept = Namespace::from(("acme", "kept"));
    pushed_with_commits(&old, &kept, "bucket").await?;
    let mut lineage = old
        .commit(&taken, PackageLineage::default(), &[("a.txt", b"mine")])
        .await?;
    lineage.sync_scope = SyncScope::EntirePackage;
    old.install(&taken, &lineage).await?;
    let damaged_ref = old.paths.package_ref(&damaged);
    std::fs::create_dir_all(damaged_ref.parent().expect("a parent"))?;
    std::fs::write(&damaged_ref, b"{")?;
    // Another revision of the name, already the local registry's `latest`.
    let local = old.domain().local();
    let other = record(
        &local,
        &old.storage,
        &MockRemote::default(),
        WorkflowCheck::Warn { host: None },
        &taken,
        PackageLineage::default(),
        Manifest {
            rows: vec![ManifestRow {
                logical_key: LogicalKey::new("a.txt")?,
                ..ManifestRow::default()
            }],
            ..Manifest::default()
        },
        &BTreeMap::from([(
            LogicalKey::new("a.txt")?,
            RowContent::Bytes(Bytes::from_static(b"theirs")),
        )]),
    )
    .await?
    .0
    .commit
    .expect("a commit")
    .hash;
    local.publish(&old.storage, &taken, None, &other).await?;

    let imported = old.import().await?;

    assert!(
        matches!(
            imported.packages[&damaged],
            PackageImport::Unreadable { .. }
        ),
        "{:?}",
        imported.packages[&damaged]
    );
    assert!(
        matches!(
            &imported.packages[&taken],
            PackageImport::LocalTaken { latest: Some(latest), .. } if *latest == other
        ),
        "{:?}",
        imported.packages[&taken]
    );
    assert!(matches!(
        imported.packages[&kept],
        PackageImport::Locked { .. }
    ));
    assert_eq!(
        local.read_latest(&old.storage, &taken).await?,
        Some(other),
        "the local registry's own revision stays `latest`"
    );
    let (contract, _) = old.dataspace().await?;
    let declared: Vec<&Namespace> = contract.entries().iter().map(|e| &e.package).collect();
    assert_eq!(declared, [&kept]);
    Ok(())
}

/// A Domain already in use, with a `domain.json` no import wrote, is not
/// imported into.
#[test(tokio::test)]
async fn a_domain_in_use_is_not_imported_into() -> Res {
    let old = Old::new()?;
    pushed_with_commits(&old, &("acme", "demo").into(), "bucket").await?;
    old.domain()
        .set_autosync(&old.storage, &old.dir.path().join("elsewhere"), true)
        .await?;

    let refused = old.import().await;

    assert!(matches!(refused, Err(ImportError::InUse)), "{refused:?}");
    assert!(!old.home().join(CONTRACT_FILE).exists());
    Ok(())
}

/// An import stopped part way, after a package or before the lock, is run
/// again and finishes as one never stopped would.
#[test(tokio::test)]
async fn an_interrupted_import_finishes_when_run_again() -> Res {
    for (point, passes) in [
        ("import.package", 0),
        ("import.package", 1),
        ("import.lock", 0),
    ] {
        let old = Old::new()?;
        let first = Namespace::from(("acme", "first"));
        let second = Namespace::from(("acme", "second"));
        let first_hashes = pushed_with_commits(&old, &first, "bucket").await?;
        pushed_with_commits(&old, &second, "bucket").await?;

        let stopped = crate::failpoint::stopping_at(point, passes, old.import()).await;
        assert!(stopped.is_err(), "{point} {passes}");
        let record = old.domain().read_file(&old.storage).await?;
        assert_eq!(
            record
                .and_then(|file| file.import)
                .map(|record| record.done),
            Some(false),
            "{point} {passes}"
        );

        let imported = old.import().await?;

        assert_eq!(imported.packages.len(), 2);
        let (_, lock) = old.dataspace().await?;
        assert_eq!(
            lock.packages[&key(Registry::S3("bucket".to_string()), &first)],
            LockEntry {
                base: first_hashes[0].clone(),
                commits: first_hashes[1..].to_vec(),
            },
            "{point} {passes}"
        );
        assert_eq!(lock.packages.len(), 2);
    }
    Ok(())
}

/// A home changed after the import stopped is a different dataspace than the
/// one it began, so finishing would write the new one's files.
#[test(tokio::test)]
async fn an_interrupted_import_is_refused_once_the_home_has_changed() -> Res {
    let old = Old::new()?;
    pushed_with_commits(&old, &("acme", "demo").into(), "bucket").await?;
    let stopped = crate::failpoint::stopping_at("import.package", 0, old.import()).await;
    assert!(stopped.is_err());
    let other = old.dir.path().join("other");
    let config = serde_json::json!({"format": 1, "home": other, "migration": "done"});
    std::fs::write(old.paths.domain_config(), serde_json::to_vec(&config)?)?;

    let refused = old.import().await;

    assert!(
        matches!(&refused, Err(ImportError::HomeChanged { was, now })
            if *was == old.home() && *now == other),
        "{refused:?}"
    );
    Ok(())
}

/// A finished import is not run again, so a second cannot overwrite what was
/// done in the Domain since.
#[test(tokio::test)]
async fn a_finished_import_is_not_run_again() -> Res {
    let old = Old::new()?;
    pushed_with_commits(&old, &("acme", "demo").into(), "bucket").await?;
    old.import().await?;

    let again = old.import().await;

    assert!(
        matches!(&again, Err(ImportError::Done(from)) if *from == old.app()),
        "{again:?}"
    );
    Ok(())
}

/// A home that already holds a contract is someone's dataspace, so the
/// import refuses before it writes anything.
#[test(tokio::test)]
async fn a_home_that_is_a_dataspace_already_is_refused() -> Res {
    let old = Old::new()?;
    pushed_with_commits(&old, &("acme", "demo").into(), "bucket").await?;
    std::fs::create_dir_all(old.home())?;
    std::fs::write(old.home().join(CONTRACT_FILE), b"[packages]\n")?;

    let refused = old.import().await;

    assert!(
        matches!(&refused, Err(ImportError::Dataspace(home)) if *home == old.home()),
        "{refused:?}"
    );
    assert!(!old.domain().domain_file().exists());
    Ok(())
}

/// A domain still on `data.json` is not read: only the move to refs, which
/// a build with refs makes on opening it, gives the import its packages.
#[test(tokio::test)]
async fn a_domain_on_data_json_or_none_is_not_imported() -> Res {
    let old = Old::new()?;
    std::fs::remove_file(old.paths.domain_config())?;
    std::fs::write(
        old.paths.lineage(),
        serde_json::to_vec(&serde_json::json!({"home": old.home(), "packages": {}}))?,
    )?;

    let on_data_json = old.import().await;
    assert!(
        matches!(&on_data_json, Err(ImportError::NotOnRefs(_))),
        "{on_data_json:?}"
    );

    std::fs::remove_file(old.paths.lineage())?;
    let none = old.import().await;
    assert!(matches!(&none, Err(ImportError::NoDomain(_))), "{none:?}");
    Ok(())
}

/// The dataspaces the old `queue.db` held on autosync move into
/// `domain.json`, and the queue itself beside the new `state.db`, without
/// them; the old queue is left as it was.
#[test(tokio::test)]
async fn the_autosync_list_moves_from_the_queue_to_domain_json() -> Res {
    let old = Old::new()?;
    let dataspaces = [old.dir.path().join("one"), old.dir.path().join("two")];
    {
        let db = rusqlite::Connection::open(old.paths.queue_db())?;
        db.execute_batch(
            "CREATE TABLE job (registry TEXT NOT NULL, namespace TEXT NOT NULL, folder BLOB NOT NULL, access TEXT NOT NULL, running_in TEXT, again INTEGER NOT NULL DEFAULT 0, UNIQUE (folder, registry, namespace, access));
             CREATE TABLE failure (registry TEXT NOT NULL, namespace TEXT NOT NULL, folder BLOB NOT NULL, access TEXT NOT NULL, failures INTEGER NOT NULL, error TEXT NOT NULL, retry_at INTEGER NOT NULL, PRIMARY KEY (folder, registry, namespace, access)) WITHOUT ROWID;
             CREATE TABLE kept (registry TEXT NOT NULL, namespace TEXT NOT NULL, folder BLOB NOT NULL, hash TEXT NOT NULL, base TEXT, reason TEXT NOT NULL, PRIMARY KEY (folder, registry, namespace)) WITHOUT ROWID;
             CREATE TABLE autosync (dataspace BLOB NOT NULL UNIQUE);
             PRAGMA user_version = 1;",
        )?;
        for dataspace in &dataspaces {
            db.execute(
                "INSERT INTO autosync (dataspace) VALUES (?1)",
                [crate::state_store::os_bytes(dataspace)],
            )?;
        }
    }
    let before = old.snapshot()?;

    let imported = old.import().await?;

    assert_eq!(imported.autosync, dataspaces);
    assert_eq!(old.domain().autosync(&old.storage).await?, dataspaces);
    let queue = crate::queue::Queue::open(&old.domain().queue_db()).await?;
    assert_eq!(queue.jobs().await?, Vec::new());
    assert_eq!(old.snapshot()?, before);
    Ok(())
}

/// Each object is synced before it takes its name, and the folders the
/// copies made before the import records itself done, so a power loss after
/// `done` loses nothing the lock names.
#[test(tokio::test)]
async fn the_copies_are_durable_before_the_import_is_done() -> Res {
    let old = Old::new()?;
    let namespace = Namespace::from(("acme", "demo"));
    pushed_with_commits(&old, &namespace, "bucket").await?;
    // A bucket named for a store's own folder is still a store to sync.
    pushed_with_commits(&old, &Namespace::from(("acme", "other")), "packages").await?;
    let storage = Recording::default();

    import(&storage, &old.app(), &old.domain()).await?;

    let steps = storage.steps();
    let done = steps
        .iter()
        .rposition(|step| *step == Step::Write(old.domain().domain_file()))
        .expect("the import recorded done");
    let store = old.domain().registry("bucket")?;
    let objects = std::fs::read_dir(store.objects_dir())?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()?;
    assert_ne!(objects, Vec::<PathBuf>::new());
    for object in &objects {
        let renamed = steps
            .iter()
            .position(|step| matches!(step, Step::Rename { to, .. } if to == object))
            .expect("copied by way of a temp");
        let Step::Rename { from: temp, .. } = &steps[renamed] else {
            unreachable!("found as a rename");
        };
        assert!(
            steps[..renamed].contains(&Step::SyncFile(temp.clone())),
            "{object:?} is synced before it takes its name"
        );
    }
    for folder in [
        store.objects_dir(),
        store.installed_manifests_dir(&namespace),
        store.dir().to_path_buf(),
        old.domain().registry("packages")?.objects_dir(),
        old.domain().root().to_path_buf(),
    ] {
        assert!(
            steps[..done].contains(&Step::SyncDir(folder.clone())),
            "{folder:?} is synced before the import is done"
        );
    }
    Ok(())
}

/// The queue copied beside `state.db` is synced before the import is done.
#[test(tokio::test)]
async fn the_copied_queue_is_synced_before_the_import_is_done() -> Res {
    let old = Old::new()?;
    rusqlite::Connection::open(old.paths.queue_db())?.execute_batch(
        "CREATE TABLE autosync (dataspace BLOB NOT NULL UNIQUE); PRAGMA user_version = 1;",
    )?;
    let storage = Recording::default();

    import(&storage, &old.app(), &old.domain()).await?;

    let steps = storage.steps();
    let done = steps
        .iter()
        .rposition(|step| *step == Step::Write(old.domain().domain_file()))
        .expect("the import recorded done");
    assert!(
        steps[..done].contains(&Step::SyncFile(old.domain().queue_db())),
        "the queue is synced before the import is done"
    );
    Ok(())
}

/// Two packages whose names differ only in case are one folder on a drive
/// that ignores case, so the import refuses them, saying so, before it
/// copies or records anything. Where case is kept, both are imported.
#[test(tokio::test)]
async fn names_differing_only_in_case_are_refused_where_case_is_ignored() -> Res {
    let old = Old::new()?;
    // Where case is ignored, the old domain can't hold both refs either.
    if !keeps_case(old.dir.path())? {
        return Ok(());
    }
    pushed_with_commits(&old, &("acme", "Demo").into(), "bucket").await?;
    pushed_with_commits(&old, &("acme", "demo").into(), "bucket").await?;

    let refused = import_comparing(
        &old.storage,
        &old.app(),
        &old.domain(),
        Some(FolderCase::Insensitive),
    )
    .await;

    assert!(
        matches!(&refused, Err(ImportError::Contract(err))
            if matches!(**err, ContractError::FoldersOverlap { .. })),
        "{refused:?}"
    );
    assert!(!old.domain().domain_file().exists());
    assert!(!old.domain().root().join("registries/bucket").exists());
    assert!(!old.home().join(CONTRACT_FILE).exists());

    let imported = old.import().await?;
    assert_eq!(imported.packages.len(), 2);
    Ok(())
}

/// A package whose ref keeps changing while it is copied stops the import,
/// naming the package, and leaves it to be run again.
#[test(tokio::test)]
async fn a_package_that_keeps_changing_stops_the_import() -> Res {
    let old = Old::new()?;
    let namespace = Namespace::from(("acme", "demo"));
    pushed_with_commits(&old, &namespace, "bucket").await?;
    let storage = Recording {
        changing: Some(old.paths.package_ref(&namespace)),
        ..Recording::default()
    };

    let stopped = import(&storage, &old.app(), &old.domain()).await;

    assert!(
        matches!(&stopped, Err(ImportError::Changing(name)) if **name == namespace),
        "{stopped:?}"
    );
    let record = old.domain().read_file(&old.storage).await?;
    assert_eq!(
        record
            .and_then(|file| file.import)
            .map(|record| record.done),
        Some(false)
    );
    Ok(())
}

/// A package with no revision, published or committed, is left out, with a
/// bucket or without; one whose entry the contract refuses on its own is left
/// out with the reason, and costs no other package.
#[test(tokio::test)]
async fn a_package_with_no_revision_or_no_entry_is_left_out() -> Res {
    let old = Old::new()?;
    let bucketed = Namespace::from(("acme", "empty"));
    let local = Namespace::from(("me", "empty"));
    let colliding = Namespace::from(("acme", "colliding"));
    let kept = Namespace::from(("acme", "kept"));
    old.install(
        &bucketed,
        &PackageLineage {
            remote_uri: Some(remote("bucket", &bucketed, "")),
            sync_scope: SyncScope::EntirePackage,
            ..PackageLineage::default()
        },
    )
    .await?;
    old.install(
        &local,
        &PackageLineage {
            sync_scope: SyncScope::EntirePackage,
            ..PackageLineage::default()
        },
    )
    .await?;
    let base = old
        .commit(&colliding, PackageLineage::default(), &[("a.txt", b"a")])
        .await?
        .commit
        .expect("a commit")
        .hash;
    // `a.txt` as a file and as a folder.
    old.install(
        &colliding,
        &PackageLineage {
            remote_uri: Some(remote("bucket", &colliding, &base)),
            base_hash: base.clone(),
            latest_hash: base,
            paths: tracked(&["a.txt", "a.txt/b.txt"]),
            sync_scope: SyncScope::IndividualFiles,
            ..PackageLineage::default()
        },
    )
    .await?;
    pushed_with_commits(&old, &kept, "bucket").await?;

    let imported = old.import().await?;

    assert_eq!(imported.packages[&bucketed], PackageImport::NoRevision);
    assert_eq!(imported.packages[&local], PackageImport::NoRevision);
    assert!(
        matches!(
            &imported.packages[&colliding],
            PackageImport::Undeclarable { key, reason }
                if key.package == colliding && reason.contains("both a file and a folder")
        ),
        "{:?}",
        imported.packages[&colliding]
    );
    assert!(matches!(
        imported.packages[&kept],
        PackageImport::Locked { .. }
    ));
    let (contract, _) = old.dataspace().await?;
    let declared: Vec<&Namespace> = contract.entries().iter().map(|e| &e.package).collect();
    assert_eq!(declared, [&kept]);
    Ok(())
}

/// An import stopped part way is finished from the folder it began with:
/// one from another folder is refused, naming the first.
#[test(tokio::test)]
async fn an_unfinished_import_refuses_one_from_another_folder() -> Res {
    let old = Old::new()?;
    pushed_with_commits(&old, &("acme", "demo").into(), "bucket").await?;
    let stopped = crate::failpoint::stopping_at("import.package", 0, old.import()).await;
    assert!(stopped.is_err());
    let other = Old::new()?;
    pushed_with_commits(&other, &("acme", "other").into(), "bucket").await?;

    let refused = import(&other.storage, &other.app(), &old.domain()).await;

    assert!(
        matches!(&refused, Err(ImportError::Unfinished(from)) if *from == old.app()),
        "{refused:?}"
    );
    Ok(())
}
