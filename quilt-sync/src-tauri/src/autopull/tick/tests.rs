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

/// Hex of an ASCII string, matching the per-byte path encoding in the status
/// fingerprint — for asserting a path appears hex-encoded in an outcome digest.
fn hex(s: &str) -> String {
    use std::fmt::Write as _;
    s.bytes().fold(String::new(), |mut out, b| {
        let _ = write!(out, "{b:02x}");
        out
    })
}

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
    let err = Error::from(quilt::Error::PackageOp(
        quilt::PackageOpError::AlreadyUpToDate,
    ));
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
        quilt::workflow::WorkflowValidationError::Rejected(
            quilt::workflow::RuleViolation::MessageRequired.into(),
        ),
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

/// An S3 refusal carrying the `AccessDenied` code, as it reaches the
/// classifiers from a publish, a pull, or the cheap status refresh.
fn access_denied_error() -> Error {
    Error::from(quilt::Error::S3(quilt::S3Error::new(
        quilt::S3ErrorKind::AccessDenied("s3://bucket/key".to_string()),
    )))
}

/// A role denial can never succeed on retry — the role must change first.
/// Leaving it in the transient bucket means autosync retries forever (capped
/// at 64 s) and the user is never told why nothing is syncing.
#[test]
fn access_denied_pauses_rather_than_retrying_forever() {
    let classified = classify_sync_err(access_denied_error()).unwrap_err();

    assert!(
        matches!(
            classified,
            WatchError::Conflict(PausedReason::RoleDenied { .. })
        ),
        "expected a role-denial pause, got: {classified:?}"
    );
}

/// The role name is *not* filled in by the classifier: it is a network call
/// and `classify_sync_err` is sync. `name_denied_role` fills it at the pause
/// site, where an await is available.
#[test]
fn classified_denial_leaves_the_role_unnamed() {
    match classify_sync_err(access_denied_error()).unwrap_err() {
        WatchError::Conflict(PausedReason::RoleDenied { role }) => {
            assert!(role.is_empty(), "classifier must not name the role: {role}");
        }
        other => panic!("expected Conflict(RoleDenied), got {other:?}"),
    }
}

/// The read side: Task 5 made `InstalledPackage::status` propagate a denial
/// instead of swallowing it, so the cheap status refresh now hits this
/// classifier too. Its default arm would otherwise back the denial off
/// forever, exactly like the publish side did.
#[test]
fn access_denied_on_status_refresh_pauses() {
    match classify_transient_or_login(access_denied_error()) {
        WatchError::Conflict(PausedReason::RoleDenied { .. }) => {}
        other => panic!("expected Conflict(RoleDenied), got {other:?}"),
    }
}

/// The counterweight to the arm above: only a denial pauses the status
/// refresh. A blip, a 5xx, or throttling must keep backing off, or a single
/// bad minute would park every package until the user intervenes.
#[test]
fn non_denial_s3_on_status_refresh_stays_transient() {
    let err = Error::from(quilt::Error::S3(quilt::S3Error::new(
        quilt::S3ErrorKind::ListObjects("connection reset by peer".to_string()),
    )));
    match classify_transient_or_login(err) {
        WatchError::Transient(_) => {}
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
    //
    // Also the counterweight to the `AccessDenied` arm above it: that arm
    // must catch denials *only*. A classifier that paused on every S3 error
    // would be a worse bug than the retry loop it replaced.
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
    run_once(&model, &RoleCache::default(), &inner).await?;
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
    // Clean tree: the dry-run classifier reports a straight surgical update.
    model
        .expect_package_pull_outcome()
        .times(1)
        .returning(|_| Ok(PullOutcome::CleanUpdate));
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

    run_once(&model, &RoleCache::default(), &inner).await?;

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

/// Behind + a non-conflicting local change now auto-pulls (was: fell through
/// to the publish arm and diverged). The dry-run classifier returns
/// `KeepsLocalChanges`, so the tick calls `package_pull`, which preserves the
/// local work — the resulting tree is `UpToDate` **and** still dirty.
#[tokio::test]
async fn behind_with_kept_changes_pulls() -> Result<(), Error> {
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
    // Behind with a local addition present.
    let mut changes = BTreeMap::new();
    changes.insert(
        std::path::PathBuf::from("local.txt"),
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
    // Dry run: the surgical update reconciles cleanly, keeping the local add.
    model.expect_package_pull_outcome().times(1).returning(|_| {
        Ok(PullOutcome::KeepsLocalChanges {
            added: vec![std::path::PathBuf::from("local.txt")],
            modified: Vec::new(),
            removed: Vec::new(),
        })
    });
    // The pull is actually performed.
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

    run_once(&model, &RoleCache::default(), &inner).await?;

    // Pulled → UpToDate, but the kept local work leaves the tree dirty.
    {
        let statuses = reporter.statuses.lock().unwrap();
        assert_eq!(statuses.len(), 1, "expected one status emit");
        assert_eq!(statuses[0].0, ns);
        assert_eq!(statuses[0].1.status, "up_to_date");
        assert!(
            statuses[0].1.has_changes,
            "kept local work must leave the tree dirty after pull"
        );
    }
    assert!(inner.paused.read().await.is_empty());
    Ok(())
}

/// Behind + a local change that the pull trivially resolves (identical edit /
/// both-removed) → the dry-run reports `KeepsLocalChanges` with ALL-EMPTY
/// lists. The pre-pull `has_changes` is true (the tree was dirty), but after
/// the pull the tree is clean, so the SUCCESS outcome must report
/// `has_changes == false` — no phantom pending changes for the UI.
#[tokio::test]
async fn behind_trivially_resolved_reports_clean() -> Result<(), Error> {
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
    // Pre-pull: the tree is dirty (a local edit is present), so the stale-true
    // path would report `has_changes == true` if we trusted the pre-pull walk.
    let mut changes = BTreeMap::new();
    changes.insert(
        std::path::PathBuf::from("same.txt"),
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
    // Dry run: the pull reconciles every local change (e.g. identical edit) →
    // KeepsLocalChanges with all-empty lists = nothing kept.
    model.expect_package_pull_outcome().times(1).returning(|_| {
        Ok(PullOutcome::KeepsLocalChanges {
            added: Vec::new(),
            modified: Vec::new(),
            removed: Vec::new(),
        })
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

    run_once(&model, &RoleCache::default(), &inner).await?;

    // Pulled → UpToDate, and the trivially-resolved tree is clean: the
    // post-pull outcome overrides the stale-true pre-pull `has_changes`.
    {
        let statuses = reporter.statuses.lock().unwrap();
        assert_eq!(statuses.len(), 1, "expected one status emit");
        assert_eq!(statuses[0].0, ns);
        assert_eq!(statuses[0].1.status, "up_to_date");
        assert!(
            !statuses[0].1.has_changes,
            "a trivially-resolved pull must report a clean tree, not phantom changes"
        );
    }
    assert!(inner.paused.read().await.is_empty());
    Ok(())
}

/// Behind + a `CleanUpdate` while the pre-pull walk saw local changes. A
/// `CleanUpdate` keeps nothing, so the success outcome reports
/// `has_changes == false` regardless of the (stale) pre-pull boolean.
#[tokio::test]
async fn behind_clean_update_ignores_stale_pre_pull_changes() -> Result<(), Error> {
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
    // Pre-pull walk reports a dirty tree (stale-true source).
    let mut changes = BTreeMap::new();
    changes.insert(
        std::path::PathBuf::from("stale.txt"),
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
    model
        .expect_package_pull_outcome()
        .times(1)
        .returning(|_| Ok(PullOutcome::CleanUpdate));
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

    run_once(&model, &RoleCache::default(), &inner).await?;

    {
        let statuses = reporter.statuses.lock().unwrap();
        assert_eq!(statuses.len(), 1, "expected one status emit");
        assert_eq!(statuses[0].1.status, "up_to_date");
        assert!(
            !statuses[0].1.has_changes,
            "a CleanUpdate keeps nothing — the tree is clean after pull"
        );
    }
    assert!(inner.paused.read().await.is_empty());
    Ok(())
}

/// An expired session surfacing mid-dry-run (from `package_pull_outcome`) must
/// classify as `LoginRequired`, not back off as a `Transient` — so the login
/// affordance appears this tick instead of one backoff later.
#[tokio::test]
async fn dry_run_login_required_is_classified() -> Result<(), Error> {
    let ns: Namespace = ("acme", "demo").into();
    let host: Host = "catalog.dev".parse().unwrap();
    let remote = quilt_uri::ManifestUri {
        bucket: "bucket".to_string(),
        namespace: ns.clone(),
        hash: "h0".to_string(),
        origin: Some(host.clone()),
    };
    let lineage = quilt::lineage::PackageLineage::from_remote(remote, "h1".to_string());

    let mut model = MockQuiltModel::new();
    model.expect_get_installed_package().returning(|_| {
        Ok(Some(
            quilt::LocalDomain::new(std::path::PathBuf::new())
                .create_installed_package(("acme", "demo").into())
                .unwrap(),
        ))
    });
    // Status refresh succeeds and reports Behind, so the pull dry-run runs.
    model
        .expect_get_installed_package_status()
        .returning(|_, _| {
            Ok(quilt::lineage::InstalledPackageStatus::new(
                UpstreamState::Behind,
                BTreeMap::new(),
            ))
        });
    // The dry-run itself hits an expired token.
    let host_for_dry_run = host.clone();
    model
        .expect_package_pull_outcome()
        .times(1)
        .returning(move |_| {
            Err(Error::from(quilt::Error::Login(
                quilt::LoginError::Required(Some(host_for_dry_run.clone())),
            )))
        });

    let result = refresh_then_maybe_sync(
        &model,
        &ns,
        &lineage,
        &PublishSettings::default(),
        Duration::from_secs(0),
        true,
        true,
    )
    .await;

    match result {
        Err(WatchError::LoginRequired(Some(h))) => assert_eq!(h, host),
        other => panic!("expected LoginRequired(Some(_)), got {other:?}"),
    }
    Ok(())
}

/// Behind + a real conflict (a tracked path changed on both sides) pauses the
/// namespace with `PullConflict`. The dry-run classifier returns `Blocked`, so
/// the tick never calls `package_pull`.
#[tokio::test]
async fn behind_blocked_pauses() -> Result<(), Error> {
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
    let mut changes = BTreeMap::new();
    changes.insert(
        std::path::PathBuf::from("conflict.txt"),
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
    // Dry run: a tracked path changed on both sides → the whole pull blocks.
    model.expect_package_pull_outcome().times(1).returning(|_| {
        Ok(PullOutcome::Blocked {
            conflicts: vec![std::path::PathBuf::from("conflict.txt")],
        })
    });
    // The pull itself must never run when the outcome is Blocked.
    model.expect_package_pull().times(0);

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

    run_once(&model, &RoleCache::default(), &inner).await?;

    // Namespace is paused with PullConflict carrying the conflicting file.
    {
        let paused = inner.paused.read().await;
        match paused.get(&ns) {
            Some(PausedReason::PullConflict(files)) => {
                assert_eq!(files, &vec!["conflict.txt".to_string()]);
            }
            other => panic!("expected PullConflict, got {other:?}"),
        }
    }
    // A Conflict does not back off — it waits for the user's action.
    assert!(inner.backoff.read().await.is_empty());
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

    run_once(&model, &RoleCache::default(), &inner).await?;

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

/// A no-action tick (up-to-date, local changes, nothing to pull or publish)
/// carries a fingerprint of the observed status, so two identical ticks
/// produce identical outcomes and a consumer can skip the repeat.
#[tokio::test]
async fn no_action_tick_carries_status_fingerprint() -> Result<(), Error> {
    let ns: Namespace = ("acme", "demo").into();
    let remote = quilt_uri::ManifestUri {
        bucket: "bucket".to_string(),
        namespace: ns.clone(),
        hash: "h0".to_string(),
        origin: None,
    };
    let lineage = quilt::lineage::PackageLineage::from_remote(remote, "h0".to_string());

    let mut model = MockQuiltModel::new();
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
            let mut changes = BTreeMap::new();
            changes.insert(
                std::path::PathBuf::from("a.txt"),
                quilt::lineage::Change::Modified(quilt::manifest::ManifestRow::default()),
            );
            Ok(quilt::lineage::InstalledPackageStatus::new(
                UpstreamState::UpToDate,
                changes,
            ))
        });

    // pull + push disabled → the tick observes and does nothing.
    let outcome = refresh_then_maybe_sync(
        &model,
        &ns,
        &lineage,
        &PublishSettings::default(),
        Duration::from_secs(0),
        false,
        false,
    )
    .await
    .expect("no-action tick should be Ok");

    assert!(outcome.has_changes);
    let hex_path = hex("a.txt");
    assert!(
        outcome.fingerprint.starts_with("up_to_date;")
            && outcome.fingerprint.contains(&format!("{hex_path}:M:")),
        "outcome should carry the observation fingerprint, got: {}",
        outcome.fingerprint
    );
    Ok(())
}

/// A persistently-paused package re-emits its conflict every tick. The status
/// event it carries must have a stable, non-empty fingerprint (derived from the
/// heuristic status it reports) — otherwise a paused page rebuilds every tick,
/// the same trap the no-action path had.
#[tokio::test]
async fn conflict_emit_carries_stable_fingerprint() -> Result<(), Error> {
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
    let mut changes = BTreeMap::new();
    changes.insert(
        std::path::PathBuf::from("conflict.txt"),
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
    model.expect_package_pull_outcome().times(1).returning(|_| {
        Ok(PullOutcome::Blocked {
            conflicts: vec![std::path::PathBuf::from("conflict.txt")],
        })
    });
    model.expect_package_pull().times(0);

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

    run_once(&model, &RoleCache::default(), &inner).await?;

    let statuses = reporter.statuses.lock().unwrap();
    assert_eq!(
        statuses.len(),
        1,
        "the conflict tick emits one status event"
    );
    let event = &statuses[0].1;
    assert_eq!(event.status, "paused");
    // The fingerprint is a stable function of the heuristic observation, so the
    // same pause re-reported carries the same fingerprint (consumer skips it).
    assert_eq!(
        event.fingerprint,
        format!("{};{}", event.status, event.has_changes),
        "conflict emit should carry a stable heuristic fingerprint, not an empty default"
    );
    assert!(!event.fingerprint.is_empty());
    Ok(())
}
