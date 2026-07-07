//! Publish-branch (autosync M1) `run_once` tests, plus the per-direction
//! flag-gating and tray-aggregator cases that share the same fixtures.

use super::*;

/// Shared boilerplate for the publish-branch tests: returns a
/// `MockQuiltModel` wired with the package list, lineage, package, and
/// status mocks, plus the namespace and lineage clones for tests that
/// need them. Each caller wires its own `expect_package_publish` /
/// `expect_package_pull` next.
#[allow(clippy::needless_pass_by_value)]
fn fixture_with_lineage_and_status(
    lineage: quilt::lineage::PackageLineage,
    status: quilt::lineage::InstalledPackageStatus,
) -> (MockQuiltModel, Namespace) {
    let ns: Namespace = ("acme", "demo").into();
    let mut model = MockQuiltModel::new();
    model.expect_get_installed_packages_list().returning(|| {
        Ok(vec![
            quilt::LocalDomain::new(std::path::PathBuf::new())
                .create_installed_package(("acme", "demo").into())
                .unwrap(),
        ])
    });
    let lineage_clone = lineage.clone();
    model
        .expect_get_installed_package_lineage()
        .returning(move |_| Ok(lineage_clone.clone()));
    model.expect_get_installed_package().returning(|_| {
        Ok(Some(
            quilt::LocalDomain::new(std::path::PathBuf::new())
                .create_installed_package(("acme", "demo").into())
                .unwrap(),
        ))
    });
    let status_mutex = std::sync::Mutex::new(Some(status));
    model
        .expect_get_installed_package_status()
        .returning(move |_, _| Ok(status_mutex.lock().unwrap().take().unwrap()));
    // `model::package_publish` (free fn) routes the workflow lookup
    // through the trait. These fixtures use default publish settings, so
    // `publish_with_settings` sends `WorkflowIntent::BucketDefault`; the
    // per-remote workflow gets enforced by `flow::publish_package` from
    // the remote-side config regardless. The mock ignores the intent and
    // returns "no workflow".
    model.expect_resolve_workflow().returning(|_, _| Ok(None));
    (model, ns)
}

fn remote_for(ns: &Namespace) -> quilt_uri::ManifestUri {
    let host: Host = "catalog.dev".parse().unwrap();
    quilt_uri::ManifestUri {
        bucket: "bucket".to_string(),
        namespace: ns.clone(),
        hash: "h0".to_string(),
        origin: Some(host),
    }
}

fn fake_push_outcome(ns: &Namespace) -> quilt::PushOutcome {
    quilt::PushOutcome {
        manifest_uri: quilt_uri::ManifestUri {
            bucket: "bucket".to_string(),
            namespace: ns.clone(),
            hash: "h2".to_string(),
            origin: None,
        },
        certified_latest: true,
    }
}

fn quiet_status(
    upstream: UpstreamState,
    changes: quilt::lineage::ChangeSet,
) -> quilt::lineage::InstalledPackageStatus {
    let mut status = quilt::lineage::InstalledPackageStatus::new(upstream, changes);
    // Far in the past so `working_tree_quiet(now, 30s)` passes. We
    // would write `Duration::from_hours(1)` but it is 1.87+ and
    // workspace MSRV is 1.85.
    #[allow(clippy::duration_suboptimal_units)]
    let one_hour = Duration::from_secs(3600);
    status.most_recent_mtime = Some(SystemTime::now() - one_hour);
    status
}

fn make_inner_for_run_once(reporter: Arc<RecordingReporter>) -> WatcherInner {
    WatcherInner {
        settings: Arc::new(RwLock::new(enabled())),
        window_mode: Arc::new(RwLock::new(WindowMode::Focused)),
        publish_settings: Arc::new(RwLock::new(PublishSettings::default())),
        paused: RwLock::new(BTreeMap::new()),
        backoff: RwLock::new(BTreeMap::new()),
        login_blocked: RwLock::new(BTreeMap::new()),
        reporter,
        aggregator: test_aggregator(),
    }
}

fn make_inner_with_flags(
    reporter: Arc<RecordingReporter>,
    pull_enabled: bool,
    push_enabled: bool,
) -> WatcherInner {
    WatcherInner {
        settings: Arc::new(RwLock::new(AutosyncSettings {
            pull: PullSettings {
                enabled: pull_enabled,
                ..PullSettings::default()
            },
            push: PushSettings {
                enabled: push_enabled,
                ..PushSettings::default()
            },
            close_to_tray: false,
        })),
        window_mode: Arc::new(RwLock::new(WindowMode::Focused)),
        publish_settings: Arc::new(RwLock::new(PublishSettings::default())),
        paused: RwLock::new(BTreeMap::new()),
        backoff: RwLock::new(BTreeMap::new()),
        login_blocked: RwLock::new(BTreeMap::new()),
        reporter,
        aggregator: test_aggregator(),
    }
}

#[tokio::test]
async fn run_once_publishes_on_changes() -> Result<(), Error> {
    let ns: Namespace = ("acme", "demo").into();
    let mut changes = BTreeMap::new();
    changes.insert(
        std::path::PathBuf::from("file.txt"),
        quilt::lineage::Change::Added(quilt::manifest::ManifestRow::default()),
    );
    let lineage = quilt::lineage::PackageLineage::from_remote(remote_for(&ns), "h0".to_string());

    let (mut model, _) =
        fixture_with_lineage_and_status(lineage, quiet_status(UpstreamState::UpToDate, changes));
    let ns_for_push = ns.clone();
    model
        .expect_package_publish()
        .times(1)
        .returning(move |_, _, _, _, _, _| {
            Ok(quilt::PublishOutcome::CommittedAndPushed(
                fake_push_outcome(&ns_for_push),
            ))
        });

    let reporter = Arc::new(RecordingReporter::default());
    let inner = make_inner_for_run_once(reporter.clone());
    run_once(&model, &inner).await?;

    {
        let published = reporter.published.lock().unwrap();
        assert_eq!(published.len(), 1, "expected exactly one publish");
        assert_eq!(published[0].0, ns);
        // Default message_template is None → falls back to summary.
        assert_eq!(published[0].1, "Add file.txt");
    }
    {
        let statuses = reporter.statuses.lock().unwrap();
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].1.status, "up_to_date");
        assert!(!statuses[0].1.has_changes);
    }
    assert!(inner.paused.read().await.is_empty());
    Ok(())
}

#[tokio::test]
async fn run_once_publishes_on_pending_commit_only() -> Result<(), Error> {
    let ns: Namespace = ("acme", "demo").into();
    let lineage = quilt::lineage::PackageLineage {
        commit: Some(quilt::lineage::CommitState {
            hash: "h_local".to_string(),
            ..quilt::lineage::CommitState::default()
        }),
        remote_uri: Some(remote_for(&ns)),
        base_hash: "h0".to_string(),
        latest_hash: "h0".to_string(),
        ..quilt::lineage::PackageLineage::default()
    };
    // No changes → working_tree_quiet passes trivially.
    let status = quilt::lineage::InstalledPackageStatus::new(UpstreamState::Ahead, BTreeMap::new());

    let (mut model, _) = fixture_with_lineage_and_status(lineage, status);
    let ns_for_push = ns.clone();
    model
        .expect_package_publish()
        .times(1)
        .returning(move |_, _, _, _, _, _| {
            Ok(quilt::PublishOutcome::PushedOnly(fake_push_outcome(
                &ns_for_push,
            )))
        });

    let reporter = Arc::new(RecordingReporter::default());
    let inner = make_inner_for_run_once(reporter.clone());
    run_once(&model, &inner).await?;

    assert_eq!(reporter.published.lock().unwrap().len(), 1);
    let statuses = reporter.statuses.lock().unwrap();
    assert_eq!(statuses.len(), 1);
    assert_eq!(statuses[0].1.status, "up_to_date");
    Ok(())
}

#[tokio::test]
async fn run_once_skips_publish_when_not_quiet() -> Result<(), Error> {
    let ns: Namespace = ("acme", "demo").into();
    let mut changes = BTreeMap::new();
    changes.insert(
        std::path::PathBuf::from("file.txt"),
        quilt::lineage::Change::Added(quilt::manifest::ManifestRow::default()),
    );
    let mut status = quilt::lineage::InstalledPackageStatus::new(UpstreamState::UpToDate, changes);
    // Just edited → not quiet (focused cadence is 30 s by default).
    status.most_recent_mtime = Some(SystemTime::now());

    let lineage = quilt::lineage::PackageLineage::from_remote(remote_for(&ns), "h0".to_string());
    let (mut model, _) = fixture_with_lineage_and_status(lineage, status);
    // No `expect_package_publish` — mockall panics if the call happens.
    model.expect_package_publish().times(0);

    let reporter = Arc::new(RecordingReporter::default());
    let inner = make_inner_for_run_once(reporter.clone());
    run_once(&model, &inner).await?;

    assert!(reporter.published.lock().unwrap().is_empty());
    let statuses = reporter.statuses.lock().unwrap();
    assert_eq!(statuses.len(), 1);
    // Stays in the pre-publish state.
    assert_eq!(statuses[0].1.status, "up_to_date");
    assert!(statuses[0].1.has_changes);
    Ok(())
}

#[tokio::test]
async fn run_once_skips_publish_when_behind() -> Result<(), Error> {
    let ns: Namespace = ("acme", "demo").into();
    let mut changes = BTreeMap::new();
    changes.insert(
        std::path::PathBuf::from("file.txt"),
        quilt::lineage::Change::Added(quilt::manifest::ManifestRow::default()),
    );
    let status = quiet_status(UpstreamState::Behind, changes);
    let lineage = quilt::lineage::PackageLineage::from_remote(remote_for(&ns), "h1".to_string());

    let (mut model, _) = fixture_with_lineage_and_status(lineage, status);
    model.expect_package_pull().times(0);
    model.expect_package_publish().times(0);

    let reporter = Arc::new(RecordingReporter::default());
    let inner = make_inner_for_run_once(reporter.clone());
    run_once(&model, &inner).await?;

    assert!(reporter.published.lock().unwrap().is_empty());
    let statuses = reporter.statuses.lock().unwrap();
    assert_eq!(statuses.len(), 1);
    assert_eq!(statuses[0].1.status, "behind");
    assert!(statuses[0].1.has_changes);
    Ok(())
}

#[tokio::test]
async fn run_once_publishes_local_first_push() -> Result<(), Error> {
    let ns: Namespace = ("acme", "demo").into();
    let host: Host = "catalog.dev".parse().unwrap();
    let lineage = quilt::lineage::PackageLineage {
        remote_uri: Some(quilt_uri::ManifestUri {
            bucket: "bucket".to_string(),
            namespace: ns.clone(),
            hash: String::new(),
            origin: Some(host),
        }),
        // `latest_hash` empty + `remote.hash` empty → `Local` (truly
        // empty bucket, safe to first-push).
        latest_hash: String::new(),
        ..quilt::lineage::PackageLineage::default()
    };
    let mut changes = BTreeMap::new();
    changes.insert(
        std::path::PathBuf::from("file.txt"),
        quilt::lineage::Change::Added(quilt::manifest::ManifestRow::default()),
    );
    let status = quiet_status(UpstreamState::Local, changes);

    let (mut model, _) = fixture_with_lineage_and_status(lineage, status);
    let ns_for_push = ns.clone();
    model
        .expect_package_publish()
        .times(1)
        .returning(move |_, _, _, _, _, _| {
            Ok(quilt::PublishOutcome::CommittedAndPushed(
                fake_push_outcome(&ns_for_push),
            ))
        });

    let reporter = Arc::new(RecordingReporter::default());
    let inner = make_inner_for_run_once(reporter.clone());
    run_once(&model, &inner).await?;

    assert_eq!(reporter.published.lock().unwrap().len(), 1);
    Ok(())
}

#[tokio::test]
async fn run_once_pauses_on_classic_diverged() -> Result<(), Error> {
    let ns: Namespace = ("acme", "demo").into();
    // `Diverged` from the conventional source: differing base/latest and
    // a local commit.
    let lineage = quilt::lineage::PackageLineage {
        commit: Some(quilt::lineage::CommitState {
            hash: "local".to_string(),
            ..quilt::lineage::CommitState::default()
        }),
        remote_uri: Some(remote_for(&ns)),
        base_hash: "h0".to_string(),
        latest_hash: "h1".to_string(),
        ..quilt::lineage::PackageLineage::default()
    };
    let status = quiet_status(UpstreamState::Diverged, BTreeMap::new());

    let (mut model, _) = fixture_with_lineage_and_status(lineage, status);
    model.expect_package_publish().times(0);
    model.expect_package_pull().times(0);

    let reporter = Arc::new(RecordingReporter::default());
    let inner = make_inner_for_run_once(reporter.clone());
    run_once(&model, &inner).await?;

    assert!(reporter.published.lock().unwrap().is_empty());
    {
        let statuses = reporter.statuses.lock().unwrap();
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].1.status, "diverged");
    }
    let paused = inner.paused.read().await;
    assert!(matches!(paused.get(&ns), Some(PausedReason::Diverged)));
    Ok(())
}

#[tokio::test]
async fn run_once_pauses_on_foreign_remote_diverged() -> Result<(), Error> {
    let ns: Namespace = ("acme", "demo").into();
    let host: Host = "catalog.dev".parse().unwrap();
    // Foreign-remote `Diverged`: `remote.hash` empty, `latest_hash`
    // non-empty (a teammate published under the same namespace).
    let lineage = quilt::lineage::PackageLineage {
        remote_uri: Some(quilt_uri::ManifestUri {
            bucket: "bucket".to_string(),
            namespace: ns.clone(),
            hash: String::new(),
            origin: Some(host),
        }),
        latest_hash: "abc".to_string(),
        ..quilt::lineage::PackageLineage::default()
    };
    let status = quiet_status(UpstreamState::Diverged, BTreeMap::new());

    let (mut model, _) = fixture_with_lineage_and_status(lineage, status);
    model.expect_package_publish().times(0);
    model.expect_package_pull().times(0);

    let reporter = Arc::new(RecordingReporter::default());
    let inner = make_inner_for_run_once(reporter.clone());
    run_once(&model, &inner).await?;

    let paused = inner.paused.read().await;
    assert!(matches!(paused.get(&ns), Some(PausedReason::Diverged)));
    Ok(())
}

#[tokio::test]
async fn run_once_pauses_on_push_workflow_failure() -> Result<(), Error> {
    let ns: Namespace = ("acme", "demo").into();
    let mut changes = BTreeMap::new();
    changes.insert(
        std::path::PathBuf::from("file.txt"),
        quilt::lineage::Change::Added(quilt::manifest::ManifestRow::default()),
    );
    let status = quiet_status(UpstreamState::UpToDate, changes);
    let lineage = quilt::lineage::PackageLineage::from_remote(remote_for(&ns), "h0".to_string());

    let (mut model, _) = fixture_with_lineage_and_status(lineage, status);
    model
        .expect_package_publish()
        .times(1)
        .returning(|_, _, _, _, _, _| {
            Err(Error::from(quilt::Error::PackageOp(
                quilt::PackageOpError::Push("workflow rejected metadata".to_string()),
            )))
        });

    let reporter = Arc::new(RecordingReporter::default());
    let inner = make_inner_for_run_once(reporter.clone());
    run_once(&model, &inner).await?;

    {
        let paused_evts = reporter.paused.lock().unwrap();
        assert_eq!(paused_evts.len(), 1);
        match &paused_evts[0].1 {
            PausedReason::Other(msg) => assert_eq!(msg, "workflow rejected metadata"),
            other => panic!("expected Other(_), got {other:?}"),
        }
    }
    // Status string must be `"paused"`, NOT `"error"`: `"error"` is
    // reserved for "we couldn't reach the remote" and triggers the
    // Login affordance in the UI, which would be misleading for a
    // workflow-validation failure.
    {
        let statuses = reporter.statuses.lock().unwrap();
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].1.status, "paused");
        assert!(!statuses[0].1.has_changes);
    }
    let paused_map = inner.paused.read().await;
    assert!(matches!(
        paused_map.get(&ns),
        Some(PausedReason::Other(msg)) if msg == "workflow rejected metadata"
    ));
    Ok(())
}

#[tokio::test]
async fn run_once_backoffs_on_transient_publish_error() -> Result<(), Error> {
    let ns: Namespace = ("acme", "demo").into();
    let mut changes = BTreeMap::new();
    changes.insert(
        std::path::PathBuf::from("file.txt"),
        quilt::lineage::Change::Added(quilt::manifest::ManifestRow::default()),
    );
    let status = quiet_status(UpstreamState::UpToDate, changes);
    let lineage = quilt::lineage::PackageLineage::from_remote(remote_for(&ns), "h0".to_string());

    let (mut model, _) = fixture_with_lineage_and_status(lineage, status);
    model
        .expect_package_publish()
        .times(1)
        .returning(|_, _, _, _, _, _| {
            Err(Error::from(quilt::Error::Io(std::io::Error::new(
                std::io::ErrorKind::ConnectionReset,
                "connection reset",
            ))))
        });

    let reporter = Arc::new(RecordingReporter::default());
    let inner = make_inner_for_run_once(reporter.clone());
    run_once(&model, &inner).await?;

    // Transient: no paused entry, no status emit, but backoff bumped.
    assert!(inner.paused.read().await.is_empty());
    let backoff = inner.backoff.read().await;
    let entry = backoff
        .get(&ns)
        .expect("backoff entry should be set on Transient");
    assert_eq!(entry.consecutive_failures, 1);
    Ok(())
}

#[tokio::test]
async fn run_once_login_required_on_publish() -> Result<(), Error> {
    let ns: Namespace = ("acme", "demo").into();
    let host: Host = "catalog.dev".parse().unwrap();
    let mut changes = BTreeMap::new();
    changes.insert(
        std::path::PathBuf::from("file.txt"),
        quilt::lineage::Change::Added(quilt::manifest::ManifestRow::default()),
    );
    let status = quiet_status(UpstreamState::UpToDate, changes);
    let lineage = quilt::lineage::PackageLineage::from_remote(remote_for(&ns), "h0".to_string());

    let (mut model, _) = fixture_with_lineage_and_status(lineage, status);
    let host_for_publish = host.clone();
    model
        .expect_package_publish()
        .times(1)
        .returning(move |_, _, _, _, _, _| {
            Err(Error::from(quilt::Error::Login(
                quilt::LoginError::Required(Some(host_for_publish.clone())),
            )))
        });

    let reporter = Arc::new(RecordingReporter::default());
    let inner = make_inner_for_run_once(reporter.clone());
    run_once(&model, &inner).await?;

    assert!(inner.paused.read().await.is_empty());
    let backoff = inner.backoff.read().await;
    let entry = backoff
        .get(&ns)
        .expect("backoff entry should be set on LoginRequired");
    assert_eq!(entry.consecutive_failures, 1);

    let logins = reporter.logins.lock().unwrap();
    assert_eq!(logins.len(), 1);
    assert_eq!(logins[0].as_ref(), Some(&host));
    Ok(())
}

#[tokio::test]
async fn publish_quiet_window_reads_idle_timeout_not_pull_cadence() -> Result<(), Error> {
    // The push-side quiet window must come from
    // `push.idle_timeout_secs`, not `cadence_for_mode(...)`.
    // Concretely: with focused_secs=5 and idle_timeout_secs=60, a
    // tick taken with most_recent_mtime=30s-ago must NOT publish
    // (still inside the 60s idle window), even though the pull
    // cadence (5s) has long elapsed.
    let ns: Namespace = ("acme", "demo").into();
    let mut changes = BTreeMap::new();
    changes.insert(
        std::path::PathBuf::from("file.txt"),
        quilt::lineage::Change::Added(quilt::manifest::ManifestRow::default()),
    );
    let mut status = quilt::lineage::InstalledPackageStatus::new(UpstreamState::UpToDate, changes);
    status.most_recent_mtime = Some(SystemTime::now() - Duration::from_secs(30));

    let lineage = quilt::lineage::PackageLineage::from_remote(remote_for(&ns), "h0".to_string());
    let (mut model, _) = fixture_with_lineage_and_status(lineage, status);
    // Mockall panics if package_publish is called — that's the assertion.
    model.expect_package_publish().times(0);

    let settings = AutosyncSettings {
        pull: PullSettings {
            enabled: true,
            focused_secs: 5,
            unfocused_secs: 5,
            closed_secs: 5,
        },
        push: PushSettings {
            enabled: true,
            idle_timeout_secs: 60,
        },
        close_to_tray: false,
    };
    let reporter = Arc::new(RecordingReporter::default());
    let inner = WatcherInner {
        settings: Arc::new(RwLock::new(settings)),
        window_mode: Arc::new(RwLock::new(WindowMode::Focused)),
        publish_settings: Arc::new(RwLock::new(PublishSettings::default())),
        paused: RwLock::new(BTreeMap::new()),
        backoff: RwLock::new(BTreeMap::new()),
        login_blocked: RwLock::new(BTreeMap::new()),
        reporter: reporter.clone(),
        aggregator: test_aggregator(),
    };

    run_once(&model, &inner).await?;

    assert!(
        reporter.published.lock().unwrap().is_empty(),
        "tick must defer publish while inside the 60s idle window"
    );
    Ok(())
}

// ── per-direction flag gating (pull_enabled / push_enabled) ─────────

#[tokio::test]
async fn run_once_pull_only_does_not_publish_changes() -> Result<(), Error> {
    // pull_enabled=true, push_enabled=false: an UpToDate package
    // with local changes must not be auto-published. The pull-side
    // gate doesn't fire either (UpToDate isn't Behind), so this is
    // a no-action tick.
    let ns: Namespace = ("acme", "demo").into();
    let mut changes = BTreeMap::new();
    changes.insert(
        std::path::PathBuf::from("file.txt"),
        quilt::lineage::Change::Added(quilt::manifest::ManifestRow::default()),
    );
    let lineage = quilt::lineage::PackageLineage::from_remote(remote_for(&ns), "h0".to_string());
    let (mut model, _) =
        fixture_with_lineage_and_status(lineage, quiet_status(UpstreamState::UpToDate, changes));
    model.expect_package_publish().times(0);
    model.expect_package_pull().times(0);

    let reporter = Arc::new(RecordingReporter::default());
    let inner = make_inner_with_flags(reporter.clone(), true, false);
    run_once(&model, &inner).await?;

    assert!(reporter.published.lock().unwrap().is_empty());
    let statuses = reporter.statuses.lock().unwrap();
    assert_eq!(statuses.len(), 1);
    assert_eq!(statuses[0].1.status, "up_to_date");
    assert!(statuses[0].1.has_changes);
    Ok(())
}

#[tokio::test]
async fn run_once_push_only_does_not_pull_behind() -> Result<(), Error> {
    // pull_enabled=false, push_enabled=true: a Behind/clean package
    // (the pull branch's home turf) must not be auto-pulled. The
    // publish branch doesn't fire either (no local changes, no
    // pending commit).
    let ns: Namespace = ("acme", "demo").into();
    let lineage = quilt::lineage::PackageLineage::from_remote(remote_for(&ns), "h1".to_string());
    let status =
        quilt::lineage::InstalledPackageStatus::new(UpstreamState::Behind, BTreeMap::new());
    let (mut model, _) = fixture_with_lineage_and_status(lineage, status);
    model.expect_package_pull().times(0);
    model.expect_package_publish().times(0);

    let reporter = Arc::new(RecordingReporter::default());
    let inner = make_inner_with_flags(reporter.clone(), false, true);
    run_once(&model, &inner).await?;

    let statuses = reporter.statuses.lock().unwrap();
    assert_eq!(statuses.len(), 1);
    assert_eq!(statuses[0].1.status, "behind");
    assert!(!statuses[0].1.has_changes);
    Ok(())
}

#[tokio::test]
async fn run_once_both_off_is_a_noop() -> Result<(), Error> {
    // Both flags off: run_once short-circuits before touching the
    // model. The mock has zero expectations to prove no calls are
    // made.
    let model = MockQuiltModel::new();
    let reporter = Arc::new(RecordingReporter::default());
    let inner = make_inner_with_flags(reporter.clone(), false, false);
    run_once(&model, &inner).await?;
    assert!(reporter.statuses.lock().unwrap().is_empty());
    Ok(())
}

#[tokio::test]
async fn run_once_publishes_aggregator_status_on_pause() -> Result<(), Error> {
    let ns: Namespace = ("acme", "demo").into();
    let mut changes = BTreeMap::new();
    changes.insert(
        std::path::PathBuf::from("file.txt"),
        quilt::lineage::Change::Added(quilt::manifest::ManifestRow::default()),
    );
    let status = quiet_status(UpstreamState::UpToDate, changes);
    let lineage = quilt::lineage::PackageLineage::from_remote(remote_for(&ns), "h0".to_string());
    let (mut model, _) = fixture_with_lineage_and_status(lineage, status);
    model
        .expect_package_publish()
        .times(1)
        .returning(|_, _, _, _, _, _| {
            Err(Error::from(quilt::Error::PackageOp(
                quilt::PackageOpError::Push("workflow rejected".to_string()),
            )))
        });

    let reporter = Arc::new(RecordingReporter::default());
    let (tx, rx) = tokio::sync::watch::channel(crate::autopull::status::SyncTrayStatus::default());
    let aggregator = Arc::new(crate::autopull::status::SyncTrayAggregator::new(tx));
    let inner = WatcherInner {
        settings: Arc::new(RwLock::new(enabled())),
        window_mode: Arc::new(RwLock::new(WindowMode::Focused)),
        publish_settings: Arc::new(RwLock::new(PublishSettings::default())),
        paused: RwLock::new(BTreeMap::new()),
        backoff: RwLock::new(BTreeMap::new()),
        login_blocked: RwLock::new(BTreeMap::new()),
        reporter: reporter.clone(),
        aggregator,
    };
    run_once(&model, &inner).await?;
    let after = rx.borrow().clone();
    assert_eq!(after.mode, crate::autopull::status::TrayMode::Paused);
    assert_eq!(after.error.as_deref(), Some("workflow rejected"));
    Ok(())
}

#[tokio::test]
async fn run_once_publishes_pending_changes_count() -> Result<(), Error> {
    // Behind + has_changes path: refresh_then_maybe_sync returns
    // has_changes = true and tick.rs must propagate that to the
    // aggregator as pending_changes = 1.
    let ns: Namespace = ("acme", "demo").into();
    let mut changes = BTreeMap::new();
    changes.insert(
        std::path::PathBuf::from("file.txt"),
        quilt::lineage::Change::Added(quilt::manifest::ManifestRow::default()),
    );
    let lineage = quilt::lineage::PackageLineage::from_remote(remote_for(&ns), "h1".to_string());
    let (model, _) =
        fixture_with_lineage_and_status(lineage, quiet_status(UpstreamState::Behind, changes));
    let reporter = Arc::new(RecordingReporter::default());
    let (tx, rx) = tokio::sync::watch::channel(crate::autopull::status::SyncTrayStatus::default());
    let aggregator = Arc::new(crate::autopull::status::SyncTrayAggregator::new(tx));
    let inner = WatcherInner {
        settings: Arc::new(RwLock::new(enabled())),
        window_mode: Arc::new(RwLock::new(WindowMode::Focused)),
        publish_settings: Arc::new(RwLock::new(PublishSettings::default())),
        paused: RwLock::new(BTreeMap::new()),
        backoff: RwLock::new(BTreeMap::new()),
        login_blocked: RwLock::new(BTreeMap::new()),
        reporter: reporter.clone(),
        aggregator,
    };
    run_once(&model, &inner).await?;
    assert_eq!(rx.borrow().pending_changes, 1);
    Ok(())
}
