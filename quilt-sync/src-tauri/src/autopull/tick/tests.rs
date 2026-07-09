use super::*;

use std::sync::Arc;

use tokio::sync::RwLock;

use crate::autopull::AutosyncSettings;
use crate::autopull::PullSettings;
use crate::autopull::PushSettings;
use crate::autopull::WindowMode;
use crate::autopull::reporter::LogReporter;
use crate::autopull::reporter::test_support::RecordingReporter;
use crate::model::MockQuiltModel;
use crate::quilt::lineage::UpstreamState;

mod publish;

fn test_aggregator() -> Arc<crate::autopull::status::SyncTrayAggregator> {
    let (tx, _) = tokio::sync::watch::channel(crate::autopull::status::SyncTrayStatus::default());
    Arc::new(crate::autopull::status::SyncTrayAggregator::new(tx))
}

fn make_inner(settings: AutosyncSettings) -> WatcherInner {
    WatcherInner {
        settings: Arc::new(RwLock::new(settings)),
        window_mode: Arc::new(RwLock::new(WindowMode::Focused)),
        publish_settings: Arc::new(RwLock::new(PublishSettings::default())),
        paused: RwLock::new(BTreeMap::new()),
        backoff: RwLock::new(BTreeMap::new()),
        login_blocked: RwLock::new(BTreeMap::new()),
        reporter: Arc::new(LogReporter),
        aggregator: test_aggregator(),
    }
}

fn enabled() -> AutosyncSettings {
    AutosyncSettings {
        pull: PullSettings {
            enabled: true,
            ..PullSettings::default()
        },
        push: PushSettings {
            enabled: true,
            ..PushSettings::default()
        },
        close_to_tray: false,
    }
}

#[test]
fn classify_sync_pending_changes() {
    let err = Error::from(quilt::Error::PackageOp(quilt::PackageOpError::Package(
        "package has pending changes".to_string(),
    )));
    match classify_sync_err(err) {
        Err(WatchError::Conflict(PausedReason::PendingChanges)) => {}
        other => panic!("expected Conflict(PendingChanges), got {other:?}"),
    }
}

#[test]
fn classify_sync_pending_commits() {
    let err = Error::from(quilt::Error::PackageOp(quilt::PackageOpError::Package(
        "package has pending commits".to_string(),
    )));
    match classify_sync_err(err) {
        Err(WatchError::Conflict(PausedReason::PendingCommit)) => {}
        other => panic!("expected Conflict(PendingCommit), got {other:?}"),
    }
}

#[test]
fn classify_sync_diverged() {
    let err = Error::from(quilt::Error::PackageOp(quilt::PackageOpError::Package(
        "package has diverged".to_string(),
    )));
    match classify_sync_err(err) {
        Err(WatchError::Conflict(PausedReason::Diverged)) => {}
        other => panic!("expected Conflict(Diverged), got {other:?}"),
    }
}

#[test]
fn classify_sync_already_up_to_date_is_ok() {
    let err = Error::from(quilt::Error::PackageOp(quilt::PackageOpError::Package(
        "package is already up-to-date".to_string(),
    )));
    assert!(classify_sync_err(err).is_ok());
}

#[test]
fn classify_sync_login_required() {
    let host: Host = "catalog.dev".parse().unwrap();
    let err = Error::from(quilt::Error::Login(quilt::LoginError::Required(Some(
        host.clone(),
    ))));
    match classify_sync_err(err) {
        Err(WatchError::LoginRequired(Some(h))) => assert_eq!(h, host),
        other => panic!("expected LoginRequired(Some(_)), got {other:?}"),
    }
}

#[test]
fn classify_generic_is_paused() {
    // Unknown `PackageOpError::Package` text → Other(_).
    let err = Error::from(quilt::Error::PackageOp(quilt::PackageOpError::Package(
        "no rule matches this".to_string(),
    )));
    match classify_sync_err(err) {
        Err(WatchError::Conflict(PausedReason::Other(msg))) => {
            assert_eq!(msg, "no rule matches this");
        }
        other => panic!("expected Conflict(Other(_)), got {other:?}"),
    }

    // Bare `Error::General(_)` (no `Quilt(_)` wrapper) → also Other(_).
    let err = Error::General("network".to_string());
    match classify_sync_err(err) {
        Err(WatchError::Conflict(PausedReason::Other(_))) => {}
        other => panic!("expected Conflict(Other(_)), got {other:?}"),
    }
}

#[test]
fn classify_config_format_error_is_conflict() {
    // A malformed `.quilt/workflows/config.yml` surfaces as
    // `RemoteCatalogError::InvalidWorkflowsConfig` (config-schema rejection). It
    // is a user-actionable misconfiguration, so it must pause the namespace
    // (Conflict), not retry as a transient. The dedicated arm binds the inner
    // error so the reason text drops the outer "Quilt error:" wrapper.
    let err = Error::from(quilt::Error::RemoteCatalog(
        quilt::RemoteCatalogError::InvalidWorkflowsConfig(
            "workflows/config.yml does not satisfy the workflows config schema".to_string(),
        ),
    ));
    match classify_sync_err(err) {
        Err(WatchError::Conflict(PausedReason::Other(msg))) => {
            assert!(
                msg.contains("does not satisfy the workflows config schema"),
                "reason text should carry the config-schema message, got: {msg}"
            );
            assert!(
                !msg.contains("Quilt error:"),
                "the dedicated arm must strip the outer wrapper, got: {msg}"
            );
        }
        other => panic!("expected Conflict(Other(_)), got {other:?}"),
    }
}

#[test]
fn classify_push_error_is_paused() {
    let err = Error::from(quilt::Error::PackageOp(quilt::PackageOpError::Push(
        "workflow rejected metadata".to_string(),
    )));
    match classify_sync_err(err) {
        Err(WatchError::Conflict(PausedReason::Other(msg))) => {
            assert_eq!(msg, "workflow rejected metadata");
        }
        other => panic!("expected Conflict(Other(_)), got {other:?}"),
    }
}

/// A workflow-rejection error as it arrives from the quilt-rs commit/push
/// flow: `crate::Error::Quilt(quilt::Error::WorkflowValidation(Rejected(..)))`.
/// The single `MessageRequired` violation gives a deterministic Display we
/// can assert on without pinning a whole schema payload.
pub(super) fn workflow_rejection() -> Error {
    Error::from(quilt::Error::from(
        quilt::workflow::WorkflowValidationError::Rejected(vec![
            quilt::workflow::RuleViolation::MessageRequired,
        ]),
    ))
}

#[test]
fn classify_workflow_validation_is_conflict_with_clean_message() {
    match classify_sync_err(workflow_rejection()) {
        Err(WatchError::Conflict(PausedReason::Other(msg))) => {
            // The tray/tooltip should show the validator's own message that
            // names the failed rule, not the outer `Quilt error:` wrapper
            // prefix that `Error::Quilt`'s Display adds.
            assert!(
                !msg.starts_with("Quilt error:"),
                "reason text should drop the wrapper prefix, got: {msg}"
            );
            assert!(
                msg.starts_with("package does not satisfy the workflow"),
                "reason text should lead with the validator message, got: {msg}"
            );
            assert!(
                msg.contains("a commit message is required"),
                "reason text should name the failed rule, got: {msg}"
            );
        }
        other => panic!("expected Conflict(Other(_)), got {other:?}"),
    }
}

#[test]
fn classify_io_is_transient() {
    let err = Error::from(quilt::Error::Io(std::io::Error::new(
        std::io::ErrorKind::ConnectionRefused,
        "connection refused",
    )));
    match classify_sync_err(err) {
        Err(WatchError::Transient(_)) => {}
        other => panic!("expected Transient(_), got {other:?}"),
    }
}

#[test]
fn classify_s3_is_transient() {
    // Greptile P1 regression: `quilt::Error::S3(_)` is a peer
    // variant of `PackageOp` on `quilt::Error`, *not* nested
    // inside it. Every autopush attempt runs S3 ops (`PutObject`,
    // `UploadFile`, `ListObjects`, …), and a network blip /
    // throttling / 5xx must back off rather than permanently
    // pause the namespace — pausing on a single transient S3
    // hiccup would silently break autopush at the rate AWS hiccups
    // in real workloads.
    let err = Error::from(quilt::Error::S3(quilt::S3Error::new(
        quilt::S3ErrorKind::PutObject("connection reset by peer".to_string()),
    )));
    match classify_sync_err(err) {
        Err(WatchError::Transient(_)) => {}
        other => panic!("expected Transient(_), got {other:?}"),
    }
}

#[test]
fn backoff_curve() {
    // 1st failure → 2 s, then 4, 8, 16, 32, 64, then capped at 64 s.
    assert_eq!(backoff_duration(1), Duration::from_secs(2));
    assert_eq!(backoff_duration(2), Duration::from_secs(4));
    assert_eq!(backoff_duration(3), Duration::from_secs(8));
    assert_eq!(backoff_duration(4), Duration::from_secs(16));
    assert_eq!(backoff_duration(5), Duration::from_secs(32));
    assert_eq!(backoff_duration(6), Duration::from_secs(64));
    assert_eq!(backoff_duration(7), Duration::from_secs(64));
    assert_eq!(backoff_duration(99), Duration::from_secs(64));
}

#[tokio::test]
async fn run_once_disabled_is_a_noop() -> Result<(), Error> {
    let model = MockQuiltModel::new();
    let inner = make_inner(AutosyncSettings::default());
    run_once(&model, &inner).await?;
    Ok(())
}

#[tokio::test]
async fn run_once_behind_and_clean_pulls_and_emits_up_to_date() -> Result<(), Error> {
    let ns: Namespace = ("acme", "demo").into();
    let host: Host = "catalog.dev".parse().unwrap();
    let remote = quilt_uri::ManifestUri {
        bucket: "bucket".to_string(),
        namespace: ns.clone(),
        hash: "h0".to_string(),
        origin: Some(host),
    };
    let lineage = quilt::lineage::PackageLineage::from_remote(remote, "h1".to_string());

    let mut model = MockQuiltModel::new();
    let lineage_for_list = lineage.clone();
    model
        .expect_get_installed_packages_list()
        .returning(move || {
            Ok(vec![
                quilt::LocalDomain::new(std::path::PathBuf::new())
                    .create_installed_package(("acme", "demo").into())
                    .unwrap(),
            ])
        });
    model
        .expect_get_installed_package_lineage()
        .returning(move |_| Ok(lineage_for_list.clone()));
    model.expect_get_installed_package().returning(|_| {
        Ok(Some(
            quilt::LocalDomain::new(std::path::PathBuf::new())
                .create_installed_package(("acme", "demo").into())
                .unwrap(),
        ))
    });
    model
        .expect_get_installed_package_status()
        .returning(|_, _| {
            Ok(quilt::lineage::InstalledPackageStatus::new(
                UpstreamState::Behind,
                BTreeMap::new(),
            ))
        });
    model.expect_package_pull().times(1).returning(|_, _| {
        Ok(quilt_uri::ManifestUri {
            bucket: "bucket".to_string(),
            namespace: ("acme", "demo").into(),
            hash: "h1".to_string(),
            origin: None,
        })
    });

    let reporter = Arc::new(RecordingReporter::default());
    let inner = WatcherInner {
        settings: Arc::new(RwLock::new(enabled())),
        window_mode: Arc::new(RwLock::new(WindowMode::Focused)),
        publish_settings: Arc::new(RwLock::new(PublishSettings::default())),
        paused: RwLock::new(BTreeMap::new()),
        backoff: RwLock::new(BTreeMap::new()),
        login_blocked: RwLock::new(BTreeMap::new()),
        reporter: reporter.clone(),
        aggregator: test_aggregator(),
    };

    run_once(&model, &inner).await?;

    {
        let statuses = reporter.statuses.lock().unwrap();
        assert_eq!(statuses.len(), 1, "expected one status emit");
        assert_eq!(statuses[0].0, ns);
        assert_eq!(statuses[0].1.status, "up_to_date");
        assert!(!statuses[0].1.has_changes);
    }
    assert!(inner.paused.read().await.is_empty());
    Ok(())
}

#[tokio::test]
async fn run_once_pending_changes_pauses_namespace() -> Result<(), Error> {
    let ns: Namespace = ("acme", "demo").into();
    let host: Host = "catalog.dev".parse().unwrap();
    let remote = quilt_uri::ManifestUri {
        bucket: "bucket".to_string(),
        namespace: ns.clone(),
        hash: "h0".to_string(),
        origin: Some(host),
    };
    let lineage = quilt::lineage::PackageLineage::from_remote(remote, "h1".to_string());

    let mut model = MockQuiltModel::new();
    model.expect_get_installed_packages_list().returning(|| {
        Ok(vec![
            quilt::LocalDomain::new(std::path::PathBuf::new())
                .create_installed_package(("acme", "demo").into())
                .unwrap(),
        ])
    });
    model
        .expect_get_installed_package_lineage()
        .returning(move |_| Ok(lineage.clone()));
    model.expect_get_installed_package().returning(|_| {
        Ok(Some(
            quilt::LocalDomain::new(std::path::PathBuf::new())
                .create_installed_package(("acme", "demo").into())
                .unwrap(),
        ))
    });
    // Status says Behind with a changeset present — refresh_then_maybe_pull
    // will see has_changes=true and skip the pull, returning Behind.
    let mut changes = BTreeMap::new();
    changes.insert(
        std::path::PathBuf::from("file.txt"),
        quilt::lineage::Change::Added(quilt::manifest::ManifestRow::default()),
    );
    model
        .expect_get_installed_package_status()
        .return_once(move |_, _| {
            Ok(quilt::lineage::InstalledPackageStatus::new(
                UpstreamState::Behind,
                changes,
            ))
        });

    let reporter = Arc::new(RecordingReporter::default());
    let inner = WatcherInner {
        settings: Arc::new(RwLock::new(enabled())),
        window_mode: Arc::new(RwLock::new(WindowMode::Focused)),
        publish_settings: Arc::new(RwLock::new(PublishSettings::default())),
        paused: RwLock::new(BTreeMap::new()),
        backoff: RwLock::new(BTreeMap::new()),
        login_blocked: RwLock::new(BTreeMap::new()),
        reporter: reporter.clone(),
        aggregator: test_aggregator(),
    };

    run_once(&model, &inner).await?;

    // Behind + changes is the "user must commit before we can pull"
    // path — pull is not invoked, but the package is not yet paused
    // either (it stays in Behind until either the user commits/pushes
    // or actual divergence shows up). The emitted status reflects
    // the cheap-refresh result.
    let statuses = reporter.statuses.lock().unwrap();
    assert_eq!(statuses.len(), 1);
    assert_eq!(statuses[0].1.status, "behind");
    assert!(statuses[0].1.has_changes);
    Ok(())
}

#[tokio::test]
async fn run_once_login_required_bumps_backoff() -> Result<(), Error> {
    let ns: Namespace = ("acme", "demo").into();
    let host: Host = "catalog.dev".parse().unwrap();
    let remote = quilt_uri::ManifestUri {
        bucket: "bucket".to_string(),
        namespace: ns.clone(),
        hash: "h0".to_string(),
        origin: Some(host.clone()),
    };
    let lineage = quilt::lineage::PackageLineage::from_remote(remote, "h0".to_string());

    let mut model = MockQuiltModel::new();
    model.expect_get_installed_packages_list().returning(|| {
        Ok(vec![
            quilt::LocalDomain::new(std::path::PathBuf::new())
                .create_installed_package(("acme", "demo").into())
                .unwrap(),
        ])
    });
    model
        .expect_get_installed_package_lineage()
        .returning(move |_| Ok(lineage.clone()));
    model.expect_get_installed_package().returning(|_| {
        Ok(Some(
            quilt::LocalDomain::new(std::path::PathBuf::new())
                .create_installed_package(("acme", "demo").into())
                .unwrap(),
        ))
    });
    // Status check itself fails with LoginRequired (mirrors what
    // `InstalledPackage::status` surfaces when the cached token has
    // expired).
    let host_for_status = host.clone();
    model
        .expect_get_installed_package_status()
        .returning(move |_, _| {
            Err(Error::from(quilt::Error::Login(
                quilt::LoginError::Required(Some(host_for_status.clone())),
            )))
        });

    let reporter = Arc::new(RecordingReporter::default());
    let inner = WatcherInner {
        settings: Arc::new(RwLock::new(enabled())),
        window_mode: Arc::new(RwLock::new(WindowMode::Focused)),
        publish_settings: Arc::new(RwLock::new(PublishSettings::default())),
        paused: RwLock::new(BTreeMap::new()),
        backoff: RwLock::new(BTreeMap::new()),
        login_blocked: RwLock::new(BTreeMap::new()),
        reporter: reporter.clone(),
        aggregator: test_aggregator(),
    };

    run_once(&model, &inner).await?;

    // No `report_status` emit — login required surfaces through its
    // own reporter method, and the namespace is not marked paused
    // (an explicit user action is required, not a code-level conflict).
    assert!(reporter.statuses.lock().unwrap().is_empty());
    assert!(inner.paused.read().await.is_empty());
    // Backoff entry exists and counts a failure — the next tick must
    // wait for it instead of retrying immediately.
    let backoff = inner.backoff.read().await;
    let entry = backoff
        .get(&ns)
        .expect("backoff entry should be set for LoginRequired");
    assert_eq!(entry.consecutive_failures, 1);

    // Logins are recorded.
    let logins = reporter.logins.lock().unwrap();
    assert_eq!(logins.len(), 1);
    assert_eq!(logins[0].as_ref(), Some(&host));
    Ok(())
}
