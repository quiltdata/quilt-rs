use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use quilt_uri::Namespace;
use tauri::Manager;
use tokio::sync::mpsc;

use crate::autopull::PackageStatusEvent;
use crate::autopull::StatusReporter;
use crate::autopull::reporter::SubscriberErrorEvent;
use crate::fswatcher::settings::SharedFsWatcherSettings;
use crate::fswatcher::subscriber::MappingSignal;
use crate::fswatcher::subscriber::SubscriberError;
use crate::fswatcher::subscriber::Subscription;
use crate::model::Model;
use crate::model::QuiltModel;
use crate::telemetry::prelude::*;

/// How often the reactor re-snapshots the installed-packages list and
/// reconciles its subscription set. Bounds how long a freshly-installed
/// package waits before its first edit fires an event, and how long a
/// just-uninstalled package keeps a (harmless) live watch.
const RECONCILE_INTERVAL: Duration = Duration::from_secs(5);

pub(crate) struct ReactorState {
    pub settings: SharedFsWatcherSettings,
    pub reporter: Arc<dyn StatusReporter>,
    pub signal_rx: mpsc::Receiver<MappingSignal>,
    pub subscription: Subscription,
    /// Kind of the last `Err` returned by `reconcile`. A failed `add()`
    /// doesn't insert into the subscription's watched set, so the next
    /// reconcile retries the same namespaces and fails the same way; this
    /// marker keeps us from emitting a fresh `inotify_limit` toast every
    /// 5 s. Cleared whenever a reconcile returns `Ok`.
    pub last_reconcile_error_kind: Option<&'static str>,
}

pub(crate) async fn run(mut state: ReactorState, app_handle: tauri::AppHandle) {
    let mut reconcile_tick = tokio::time::interval(RECONCILE_INTERVAL);
    // The first tick fires immediately; drop it because `FsWatcher::spawn`
    // already did a synchronous initial reconcile, so the next periodic
    // reconcile should run after a full interval.
    reconcile_tick.tick().await;
    loop {
        tokio::select! {
            biased;
            _ = reconcile_tick.tick() => {
                let enabled = state.settings.read().await.enabled;
                if enabled {
                    reconcile_from_model(&mut state, &app_handle).await;
                } else {
                    // Disabled: drain any live watches so the OS releases
                    // the inotify slots. Especially important when the user
                    // toggles off in response to the inotify-limit toast.
                    // `reconcile(Vec::new())` is a no-op if already empty.
                    if let Err(err) = state.subscription.reconcile(Vec::new()) {
                        emit_subscriber_error(state.reporter.as_ref(), &err);
                    }
                    // Re-enabling later should be able to surface the
                    // inotify-limit toast again if the limit still applies.
                    state.last_reconcile_error_kind = None;
                }
            }
            Some(signal) = state.signal_rx.recv() => {
                if !state.settings.read().await.enabled {
                    continue;
                }
                let model = app_handle.state::<Model>();
                process_signal(&*model, state.reporter.as_ref(), signal).await;
            }
            else => break,
        }
    }
}

pub(crate) async fn process_signal(
    model: &impl QuiltModel,
    reporter: &dyn StatusReporter,
    signal: MappingSignal,
) {
    let pkg = match model.get_installed_package(&signal.namespace).await {
        Ok(Some(pkg)) => pkg,
        Ok(None) => return, // already uninstalled
        Err(err) => {
            warn!(
                "fswatcher: get_installed_package for {} failed: {err}",
                signal.namespace
            );
            return;
        }
    };
    let status = match model.recompute_local_status(&pkg, None).await {
        Ok(s) => s,
        Err(err) => {
            warn!(
                "fswatcher: recompute_local_status failed for {}: {err}",
                signal.namespace
            );
            return;
        }
    };
    // Emit on every signal — including a spurious wake that recomputes an
    // identical status. The event carries a fingerprint of the observation,
    // and the *consumer* skips a repeat. A producer-side suppression map here
    // was reactor-only (the autopull tick never consulted it), so it could
    // strand the page on a stale view after a tick moved it — stale-and-quiet,
    // with no recovery.
    reporter.report_status(
        &signal.namespace,
        PackageStatusEvent::from_status(&signal.namespace, &status),
    );
}

/// Snapshot the current installed-packages list and resolve each one's
/// `package_home`. Returns `None` if the model itself fails (so the caller
/// skips reconciliation rather than unwatching every namespace on a
/// transient error); an empty `Some(_)` is distinct and means "no
/// installed packages".
pub(crate) async fn snapshot_mappings(
    model: &impl QuiltModel,
) -> Option<Vec<(Namespace, PathBuf)>> {
    let pkgs = match model.get_installed_packages_list().await {
        Ok(list) => list,
        Err(err) => {
            warn!("fswatcher: snapshot failed: {err}");
            return None;
        }
    };
    let mut out = Vec::with_capacity(pkgs.len());
    for pkg in &pkgs {
        match pkg.package_home().await {
            Ok(home) => out.push((pkg.namespace.clone(), home)),
            Err(err) => warn!(
                "fswatcher: cannot resolve package_home for {}: {err}",
                pkg.namespace
            ),
        }
    }
    Some(out)
}

async fn reconcile_from_model(state: &mut ReactorState, app_handle: &tauri::AppHandle) {
    let model = app_handle.state::<Model>();
    let Some(mappings) = snapshot_mappings(&*model).await else {
        return;
    };
    match state.subscription.reconcile(mappings) {
        Ok(()) => {
            state.last_reconcile_error_kind = None;
        }
        Err(err) => {
            let kind = err.kind_str();
            if state.last_reconcile_error_kind != Some(kind) {
                emit_subscriber_error(state.reporter.as_ref(), &err);
                state.last_reconcile_error_kind = Some(kind);
            }
        }
    }
}

pub(crate) fn emit_subscriber_error(reporter: &dyn StatusReporter, err: &SubscriberError) {
    let event = SubscriberErrorEvent {
        kind: err.kind_str().to_string(),
        message: err.message(),
        namespace: err.namespace().map(ToString::to_string),
    };
    reporter.report_subscriber_error(event);
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::BTreeMap;

    use crate::autopull::reporter::test_support::RecordingReporter;
    use crate::model::MockQuiltModel;
    use crate::quilt;

    fn changes_with_one_file(kind: &'static str) -> BTreeMap<PathBuf, quilt::lineage::Change> {
        let mut changes = BTreeMap::new();
        let row = quilt::manifest::ManifestRow::default();
        let change = match kind {
            "added" => quilt::lineage::Change::Added(row),
            "modified" => quilt::lineage::Change::Modified(row),
            _ => quilt::lineage::Change::Removed(row),
        };
        changes.insert(PathBuf::from("file.txt"), change);
        changes
    }

    fn fresh_pkg() -> quilt::InstalledPackage {
        quilt::LocalDomain::new(PathBuf::new())
            .create_installed_package(("acme", "demo").into())
            .unwrap()
    }

    #[tokio::test]
    async fn signal_for_unknown_namespace_is_dropped() {
        let mut model = MockQuiltModel::new();
        model.expect_get_installed_package().returning(|_| Ok(None));
        let reporter = Arc::new(RecordingReporter::default());

        process_signal(
            &model,
            reporter.as_ref(),
            MappingSignal {
                namespace: ("acme", "demo").into(),
            },
        )
        .await;
        assert!(reporter.statuses.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn signal_with_changes_emits_status_event() {
        let ns: Namespace = ("acme", "demo").into();
        let mut model = MockQuiltModel::new();
        model
            .expect_get_installed_package()
            .returning(|_| Ok(Some(fresh_pkg())));
        model.expect_recompute_local_status().return_once(|_, _| {
            Ok(quilt::lineage::InstalledPackageStatus::new(
                quilt::lineage::UpstreamState::UpToDate,
                changes_with_one_file("added"),
            ))
        });
        let reporter = Arc::new(RecordingReporter::default());

        process_signal(
            &model,
            reporter.as_ref(),
            MappingSignal {
                namespace: ns.clone(),
            },
        )
        .await;

        let statuses = reporter.statuses.lock().unwrap();
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].0, ns);
        assert!(statuses[0].1.has_changes);
        assert_eq!(statuses[0].1.status, "up_to_date");
    }

    #[tokio::test]
    async fn identical_recompute_after_first_emit_still_emits() {
        // Two back-to-back signals with a byte-for-byte identical recomputed
        // status (the second from a spurious wake — e.g. an inotify OPEN fired
        // by the UI re-reading the working tree). The reactor no longer holds a
        // suppression map: it emits both, each carrying the same fingerprint,
        // and the *consumer* skips the repeat. (A producer-side map would strand
        // the page after a tick moved it — stale-and-quiet with no recovery.)
        let ns: Namespace = ("acme", "demo").into();
        let mut model = MockQuiltModel::new();
        model
            .expect_get_installed_package()
            .times(2)
            .returning(|_| Ok(Some(fresh_pkg())));
        model
            .expect_recompute_local_status()
            .times(2)
            .returning(|_, _| {
                Ok(quilt::lineage::InstalledPackageStatus::new(
                    quilt::lineage::UpstreamState::UpToDate,
                    changes_with_one_file("added"),
                ))
            });
        let reporter = Arc::new(RecordingReporter::default());
        let signal = MappingSignal {
            namespace: ns.clone(),
        };

        process_signal(&model, reporter.as_ref(), signal.clone()).await;
        process_signal(&model, reporter.as_ref(), signal).await;

        let statuses = reporter.statuses.lock().unwrap();
        assert_eq!(
            statuses.len(),
            2,
            "both emit; the consumer dedups by fingerprint"
        );
        assert_eq!(statuses[0].1.fingerprint, statuses[1].1.fingerprint);
    }

    #[tokio::test]
    async fn changed_recompute_after_first_emit_emits_again() {
        // First recompute: file added. Second recompute: file modified
        // (different `kind` in the changeset). Different fingerprint → emit.
        let ns: Namespace = ("acme", "demo").into();
        let mut model = MockQuiltModel::new();
        model
            .expect_get_installed_package()
            .times(2)
            .returning(|_| Ok(Some(fresh_pkg())));
        let mut seq = mockall::Sequence::new();
        model
            .expect_recompute_local_status()
            .times(1)
            .in_sequence(&mut seq)
            .returning(|_, _| {
                Ok(quilt::lineage::InstalledPackageStatus::new(
                    quilt::lineage::UpstreamState::UpToDate,
                    changes_with_one_file("added"),
                ))
            });
        model
            .expect_recompute_local_status()
            .times(1)
            .in_sequence(&mut seq)
            .returning(|_, _| {
                Ok(quilt::lineage::InstalledPackageStatus::new(
                    quilt::lineage::UpstreamState::UpToDate,
                    changes_with_one_file("modified"),
                ))
            });
        let reporter = Arc::new(RecordingReporter::default());
        let signal = MappingSignal {
            namespace: ns.clone(),
        };

        process_signal(&model, reporter.as_ref(), signal.clone()).await;
        process_signal(&model, reporter.as_ref(), signal).await;

        assert_eq!(reporter.statuses.lock().unwrap().len(), 2);
    }
}
