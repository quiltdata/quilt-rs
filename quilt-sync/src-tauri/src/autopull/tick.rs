use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::time::Duration;
use std::time::Instant;
use std::time::SystemTime;

use quilt_uri::Host;
use quilt_uri::Namespace;

use crate::Error;
use crate::autopull::PausedReason;
use crate::autopull::WatcherInner;
use crate::autopull::reporter::PackageStatusEvent;
use crate::model;
use crate::model::QuiltModel;
use crate::publish_settings::PublishSettings;
use crate::quilt;
use crate::telemetry::prelude::*;

#[derive(Debug)]
pub(crate) struct RefreshOutcome {
    pub upstream: quilt::lineage::UpstreamState,
    pub has_changes: bool,
    /// `Some(message)` only on the publish success path; `None` on
    /// pull success, on quiet-window deferral, and on any no-action
    /// tick. `run_once` reads this to call `report_published`.
    pub published: Option<String>,
}

#[derive(Debug)]
pub(crate) enum WatchError {
    Conflict(PausedReason),
    LoginRequired(Option<Host>),
    Transient(Error),
}

// String-matches the guard messages in `quilt-rs/src/flow/pull.rs` and
// `quilt-rs/src/flow/push.rs`. Open question in the plan: replace with
// typed `PullRefusal` / `PushRefusal` enums upstream.
//
// Policy:
// - Known pull-side refusals (`PackageOpError::Package`) keep their
//   specific `PausedReason`.
// - `Push` / `Commit` / `Publish` variants almost always reflect
//   user-actionable trouble (workflow rejected, hash mismatch, ...),
//   so we pause with `Other(_)` carrying the message.
// - HTTP / IO / S3 — including the AWS SDK `S3Error` family —
//   are `Transient` (retry with backoff). S3 is a peer variant of
//   `PackageOp` on `quilt::Error`, not nested inside it: `PutObject`,
//   `UploadFile`, throttling, 5xx, and the like all propagate as
//   `Error::S3(_)` straight through the publish flow. Treating them
//   as `Other(_)` would permanently pause the namespace on a single
//   network blip — exactly the wrong shape for autopush. Truly
//   permanent S3 sub-kinds (`NotFound`, `PermissionDenied`-like) are
//   either caught upstream (`Error::is_not_found` in `flow::push`) or
//   accepted as "retry every 64 s until the user fixes it" via the
//   capped backoff — annoying but not catastrophic, and far better
//   than silently pausing on every transient blip.
// - Everything else lands in `Other(_)` — the new default arm flips the
//   bias from "keep trying quietly" to "stop and surface."
pub(crate) fn classify_sync_err(err: Error) -> Result<(), WatchError> {
    match &err {
        Error::Quilt(quilt::Error::PackageOp(quilt::PackageOpError::Package(msg))) => {
            if msg == "package has pending changes" {
                Err(WatchError::Conflict(PausedReason::PendingChanges))
            } else if msg == "package has pending commits" {
                Err(WatchError::Conflict(PausedReason::PendingCommit))
            } else if msg == "package has diverged" {
                Err(WatchError::Conflict(PausedReason::Diverged))
            } else if msg == "package is already up-to-date" {
                Ok(())
            } else {
                Err(WatchError::Conflict(PausedReason::Other(msg.clone())))
            }
        }
        Error::Quilt(quilt::Error::PackageOp(
            quilt::PackageOpError::Push(msg)
            | quilt::PackageOpError::Commit(msg)
            | quilt::PackageOpError::Publish(msg),
        )) => Err(WatchError::Conflict(PausedReason::Other(msg.clone()))),
        Error::Quilt(quilt::Error::Reqwest(_) | quilt::Error::Io(_) | quilt::Error::S3(_)) => {
            Err(WatchError::Transient(err))
        }
        Error::Quilt(quilt::Error::Login(quilt::LoginError::Required(host))) => {
            Err(WatchError::LoginRequired(host.clone()))
        }
        _ => Err(WatchError::Conflict(PausedReason::Other(err.to_string()))),
    }
}

pub(crate) async fn refresh_then_maybe_sync(
    model: &impl QuiltModel,
    namespace: &Namespace,
    lineage: &quilt::lineage::PackageLineage,
    publish: &PublishSettings,
    quiet_window: Duration,
    pull_enabled: bool,
    push_enabled: bool,
) -> Result<RefreshOutcome, WatchError> {
    let installed = model
        .get_installed_package(namespace)
        .await
        .map_err(WatchError::Transient)?
        .ok_or_else(|| {
            WatchError::Transient(Error::from(quilt::InstallPackageError::NotInstalled(
                namespace.clone(),
            )))
        })?;

    // `status` does the cheap tag refresh; an expired token surfaces here.
    let status = model
        .get_installed_package_status(&installed, None)
        .await
        .map_err(|err| match &err {
            Error::Quilt(quilt::Error::Login(quilt::LoginError::Required(host))) => {
                WatchError::LoginRequired(host.clone())
            }
            _ => WatchError::Transient(err),
        })?;
    let upstream = status.upstream_state;
    let has_changes = !status.changes.is_empty();
    // `lineage` comes from `run_once`'s skip-filter read — re-reading it
    // here would cost an extra trait call per tick and open a narrow
    // race window where a commit landing between the two reads would
    // make `has_pending_commit` stale relative to `upstream`.
    let has_pending_commit = lineage.commit.is_some();

    // A `Diverged` state needs explicit user action (Certify Latest or
    // Reset Local). Neither the pull nor the publish branch would touch
    // it, but leaving it as an `Ok(_)` outcome means we'd re-emit
    // `diverged` every tick without pausing — the UI then looks healthy
    // even though no progress can be made. Surface as a Conflict so the
    // namespace lands in the paused set on the first observation.
    if upstream == quilt::lineage::UpstreamState::Diverged {
        return Err(WatchError::Conflict(PausedReason::Diverged));
    }

    // Pull branch.
    //
    // The `!has_pending_commit` clause is **defensive**, not load-bearing:
    // under the new `From<PackageLineage> for UpstreamState`, a package
    // with a pending commit and a stale `latest_hash` lands in `Diverged`,
    // not `Behind`, so this code path is unreachable in practice. We keep
    // the clause to make mutual exclusivity with the publish branch
    // explicit at the call site — if the `From` conversion ever changes
    // shape, this gate is what stops a pull and a publish from racing on
    // the same package in the same tick.
    if pull_enabled
        && upstream == quilt::lineage::UpstreamState::Behind
        && !has_changes
        && !has_pending_commit
    {
        return match model.package_pull(&installed, None).await {
            Ok(_) => {
                info!("autosync: pulled namespace={namespace}");
                Ok(RefreshOutcome {
                    upstream: quilt::lineage::UpstreamState::UpToDate,
                    has_changes: false,
                    published: None,
                })
            }
            Err(err) => classify_sync_err(err).map(|()| RefreshOutcome {
                upstream,
                has_changes,
                published: None,
            }),
        };
    }

    // Publish branch.
    let publish_eligible = push_enabled
        && matches!(
            upstream,
            quilt::lineage::UpstreamState::UpToDate
                | quilt::lineage::UpstreamState::Ahead
                | quilt::lineage::UpstreamState::Local,
        )
        && (has_changes || has_pending_commit);
    if publish_eligible {
        let now = SystemTime::now();
        if !status.working_tree_quiet(now, quiet_window) {
            info!("autosync: namespace={namespace} working tree not quiet, deferring");
            return Ok(RefreshOutcome {
                upstream,
                has_changes,
                published: None,
            });
        }
        // `publish_with_settings` is shared with the manual Commit &
        // Push command in `commands.rs`, so a change to publish
        // settings (new placeholder, new field) applies identically
        // regardless of who triggered the publish.
        return match model::publish_with_settings(model, namespace, publish, status).await {
            Ok((_, message)) => {
                info!("autosync: published namespace={namespace}");
                Ok(RefreshOutcome {
                    upstream: quilt::lineage::UpstreamState::UpToDate,
                    has_changes: false,
                    published: Some(message),
                })
            }
            Err(err) => classify_sync_err(err).map(|()| RefreshOutcome {
                upstream,
                has_changes,
                published: None,
            }),
        };
    }

    Ok(RefreshOutcome {
        upstream,
        has_changes,
        published: None,
    })
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct BackoffState {
    pub next_attempt: Instant,
    pub consecutive_failures: u32,
}

// 2, 4, 8, 16, 32, 64 s, then capped.
pub(crate) fn backoff_duration(failures: u32) -> Duration {
    let exp = failures.min(6);
    Duration::from_secs(1u64 << exp)
}

fn is_backoff_due(
    backoff: &BTreeMap<Namespace, BackoffState>,
    namespace: &Namespace,
    now: Instant,
) -> bool {
    backoff.get(namespace).is_none_or(|b| now >= b.next_attempt)
}

fn bump_backoff(
    backoff: &mut BTreeMap<Namespace, BackoffState>,
    namespace: &Namespace,
    now: Instant,
) {
    let entry = backoff.entry(namespace.clone()).or_insert(BackoffState {
        next_attempt: now,
        consecutive_failures: 0,
    });
    entry.consecutive_failures = entry.consecutive_failures.saturating_add(1);
    entry.next_attempt = now + backoff_duration(entry.consecutive_failures);
}

pub(crate) async fn run_once(model: &impl QuiltModel, inner: &WatcherInner) -> Result<(), Error> {
    // Cheap pre-check: if both directions are off we have nothing to
    // do. Per-direction gating lives inside `refresh_then_maybe_sync`
    // so a single-direction config (pull only / push only) still
    // exercises the cheap status refresh and the skip rules.
    let (pull_enabled, push_enabled) = {
        let settings = inner.settings.read().await;
        (settings.pull.enabled, settings.push.enabled)
    };
    if !pull_enabled && !push_enabled {
        return Ok(());
    }

    let packages = model.get_installed_packages_list().await?;
    let current: BTreeSet<Namespace> = packages.iter().map(|p| p.namespace.clone()).collect();
    inner
        .paused
        .write()
        .await
        .retain(|ns, _| current.contains(ns));
    inner
        .backoff
        .write()
        .await
        .retain(|ns, _| current.contains(ns));
    inner
        .login_blocked
        .write()
        .await
        .retain(|ns, _| current.contains(ns));
    inner.aggregator.retain_namespaces(&current);

    // Snapshot publish settings once per tick so we don't reacquire the
    // RwLock per package. Same lifetime for `quiet_window`.
    //
    // `quiet_window` is the constant `push.idle_timeout_secs`. It does
    // not depend on window mode anymore — that coupling was the bug.
    // The sleep loop in `Watcher::spawn` still reads
    // `cadence_for_mode(&settings.pull, mode)`, so pull frequency and
    // push quiet window can be tuned independently.
    let publish = inner.publish_settings.read().await.clone();
    let quiet_window = {
        let settings = inner.settings.read().await;
        Duration::from_secs(settings.push.idle_timeout_secs)
    };

    let now = Instant::now();
    for pkg in packages {
        let namespace = pkg.namespace.clone();

        // Skip Local / misconfigured packages without a network round-trip.
        let lineage = match model.get_installed_package_lineage(&pkg).await {
            Ok(l) => l,
            Err(err) => {
                warn!("autosync: lineage read failed for {namespace}: {err}");
                continue;
            }
        };
        let Some(remote) = lineage.remote_uri.as_ref() else {
            continue;
        };
        if remote.origin.is_none() || remote.bucket.is_empty() {
            continue;
        }

        if inner.paused.read().await.contains_key(&namespace) {
            continue;
        }
        if !is_backoff_due(&*inner.backoff.read().await, &namespace, now) {
            continue;
        }

        match refresh_then_maybe_sync(
            model,
            &namespace,
            &lineage,
            &publish,
            quiet_window,
            pull_enabled,
            push_enabled,
        )
        .await
        {
            Ok(outcome) => {
                inner.backoff.write().await.remove(&namespace);
                inner.login_blocked.write().await.remove(&namespace);
                if let Some(message) = outcome.published.as_deref() {
                    inner.reporter.report_published(&namespace, message);
                }
                inner.reporter.report_status(
                    &namespace,
                    PackageStatusEvent {
                        namespace: namespace.to_string(),
                        status: outcome.upstream.to_string(),
                        has_changes: outcome.has_changes,
                    },
                );
                inner.aggregator.clear_error(&namespace);
                inner
                    .aggregator
                    .note_status(&namespace, outcome.has_changes);
            }
            Err(WatchError::LoginRequired(host)) => {
                // Backoff until the user re-auths; the Ok arm clears it.
                bump_backoff(&mut *inner.backoff.write().await, &namespace, now);
                inner
                    .login_blocked
                    .write()
                    .await
                    .insert(namespace.clone(), host.clone());
                inner.reporter.report_login_required(host.as_ref());
                inner.aggregator.note_login_required(&namespace, host);
            }
            Err(WatchError::Conflict(reason)) => {
                inner
                    .paused
                    .write()
                    .await
                    .insert(namespace.clone(), reason.clone());
                inner.reporter.report_paused(&namespace, reason.clone());
                // Heuristic status from the refusal reason — flow::pull /
                // flow::publish don't expose the post-attempt state
                // directly. The string `"error"` is **reserved** for "we
                // couldn't talk to the remote" — the UI renders a Login
                // affordance on that one. Surface autosync refusals as
                // `"paused"` so the row banner is neutral, and let the UI
                // pull the message out of the `autosync-paused` event the
                // reporter emits.
                let (status, has_changes) = match reason {
                    PausedReason::PendingChanges => ("behind", true),
                    PausedReason::PendingCommit => ("ahead", false),
                    PausedReason::Diverged => ("diverged", false),
                    PausedReason::Other(ref msg) => {
                        warn!("autosync: paused namespace={namespace} error={msg}");
                        ("paused", false)
                    }
                };
                inner.reporter.report_status(
                    &namespace,
                    PackageStatusEvent {
                        namespace: namespace.to_string(),
                        status: status.to_string(),
                        has_changes,
                    },
                );
                let aggregator_message = match &reason {
                    PausedReason::PendingChanges => "pending changes",
                    PausedReason::PendingCommit => "pending commits",
                    PausedReason::Diverged => "diverged",
                    PausedReason::Other(msg) => msg.as_str(),
                };
                inner.aggregator.note_paused(&namespace, aggregator_message);
                inner.aggregator.note_status(&namespace, has_changes);
            }
            Err(WatchError::Transient(err)) => {
                bump_backoff(&mut *inner.backoff.write().await, &namespace, now);
                warn!("autosync: transient error for {namespace}: {err}");
                // Transient: don't touch the aggregator's error map — a
                // network blip should not flip the tray to Error. The
                // next tick either clears or escalates.
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
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

    fn test_aggregator() -> Arc<crate::autopull::status::SyncTrayAggregator> {
        let (tx, _) = tokio::sync::watch::channel(
            crate::autopull::status::SyncTrayStatus::default(),
        );
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

    // ── publish branch (autosync M1) ────────────────────────────────────

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
        // `model::package_publish` (free fn) now routes the workflow
        // lookup through the trait. Default to "no workflow" — autosync
        // M1 ignores `default_workflow` in publish settings; the per-
        // remote workflow gets enforced by `flow::publish_package` from
        // the remote-side config regardless.
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
        let lineage =
            quilt::lineage::PackageLineage::from_remote(remote_for(&ns), "h0".to_string());

        let (mut model, _) = fixture_with_lineage_and_status(
            lineage,
            quiet_status(UpstreamState::UpToDate, changes),
        );
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
        let status =
            quilt::lineage::InstalledPackageStatus::new(UpstreamState::Ahead, BTreeMap::new());

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
        let mut status =
            quilt::lineage::InstalledPackageStatus::new(UpstreamState::UpToDate, changes);
        // Just edited → not quiet (focused cadence is 30 s by default).
        status.most_recent_mtime = Some(SystemTime::now());

        let lineage =
            quilt::lineage::PackageLineage::from_remote(remote_for(&ns), "h0".to_string());
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
        let lineage =
            quilt::lineage::PackageLineage::from_remote(remote_for(&ns), "h1".to_string());

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
        let lineage =
            quilt::lineage::PackageLineage::from_remote(remote_for(&ns), "h0".to_string());

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
        let lineage =
            quilt::lineage::PackageLineage::from_remote(remote_for(&ns), "h0".to_string());

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
        let lineage =
            quilt::lineage::PackageLineage::from_remote(remote_for(&ns), "h0".to_string());

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
        let mut status =
            quilt::lineage::InstalledPackageStatus::new(UpstreamState::UpToDate, changes);
        status.most_recent_mtime = Some(SystemTime::now() - Duration::from_secs(30));

        let lineage =
            quilt::lineage::PackageLineage::from_remote(remote_for(&ns), "h0".to_string());
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
        let lineage =
            quilt::lineage::PackageLineage::from_remote(remote_for(&ns), "h0".to_string());
        let (mut model, _) = fixture_with_lineage_and_status(
            lineage,
            quiet_status(UpstreamState::UpToDate, changes),
        );
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
        let lineage =
            quilt::lineage::PackageLineage::from_remote(remote_for(&ns), "h1".to_string());
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
        let lineage =
            quilt::lineage::PackageLineage::from_remote(remote_for(&ns), "h0".to_string());
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
        let (tx, rx) =
            tokio::sync::watch::channel(crate::autopull::status::SyncTrayStatus::default());
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
        let lineage =
            quilt::lineage::PackageLineage::from_remote(remote_for(&ns), "h1".to_string());
        let (model, _) = fixture_with_lineage_and_status(
            lineage,
            quiet_status(UpstreamState::Behind, changes),
        );
        let reporter = Arc::new(RecordingReporter::default());
        let (tx, rx) =
            tokio::sync::watch::channel(crate::autopull::status::SyncTrayStatus::default());
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
}
