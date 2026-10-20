use std::time::Duration;
use std::time::SystemTime;

use tempfile::TempDir;
use test_log::test;

use super::*;

use crate::checksum::hash_file;
use crate::file_stamp;
use crate::local_domain::LocalDomain;
use crate::stat_cache::CachedHash;

/// A cache entry that matches the file's stamp but not its content, as a
/// change the stamp could not see would leave: a commit still names the
/// object by what the file holds.
#[test(tokio::test)]
async fn a_commit_hashes_what_it_commits_whatever_the_cache_says() -> Res {
    let temp_dir = TempDir::new()?;
    let store = Arc::new(PackageStateStore::in_memory());
    let domain = LocalDomain::new(temp_dir.path()).with_state_store(Arc::clone(&store));
    domain.set_home(temp_dir.path()).await?;
    let namespace: Namespace = ("demo", "cached").into();
    let (package, _) = domain
        .create_package(namespace.clone(), None, Some("empty".to_string()))
        .await?;

    let path = package.package_home().await?.join("data.txt");
    std::fs::write(&path, b"real content")?;
    std::fs::File::options()
        .write(true)
        .open(&path)?
        .set_modified(SystemTime::now() - Duration::from_secs(3600))?;

    let host_config = HostConfig::default();
    let algorithm = host_config.checksums.algorithm_code();
    let (real, _) = hash_file(&package.storage, &path, algorithm).await?;
    let other = temp_dir.path().join("other.txt");
    std::fs::write(&other, b"other content")?;
    let (lie, _) = hash_file(&package.storage, &other, algorithm).await?;
    let mut cache = store.stat_cache(&namespace).await;
    cache.record(
        std::path::Path::new("data.txt"),
        CachedHash {
            stamp: file_stamp::stamp(&path)?,
            hash: lie,
            saved_at: SystemTime::now(),
        },
    );
    store.save_stat_cache(&namespace, &cache).await;

    let status = package
        .recompute_local_status(Some(host_config.clone()))
        .await?;
    let Some(lineage::Change::Added(row)) = status.changes.get(std::path::Path::new("data.txt"))
    else {
        panic!("{:?}", status.changes);
    };
    assert_ne!(row.hash, real, "the walk took the cached hash");

    package
        .commit("add".to_string(), UserMeta::Keep, None, Some(host_config))
        .await?;
    let manifest = package.manifest().await?;
    let committed = manifest
        .get_record(&PathBuf::from("data.txt"))
        .expect("committed");
    assert_eq!(committed.hash, real);
    Ok(())
}
