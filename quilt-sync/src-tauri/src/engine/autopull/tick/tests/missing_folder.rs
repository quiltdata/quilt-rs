//! A package whose working folder is missing: skipped quietly every tick, and
//! synced again once the folder is back.

use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

use super::*;

const HOME: &str = "/Volumes/gone/demo";

/// `upstream`, read from a folder that is there when `present` says so.
fn status_of(upstream: UpstreamState, present: bool) -> quilt::lineage::InstalledPackageStatus {
    let mut status = quilt::lineage::InstalledPackageStatus::new(upstream, BTreeMap::new());
    if !present {
        status.working_copy = quilt::lineage::WorkingCopy::Unavailable(HOME.into());
    }
    status
}

/// One package with `lineage`, whose status reads `upstream` from a folder
/// that is there while `present` is set. Expects nothing else: a pull, a
/// publish or a merge on it fails the test unless the caller expects it.
fn model_with(
    lineage: quilt::lineage::PackageLineage,
    upstream: UpstreamState,
    present: Arc<AtomicBool>,
) -> MockQuiltModel {
    let mut model = MockQuiltModel::new();
    model.expect_get_installed_packages_list().returning(|| {
        Ok(vec![
            crate::engine::model::mocks::nowhere()
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
            crate::engine::model::mocks::nowhere()
                .create_installed_package(("acme", "demo").into()),
        ))
    });
    model
        .expect_locked_package_status()
        .returning(move |_, _| Ok(status_of(upstream, present.load(Ordering::SeqCst))));
    model
}

fn inner(reporter: Arc<RecordingReporter>) -> WatcherInner {
    WatcherInner {
        reporter,
        ..make_inner(enabled())
    }
}

fn reported(reporter: &RecordingReporter) -> Vec<String> {
    reporter
        .statuses
        .lock()
        .unwrap()
        .iter()
        .map(|(_, event)| event.status.clone())
        .collect()
}

/// While the folder is missing, a `Behind` package is neither pulled, paused
/// nor backed off, and it is announced once however many ticks find it
/// missing. The tick after the folder is back pulls it.
#[tokio::test]
async fn a_missing_folder_is_skipped_each_tick_and_pulled_once_it_is_back() -> Result<(), Error> {
    let ns: Namespace = ("acme", "demo").into();
    let present = Arc::new(AtomicBool::new(false));
    let mut model = model_with(
        behind_clean_lineage(),
        UpstreamState::Behind,
        present.clone(),
    );
    model
        .expect_locked_package_pull_verdict()
        .times(1)
        .returning(|_| Ok(PullOutcome::CleanUpdate));
    model
        .expect_locked_package_pull()
        .times(1)
        .returning(|_, _, _| Ok(quilt::flow::Pulled::Applied(applied())));
    let reporter = Arc::new(RecordingReporter::default());
    let inner = inner(reporter.clone());

    for _ in 0..3 {
        run_once(&model, &RoleCache::default(), &inner).await?;
        assert!(inner.paused.read().await.is_empty(), "nothing to clear");
        assert!(inner.backoff.read().await.is_empty(), "nothing to retry");
    }
    assert_eq!(
        reported(&reporter),
        ["unavailable"],
        "announced when it went, not every tick"
    );
    assert_eq!(inner.action_needed.get().len(), 0, "nothing needs action");

    present.store(true, Ordering::SeqCst);
    run_once(&model, &RoleCache::default(), &inner).await?;
    assert_eq!(reported(&reporter), ["unavailable", "up_to_date"]);
    assert_eq!(
        inner.aggregator.note_available(&ns),
        None,
        "the pass that read the folder ended the episode"
    );
    Ok(())
}

/// A commit waiting to be published stays waiting while its folder is
/// missing: the publish acts on what the folder holds.
#[tokio::test]
async fn a_pending_commit_is_not_published_while_its_folder_is_missing() -> Result<(), Error> {
    let remote = quilt_uri::ManifestUri {
        bucket: "bucket".to_string(),
        namespace: ("acme", "demo").into(),
        hash: "h0".to_string(),
        origin: Some(fixtures::host()),
    };
    let lineage = quilt::lineage::PackageLineage {
        commit: Some(quilt::lineage::CommitState {
            hash: "h_local".to_string(),
            ..quilt::lineage::CommitState::default()
        }),
        ..quilt::lineage::PackageLineage::from_remote(remote, "h0".to_string())
    };
    let model = model_with(
        lineage,
        UpstreamState::Ahead,
        Arc::new(AtomicBool::new(false)),
    );
    let reporter = Arc::new(RecordingReporter::default());
    let inner = inner(reporter.clone());

    run_once(&model, &RoleCache::default(), &inner).await?;
    run_once(&model, &RoleCache::default(), &inner).await?;

    assert_eq!(reported(&reporter), ["unavailable"]);
    assert_eq!(*reporter.published.lock().unwrap(), []);
    assert!(inner.backoff.read().await.is_empty());
    assert!(inner.paused.read().await.is_empty());
    Ok(())
}

/// A folder that goes between the status read and the pull is skipped the
/// same way, not retried on backoff.
#[tokio::test]
async fn a_folder_that_goes_before_the_pull_is_skipped_not_backed_off() -> Result<(), Error> {
    let mut model = model_with(
        behind_clean_lineage(),
        UpstreamState::Behind,
        Arc::new(AtomicBool::new(true)),
    );
    model.expect_locked_package_pull_verdict().returning(|_| {
        Err(Error::from(quilt::Error::PackageOp(
            quilt::PackageOpError::WorkingCopyUnavailable(HOME.into()),
        )))
    });
    let reporter = Arc::new(RecordingReporter::default());
    let inner = inner(reporter.clone());

    run_once(&model, &RoleCache::default(), &inner).await?;

    assert_eq!(reported(&reporter), ["unavailable"]);
    assert!(inner.backoff.read().await.is_empty());
    assert!(inner.paused.read().await.is_empty());
    Ok(())
}

/// A backoff left from before the folder went must not hold back the read
/// that finds it back.
#[tokio::test]
async fn a_missing_folder_clears_an_old_backoff() -> Result<(), Error> {
    let ns: Namespace = ("acme", "demo").into();
    let model = model_with(
        behind_clean_lineage(),
        UpstreamState::Behind,
        Arc::new(AtomicBool::new(false)),
    );
    let inner = inner(Arc::new(RecordingReporter::default()));
    // Due now, so the pass runs, and would wait 2 s after another failure.
    inner.backoff.write().await.insert(
        ns.clone(),
        BackoffState {
            next_attempt: Instant::now(),
            consecutive_failures: 3,
            error: "Could not reach S3".to_string(),
        },
    );

    run_once(&model, &RoleCache::default(), &inner).await?;

    assert!(!inner.backoff.read().await.contains_key(&ns));
    Ok(())
}

/// The file watcher's walk of a missing folder lists no change, as a clean
/// tree's does: it must keep the package's last-change time and its
/// countdown, with or without a commit waiting, or the queued publish loses
/// its place in the wait.
#[tokio::test]
async fn a_watcher_walk_of_a_missing_folder_keeps_the_last_change_and_countdown() {
    use crate::engine::autopull::last_change::PendingCommit;

    let ns: Namespace = ("acme", "demo").into();
    let commit_at = SystemTime::now() - Duration::from_hours(2);
    for pending in [PendingCommit::None, PendingCommit::At(commit_at)] {
        let inner = WatcherInner {
            last_change: LastChange::changed_for_test(&ns, Duration::from_hours(1)).await,
            ..make_inner(enabled())
        };
        // A walk with the change in it starts the countdown.
        let mut changed = status_of(UpstreamState::UpToDate, true);
        changed.changes.insert(
            std::path::PathBuf::from("a.csv"),
            quilt::lineage::Change::Added(quilt::manifest::ManifestRow::default()),
        );
        crate::engine::autopull::observe_walk(&inner, &ns, None, &changed, pending).await;
        let before = inner.last_change.get(&ns).await.expect("a last change");
        assert!(inner.clocks.publish_waiting.read().await.contains(&ns));

        crate::engine::autopull::observe_walk(
            &inner,
            &ns,
            None,
            &status_of(UpstreamState::UpToDate, false),
            pending,
        )
        .await;

        assert_eq!(
            inner.last_change.get(&ns).await,
            Some(before),
            "{pending:?}: neither forgotten nor restarted"
        );
        assert!(
            inner.clocks.publish_waiting.read().await.contains(&ns),
            "{pending:?}: still counting down"
        );
    }
}

/// A run of failing publishes from before the folder went is over: the
/// outage is not a failed publish, so the first failure after the folder is
/// back starts a new run rather than needing action at once.
#[tokio::test]
async fn a_missing_folder_ends_a_run_of_failing_publishes() -> Result<(), Error> {
    let ns: Namespace = ("acme", "demo").into();
    let model = model_with(
        behind_clean_lineage(),
        UpstreamState::Behind,
        Arc::new(AtomicBool::new(false)),
    );
    let inner = inner(Arc::new(RecordingReporter::default()));
    let long_ago = Instant::now()
        .checked_sub(PUBLISH_FAILING_AFTER * 2)
        .expect("the clock is past it");
    inner.action_needed.publish_failed(&ns, long_ago);

    run_once(&model, &RoleCache::default(), &inner).await?;

    assert_eq!(
        inner.action_needed.publish_failed(&ns, Instant::now()),
        Duration::ZERO,
        "the next failure starts a new run"
    );
    Ok(())
}
