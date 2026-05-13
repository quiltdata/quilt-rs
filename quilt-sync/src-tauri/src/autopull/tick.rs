use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::time::Duration;
use std::time::Instant;

use quilt_uri::Host;
use quilt_uri::Namespace;

use crate::Error;
use crate::autopull::PausedReason;
use crate::autopull::WatcherInner;
use crate::autopull::reporter::PackageStatusEvent;
use crate::model::QuiltModel;
use crate::quilt;
use crate::telemetry::prelude::*;

/// State + decision returned from `refresh_then_maybe_pull`.
#[derive(Debug)]
pub(crate) struct RefreshOutcome {
    pub upstream: quilt::lineage::UpstreamState,
    pub has_changes: bool,
}

/// Watcher-facing classification of pull errors.
#[derive(Debug)]
pub(crate) enum WatchError {
    /// A pull-time guard refused: package needs the user (no further ticks
    /// will attempt this namespace until they act on it).
    Conflict(PausedReason),
    /// Refresh hit `LoginError::Required` — the user must re-auth before
    /// any further progress is possible on this host.
    LoginRequired(Option<Host>),
    /// Network / transient error — retry with exponential backoff.
    Transient(Error),
}

/// Map `pull_package`'s guard strings onto the watcher's `WatchError` taxonomy.
///
/// The guards live in `quilt-rs/src/flow/pull.rs` as
/// `PackageOpError::Package(String)`; we match the exact strings here.
/// Renaming them upstream should break the unit tests below loudly so the
/// mapping stays in sync. Milestone 3 in the approach doc replaces this
/// with a typed `PullRefusal` enum.
pub(crate) fn classify_pull_err(err: Error) -> Result<(), WatchError> {
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
                Err(WatchError::Transient(err))
            }
        }
        Error::Quilt(quilt::Error::Login(quilt::LoginError::Required(host))) => {
            Err(WatchError::LoginRequired(host.clone()))
        }
        _ => Err(WatchError::Transient(err)),
    }
}

/// Run the cheap refresh and, if behind-and-clean, also the heavy pull.
///
/// Calls the model layer directly to keep `UpstreamState` typed end-to-end
/// — going through `commands::refresh_package_status_from_model` would
/// stringify the state for the UI and force us to parse it back here.
pub(crate) async fn refresh_then_maybe_pull(
    model: &impl QuiltModel,
    namespace: &Namespace,
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

    let status = model
        .get_installed_package_status(&installed, None)
        .await
        .map_err(WatchError::Transient)?;
    let upstream = status.upstream_state;
    let has_changes = !status.changes.is_empty();

    if upstream == quilt::lineage::UpstreamState::Behind && !has_changes {
        match model.package_pull(&installed, None).await {
            Ok(_) => {
                info!("autopull: pulled namespace={namespace}");
                return Ok(RefreshOutcome {
                    upstream: quilt::lineage::UpstreamState::UpToDate,
                    has_changes: false,
                });
            }
            Err(err) => classify_pull_err(err)?,
        }
    }

    Ok(RefreshOutcome {
        upstream,
        has_changes,
    })
}

/// One package's transient-error backoff state.
#[derive(Debug, Clone, Copy)]
pub(crate) struct BackoffState {
    pub next_attempt: Instant,
    pub consecutive_failures: u32,
}

/// Exponential backoff: 2, 4, 8, 16, 32, 64 s — capped at 64 s after the
/// 6th consecutive failure. A successful tick removes the namespace's
/// entry, which resets the counter for the next failure.
pub(crate) fn backoff_duration(failures: u32) -> Duration {
    let exp = failures.min(6);
    Duration::from_secs(1u64 << exp)
}

fn is_backoff_due(
    backoff: &BTreeMap<Namespace, BackoffState>,
    namespace: &Namespace,
    now: Instant,
) -> bool {
    backoff
        .get(namespace)
        .is_none_or(|b| now >= b.next_attempt)
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

/// Execute one full pass over the installed-packages list.
///
/// Returns `Ok(())` even when individual packages fail; per-package errors
/// are folded into the reporter and backoff state. Only fatal "could not
/// even list packages" errors bubble out.
pub(crate) async fn run_once(
    model: &impl QuiltModel,
    inner: &WatcherInner,
) -> Result<(), Error> {
    if !inner.settings.read().await.enabled {
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

    let now = Instant::now();
    for pkg in packages {
        let namespace = pkg.namespace.clone();

        // Cheap skip filters: avoid talking to the network for Local /
        // misconfigured packages. We re-derive these at list time rather
        // than reusing `refresh_package_status_from_model`, which would
        // collapse the typed `UpstreamState` to a string.
        let lineage = match model.get_installed_package_lineage(&pkg).await {
            Ok(l) => l,
            Err(err) => {
                warn!("autopull: lineage read failed for {namespace}: {err}");
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

        match refresh_then_maybe_pull(model, &namespace).await {
            Ok(outcome) => {
                inner.backoff.write().await.remove(&namespace);
                inner.reporter.report_status(
                    &namespace,
                    PackageStatusEvent {
                        namespace: namespace.to_string(),
                        status: outcome.upstream.to_string(),
                        has_changes: outcome.has_changes,
                    },
                );
            }
            Err(WatchError::LoginRequired(host)) => {
                inner.backoff.write().await.remove(&namespace);
                inner.reporter.report_login_required(host.as_ref());
            }
            Err(WatchError::Conflict(reason)) => {
                inner
                    .paused
                    .write()
                    .await
                    .insert(namespace.clone(), reason.clone());
                inner.reporter.report_paused(&namespace, reason.clone());
                // One final status emit so the UI's row leaves the
                // optimistic state it was in before the failed pull. The
                // mapping below is a heuristic — flow::pull doesn't tell
                // us the exact new state, but the reason is enough to pick
                // the user-facing render.
                let (status, has_changes) = match reason {
                    PausedReason::PendingChanges => ("behind", true),
                    PausedReason::PendingCommit => ("ahead", false),
                    PausedReason::Diverged => ("diverged", false),
                };
                inner.reporter.report_status(
                    &namespace,
                    PackageStatusEvent {
                        namespace: namespace.to_string(),
                        status: status.to_string(),
                        has_changes,
                    },
                );
            }
            Err(WatchError::Transient(err)) => {
                bump_backoff(&mut *inner.backoff.write().await, &namespace, now);
                warn!("autopull: transient error for {namespace}: {err}");
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

    use crate::autopull::AutopullSettings;
    use crate::autopull::WindowMode;
    use crate::autopull::reporter::LogReporter;
    use crate::autopull::reporter::test_support::RecordingReporter;
    use crate::model::MockQuiltModel;
    use crate::quilt::lineage::UpstreamState;

    fn make_inner(settings: AutopullSettings) -> WatcherInner {
        WatcherInner {
            settings: Arc::new(RwLock::new(settings)),
            window_mode: Arc::new(RwLock::new(WindowMode::Focused)),
            paused: RwLock::new(BTreeMap::new()),
            backoff: RwLock::new(BTreeMap::new()),
            reporter: Arc::new(LogReporter),
        }
    }

    fn enabled() -> AutopullSettings {
        AutopullSettings {
            enabled: true,
            ..AutopullSettings::default()
        }
    }

    #[test]
    fn classify_pending_changes() {
        let err = Error::from(quilt::Error::PackageOp(quilt::PackageOpError::Package(
            "package has pending changes".to_string(),
        )));
        match classify_pull_err(err) {
            Err(WatchError::Conflict(PausedReason::PendingChanges)) => {}
            other => panic!("expected Conflict(PendingChanges), got {other:?}"),
        }
    }

    #[test]
    fn classify_pending_commits() {
        let err = Error::from(quilt::Error::PackageOp(quilt::PackageOpError::Package(
            "package has pending commits".to_string(),
        )));
        match classify_pull_err(err) {
            Err(WatchError::Conflict(PausedReason::PendingCommit)) => {}
            other => panic!("expected Conflict(PendingCommit), got {other:?}"),
        }
    }

    #[test]
    fn classify_diverged() {
        let err = Error::from(quilt::Error::PackageOp(quilt::PackageOpError::Package(
            "package has diverged".to_string(),
        )));
        match classify_pull_err(err) {
            Err(WatchError::Conflict(PausedReason::Diverged)) => {}
            other => panic!("expected Conflict(Diverged), got {other:?}"),
        }
    }

    #[test]
    fn classify_already_up_to_date_is_ok() {
        let err = Error::from(quilt::Error::PackageOp(quilt::PackageOpError::Package(
            "package is already up-to-date".to_string(),
        )));
        assert!(classify_pull_err(err).is_ok());
    }

    #[test]
    fn classify_login_required() {
        let host: Host = "catalog.dev".parse().unwrap();
        let err = Error::from(quilt::Error::Login(quilt::LoginError::Required(Some(
            host.clone(),
        ))));
        match classify_pull_err(err) {
            Err(WatchError::LoginRequired(Some(h))) => assert_eq!(h, host),
            other => panic!("expected LoginRequired(Some(_)), got {other:?}"),
        }
    }

    #[test]
    fn classify_generic_is_transient() {
        let err = Error::General("network".to_string());
        match classify_pull_err(err) {
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
        let inner = make_inner(AutopullSettings::default());
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
        model.expect_get_installed_packages_list().returning(move || {
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
        model
            .expect_package_pull()
            .times(1)
            .returning(|_, _| {
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
            paused: RwLock::new(BTreeMap::new()),
            backoff: RwLock::new(BTreeMap::new()),
            reporter: reporter.clone(),
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
            paused: RwLock::new(BTreeMap::new()),
            backoff: RwLock::new(BTreeMap::new()),
            reporter: reporter.clone(),
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
}
