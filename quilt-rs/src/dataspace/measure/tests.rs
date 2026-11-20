use super::*;

use test_log::test;

use crate::io::storage::LocalStorage;

type Res = Result<(), Box<dyn std::error::Error>>;

/// Writes `len` bytes at `path`, making its folder.
fn file(path: &std::path::Path, len: usize) -> std::io::Result<()> {
    std::fs::create_dir_all(path.parent().expect("a parent"))?;
    std::fs::write(path, vec![b'x'; len])
}

/// Each bucket's store and the local registry are measured on their own,
/// and the total is every byte under the Domain, the files beside the
/// stores included.
#[test(tokio::test)]
async fn the_domain_is_measured_per_store_and_in_total() -> Res {
    let dir = tempfile::tempdir()?;
    let domain = DomainRoot::new(dir.path());
    let one = domain.registry("one")?;
    let two = domain.registry("two")?;
    file(&one.dir().join("objects/a"), 100)?;
    file(&one.cached_manifest("abc"), 20)?;
    file(&two.dir().join("objects/b"), 7)?;
    file(&domain.local().dir().join("objects/c"), 3)?;
    file(&domain.queue_db(), 50)?;

    let size = measure_storage(&LocalStorage::new(), &domain).await?;

    assert_eq!(
        size.registries,
        BTreeMap::from([("one".to_string(), 120), ("two".to_string(), 7)])
    );
    assert_eq!(size.local_bytes, 3);
    assert_eq!(size.total_bytes, 180);
    Ok(())
}

/// A Domain not made yet holds nothing.
#[test(tokio::test)]
async fn a_domain_not_made_holds_nothing() -> Res {
    let dir = tempfile::tempdir()?;
    let domain = DomainRoot::new(dir.path().join("none"));

    let size = measure_storage(&LocalStorage::new(), &domain).await?;

    assert_eq!(size, DomainSize::default());
    Ok(())
}
