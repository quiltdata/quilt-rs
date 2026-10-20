use super::*;

use std::sync::Arc;

use quilt_uri::fixtures;

use crate::autopull::AutosyncSettings;
use crate::autopull::PullSettings;
use crate::autopull::PushSettings;
use crate::autopull::inner_for_tests;
use crate::autopull::reporter::test_support::RecordingReporter;
use crate::model::MockQuiltModel;
use crate::quilt;
use crate::quilt::lineage::UpstreamState;
use crate::quilt::state_store::ChangeSource;

fn ns() -> Namespace {
    ("acme", "demo").into()
}

fn package() -> quilt::InstalledPackage {
    quilt::LocalDomain::new(std::path::PathBuf::new()).create_installed_package(ns())
}

fn status(files: usize) -> quilt::lineage::InstalledPackageStatus {
    let mut changes = quilt::lineage::ChangeSet::new();
    for i in 0..files {
        changes.insert(
            std::path::PathBuf::from(format!("f{i}.csv")),
            quilt::lineage::Change::Added(quilt::manifest::ManifestRow::default()),
        );
    }
    quilt::lineage::InstalledPackageStatus::new(UpstreamState::UpToDate, changes)
}

/// A published package whose walk finds `files` changed files, and which
/// expects `publishes` publishes. `busy` means another writer holds its lock.
fn model(files: usize, publishes: usize, busy: bool) -> MockQuiltModel {
    let mut model = MockQuiltModel::new();
    model
        .expect_get_installed_package()
        .returning(|_| Ok(Some(package())));
    model.expect_get_installed_package_lineage().returning(|_| {
        Ok(quilt::lineage::PackageLineage::from_remote(
            quilt_uri::ManifestUri {
                bucket: "bucket".to_string(),
                namespace: ns(),
                hash: "h0".to_string(),
                origin: Some(fixtures::host()),
            },
            "h0".to_string(),
        ))
    });
    model
        .expect_try_lock_package()
        .returning(move |p| Ok((!busy).then(|| p.namespace.clone())));
    model
        .expect_locked_package_status()
        .returning(move |_, _| Ok(status(files)));
    model.expect_resolve_workflow().returning(|_, _| Ok(None));
    model
        .expect_locked_package_publish()
        .times(publishes)
        .returning(|_, _, _, _, _, _| {
            Ok(quilt::PublishOutcome::CommittedAndPushed(
                quilt::PushOutcome {
                    manifest_uri: quilt_uri::ManifestUri {
                        bucket: "bucket".to_string(),
                        namespace: ns(),
                        hash: "h1".to_string(),
                        origin: None,
                    },
                    certified_latest: true,
                },
            ))
        });
    model
}

/// A wait of `wait` and a check interval three times as long.
fn inner(wait: Duration, pull: bool, push: bool) -> (WatcherInner, Arc<RecordingReporter>) {
    let reporter = Arc::new(RecordingReporter::default());
    let inner = WatcherInner {
        reporter: reporter.clone(),
        ..inner_for_tests(AutosyncSettings {
            pull: PullSettings {
                enabled: pull,
                focused_secs: wait.as_secs() * 3,
                ..PullSettings::default()
            },
            push: PushSettings {
                enabled: push,
                idle_timeout_secs: wait.as_secs(),
            },
            close_to_tray: false,
        })
    };
    (inner, reporter)
}

/// `acme/demo` last changed `ago`, was walked since, and counts down.
async fn changed(inner: WatcherInner, ago: Duration) -> WatcherInner {
    let inner = WatcherInner {
        last_change: crate::autopull::last_change::LastChange::changed_for_test(&ns(), ago).await,
        ..inner
    };
    inner.clocks.publish_waiting.write().await.insert(ns());
    inner
}

/// Move `acme/demo`'s last change to `ago`.
async fn moved(inner: &WatcherInner, ago: Duration) {
    inner
        .last_change
        .store_for_test()
        .record_change(&ns(), SystemTime::now() - ago, ChangeSource::Edit)
        .await;
}

fn published(reporter: &RecordingReporter) -> usize {
    reporter.published.lock().unwrap().len()
}

/// How far `at` is from now, in whole seconds.
fn secs_from_now(at: Option<Instant>) -> u64 {
    at.expect("a deadline")
        .saturating_duration_since(Instant::now())
        .as_secs()
}

#[tokio::test]
async fn a_one_minute_wait_publishes_a_minute_after_the_last_change() {
    let (inner, reporter) = inner(Duration::from_mins(1), true, true);
    let model = model(1, 1, false);
    let mut held = BTreeMap::new();

    let inner = changed(inner, Duration::from_secs(30)).await;
    let next = publish_due(&model, &RoleCache::default(), &inner, &mut held).await;
    assert_eq!(published(&reporter), 0, "not before the deadline");
    let secs = secs_from_now(next);
    assert!(
        (28..=30).contains(&secs),
        "due in about 30 s, not at the check: {secs}"
    );

    moved(&inner, Duration::from_secs(61)).await;
    publish_due(&model, &RoleCache::default(), &inner, &mut held).await;
    assert_eq!(published(&reporter), 1);
    assert_eq!(inner.last_change.get(&ns()).await, None);
    assert!(inner.clocks.publish_waiting.read().await.is_empty());
}

#[tokio::test]
async fn an_edit_before_the_deadline_moves_it() {
    let (inner, reporter) = inner(Duration::from_mins(1), true, true);
    let model = model(1, 0, false);
    let mut held = BTreeMap::new();
    let inner = changed(inner, Duration::from_secs(59)).await;

    inner.last_change.on_signal(&ns()).await;

    let next = publish_due(&model, &RoleCache::default(), &inner, &mut held).await;
    assert_eq!(published(&reporter), 0);
    assert!(secs_from_now(next) >= 58, "a full wait from the edit");
}

#[tokio::test]
async fn a_new_wait_applies_at_once() {
    let (inner, reporter) = inner(Duration::from_mins(5), true, true);
    let model = model(1, 1, false);
    let mut held = BTreeMap::new();
    let inner = changed(inner, Duration::from_mins(2)).await;

    let next = publish_due(&model, &RoleCache::default(), &inner, &mut held).await;
    assert_eq!(published(&reporter), 0);
    assert!(secs_from_now(next) >= 170, "three minutes left of five");

    inner.settings.write().await.push.idle_timeout_secs = 60;
    publish_due(&model, &RoleCache::default(), &inner, &mut held).await;
    assert_eq!(
        published(&reporter),
        1,
        "two minutes ago is past a one-minute wait"
    );
}

#[tokio::test]
async fn publishing_off_cancels_the_timer() {
    let (inner, reporter) = inner(Duration::from_mins(1), true, false);
    let model = model(1, 0, false);
    let inner = changed(inner, Duration::from_mins(2)).await;

    let next = publish_due(&model, &RoleCache::default(), &inner, &mut BTreeMap::new()).await;

    assert_eq!(next, None, "nothing to wake for");
    assert_eq!(published(&reporter), 0);
}

#[tokio::test]
async fn pulling_off_does_not_change_when_it_publishes() {
    let (inner, reporter) = inner(Duration::from_mins(1), false, true);
    let model = model(1, 1, false);
    let inner = changed(inner, Duration::from_secs(61)).await;

    publish_due(&model, &RoleCache::default(), &inner, &mut BTreeMap::new()).await;

    assert_eq!(published(&reporter), 1);
}

#[tokio::test]
async fn a_package_no_longer_changed_at_its_deadline_is_not_published() {
    let (inner, reporter) = inner(Duration::from_mins(1), true, true);
    let model = model(0, 0, false);
    let inner = changed(inner, Duration::from_secs(61)).await;

    let next = publish_due(&model, &RoleCache::default(), &inner, &mut BTreeMap::new()).await;

    assert_eq!(published(&reporter), 0);
    assert_eq!(next, None);
    assert_eq!(
        inner.last_change.get(&ns()).await,
        None,
        "a clean tree has no time"
    );
}

#[tokio::test]
async fn a_package_another_writer_holds_is_tried_again_soon() {
    let (inner, reporter) = inner(Duration::from_mins(1), true, true);
    let model = model(1, 0, false);
    let busy = self::model(1, 0, true);
    let mut held = BTreeMap::new();
    let inner = changed(inner, Duration::from_secs(61)).await;

    let next = publish_due(&busy, &RoleCache::default(), &inner, &mut held).await;
    assert_eq!(published(&reporter), 0);
    assert!(secs_from_now(next) <= BUSY_RETRY.as_secs());

    // Still held: not run again before its retry.
    publish_due(&model, &RoleCache::default(), &inner, &mut held).await;
    assert_eq!(published(&reporter), 0);
}

#[tokio::test]
async fn a_paused_package_waits_for_its_pause_to_clear() {
    let (inner, reporter) = inner(Duration::from_mins(1), true, true);
    let model = model(1, 0, false);
    let inner = changed(inner, Duration::from_secs(61)).await;
    inner
        .paused
        .write()
        .await
        .insert(ns(), crate::autopull::PausedReason::Diverged);

    let next = publish_due(&model, &RoleCache::default(), &inner, &mut BTreeMap::new()).await;

    assert_eq!(next, None);
    assert_eq!(published(&reporter), 0);
}

#[tokio::test]
async fn a_package_with_no_remote_stops_counting_down() {
    let (inner, _) = inner(Duration::from_mins(1), true, true);
    let mut model = MockQuiltModel::new();
    model
        .expect_get_installed_package()
        .returning(|_| Ok(Some(package())));
    model
        .expect_get_installed_package_lineage()
        .returning(|_| Ok(quilt::lineage::PackageLineage::default()));
    let inner = changed(inner, Duration::from_secs(61)).await;

    let next = publish_due(&model, &RoleCache::default(), &inner, &mut BTreeMap::new()).await;

    assert_eq!(next, None);
    assert!(inner.clocks.publish_waiting.read().await.is_empty());
}

#[tokio::test]
async fn a_reschedule_wakes_the_sleeping_timer() {
    let (inner, _) = inner(Duration::from_mins(1), true, true);
    let woken = inner.clocks.publish_timer.notified();
    inner.reschedule_publish();
    tokio::time::timeout(Duration::from_secs(1), woken)
        .await
        .expect("woken");
}
