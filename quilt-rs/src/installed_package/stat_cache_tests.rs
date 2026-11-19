use std::time::Duration;
use std::time::SystemTime;

use tempfile::TempDir;
use test_log::test;

use super::*;

use crate::checksum::hash_file;
use crate::file_stamp;
use crate::local_domain::LocalDomain;
use crate::object_hash::ObjectHash;
use crate::stat_cache::CachedHash;

/// `demo/cached` holding a new `data.txt`, edited an hour ago so its stamp
/// is not racy, and the hashes of what it holds and of other content.
struct Cached {
    temp_dir: TempDir,
    store: Arc<PackageStateStore>,
    package: InstalledPackage,
    path: PathBuf,
    real: ObjectHash,
    lie: ObjectHash,
}

impl Cached {
    async fn new() -> Res<Self> {
        let temp_dir = TempDir::new()?;
        let store = Arc::new(PackageStateStore::in_memory());
        let domain = LocalDomain::new(temp_dir.path()).with_state_store(Arc::clone(&store));
        domain.set_home(temp_dir.path()).await?;
        let (package, _) = domain
            .create_package(
                ("demo", "cached").into(),
                None,
                Some("empty".to_string()),
                UserMeta::Clear,
            )
            .await?;

        let path = package.package_home().await?.join("data.txt");
        std::fs::write(&path, b"real content")?;
        std::fs::File::options()
            .write(true)
            .open(&path)?
            .set_modified(SystemTime::now() - Duration::from_secs(3600))?;
        let algorithm = HostConfig::default().checksums.algorithm_code();
        let (real, _) = hash_file(&package.storage, &path, algorithm).await?;
        let other = temp_dir.path().join("other.txt");
        std::fs::write(&other, b"other content")?;
        let (lie, _) = hash_file(&package.storage, &other, algorithm).await?;
        Ok(Self {
            temp_dir,
            store,
            package,
            path,
            real,
            lie,
        })
    }

    /// Saves under `key` an entry that matches `data.txt`'s stamp and holds
    /// the other content's hash, as a change the stamp could not see would.
    async fn plant_lie(&self, key: &FolderKey) -> Res {
        let mut cache = self.store.stat_cache(key).await;
        cache.record(
            Path::new("data.txt"),
            CachedHash {
                stamp: file_stamp::stamp(&self.path)?,
                hash: self.lie.clone(),
                saved_at: SystemTime::now(),
            },
        );
        self.store.save_stat_cache(key, &cache).await;
        Ok(())
    }

    /// The hash a walk gives `data.txt`.
    async fn walked_hash(&self) -> Res<ObjectHash> {
        let status = self
            .package
            .recompute_local_status(Some(HostConfig::default()))
            .await?;
        let Some(lineage::Change::Added(row)) = status.changes.get(Path::new("data.txt")) else {
            panic!("{:?}", status.changes);
        };
        Ok(row.hash.clone())
    }
}

/// A commit still names the object by what the file holds.
#[test(tokio::test)]
async fn a_commit_hashes_what_it_commits_whatever_the_cache_says() -> Res {
    let cached = Cached::new().await?;
    cached
        .plant_lie(&cached.package.folder_key().await?)
        .await?;
    assert_ne!(
        cached.walked_hash().await?,
        cached.real,
        "the walk took the cached hash"
    );

    cached
        .package
        .commit(
            "add".to_string(),
            UserMeta::Keep,
            None,
            Some(HostConfig::default()),
        )
        .await?;
    let manifest = cached.package.manifest().await?;
    let committed = manifest
        .get_record(&PathBuf::from("data.txt"))
        .expect("committed");
    assert_eq!(committed.hash, cached.real);
    Ok(())
}

/// A walk reads the stat cache kept under its package's bucket and working
/// folder.
#[test(tokio::test)]
async fn a_walk_reads_the_stat_cache_of_its_bucket_and_folder() -> Res {
    let cached = Cached::new().await?;
    let namespace: Namespace = ("demo", "cached").into();
    cached
        .package
        .lineage
        .edit(&cached.package.storage, |lineage| {
            lineage.remote_uri = Some(ManifestUri {
                bucket: "bucket-a".to_string(),
                namespace: namespace.clone(),
                hash: String::new(),
                origin: None,
            });
        })
        .await?;
    cached
        .plant_lie(&FolderKey {
            registry: Some(Registry::S3("bucket-a".to_string())),
            package: namespace,
            folder: cached.temp_dir.path().join("demo/cached"),
        })
        .await?;
    assert_eq!(cached.walked_hash().await?, cached.lie);
    Ok(())
}
