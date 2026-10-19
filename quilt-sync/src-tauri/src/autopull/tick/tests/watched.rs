//! The tick's walk of a package the folder watcher vouches for.

use super::*;
use quilt_uri::fixtures;

fn up_to_date_lineage() -> quilt::lineage::PackageLineage {
    let remote = quilt_uri::ManifestUri {
        bucket: "bucket".to_string(),
        namespace: ("acme", "demo").into(),
        hash: "h0".to_string(),
        origin: Some(fixtures::host()),
    };
    quilt::lineage::PackageLineage::from_remote(remote, "h0".to_string())
}

fn clean() -> quilt::lineage::InstalledPackageStatus {
    quilt::lineage::InstalledPackageStatus::new(UpstreamState::UpToDate, BTreeMap::new())
}

/// A model with one clean, up-to-date package, expecting `walks` walks and
/// `reuses` statuses from a kept walk, which match its lineage if
/// `lineage_kept`.
fn model(walks: usize, reuses: usize, lineage_kept: bool) -> MockQuiltModel {
    let lineage = up_to_date_lineage();
    let mut model = MockQuiltModel::new();
    model.expect_get_installed_packages_list().returning(|| {
        Ok(vec![
            quilt::LocalDomain::new(std::path::PathBuf::new())
                .create_installed_package(("acme", "demo").into()),
        ])
    });
    model
        .expect_get_installed_package_lineage()
        .returning(move |_| Ok(lineage.clone()));
    model
        .expect_try_lock_package()
        .returning(|p| Ok(Some(p.namespace.clone())));
    model.expect_get_installed_package().returning(|_| {
        Ok(Some(
            quilt::LocalDomain::new(std::path::PathBuf::new())
                .create_installed_package(("acme", "demo").into()),
        ))
    });
    model
        .expect_locked_package_status()
        .times(walks)
        .returning(|_, _| Ok(clean()));
    model
        .expect_locked_package_status_from_walk()
        .times(reuses)
        .returning(move |_, _, walked| Ok(lineage_kept.then_some(walked)));
    model
}

fn inner(reporter: Arc<RecordingReporter>) -> WatcherInner {
    WatcherInner {
        reporter,
        ..crate::autopull::inner_for_tests(enabled())
    }
}

fn ns() -> Namespace {
    ("acme", "demo").into()
}

/// Watched since before a walk that found the package clean.
fn vouched_and_walked(inner: &WatcherInner) {
    inner
        .walks
        .vouch_for(&BTreeMap::from([(ns(), std::path::PathBuf::from("/pkg"))]));
    let start = inner.walks.begin(&ns());
    inner
        .walks
        .record(&ns(), start, up_to_date_lineage(), &clean());
}

#[tokio::test]
async fn a_check_over_an_unchanged_watched_package_does_no_local_walk() -> Result<(), Error> {
    let model = model(0, 2, true);
    let reporter = Arc::new(RecordingReporter::default());
    let inner = inner(reporter.clone());
    vouched_and_walked(&inner);

    run_once(&model, &RoleCache::default(), &inner).await?;
    run_once(&model, &RoleCache::default(), &inner).await?;

    let statuses = reporter.statuses.lock().unwrap();
    assert_eq!(statuses.len(), 2, "each check still reports the package");
    assert_eq!(statuses[1].1.status, "up_to_date");
    Ok(())
}

#[tokio::test]
async fn a_signal_since_the_walk_makes_the_check_walk() -> Result<(), Error> {
    let model = model(1, 1, true);
    let inner = inner(Arc::new(RecordingReporter::default()));
    vouched_and_walked(&inner);
    inner.walks.on_signal(&ns());

    run_once(&model, &RoleCache::default(), &inner).await?;
    // That walk is kept: the next check reuses it.
    run_once(&model, &RoleCache::default(), &inner).await?;
    Ok(())
}

/// The watcher off, its subscribe failed or the watch lost: no package is
/// vouched for, and every check walks, as before.
#[tokio::test]
async fn a_package_the_watcher_cannot_vouch_for_is_walked_by_every_check() -> Result<(), Error> {
    let model = model(2, 0, true);
    let inner = inner(Arc::new(RecordingReporter::default()));

    run_once(&model, &RoleCache::default(), &inner).await?;
    run_once(&model, &RoleCache::default(), &inner).await?;
    Ok(())
}

#[tokio::test]
async fn a_lineage_changed_since_the_walk_makes_the_check_walk() -> Result<(), Error> {
    let model = model(1, 1, false);
    let inner = inner(Arc::new(RecordingReporter::default()));
    vouched_and_walked(&inner);

    run_once(&model, &RoleCache::default(), &inner).await?;
    Ok(())
}
