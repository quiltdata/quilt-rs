use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fmt::Write;
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
use crate::quilt;
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
    /// Per-namespace fingerprint of the last emitted status. Two recomputes
    /// that produce the same fingerprint emit only the first event — this
    /// is how the watcher avoids feeding back on itself when the backend's
    /// own reads (e.g. `flow::status` walking the working tree to compute
    /// the next status) trigger inotify `OPEN`/`ATTRIB` events.
    pub previous_fingerprints: BTreeMap<Namespace, String>,
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
                reconcile_from_model(&mut state, &app_handle).await;
            }
            Some(signal) = state.signal_rx.recv() => {
                if !state.settings.read().await.enabled {
                    continue;
                }
                let model = app_handle.state::<Model>();
                process_signal(
                    &*model,
                    state.reporter.as_ref(),
                    &mut state.previous_fingerprints,
                    signal,
                )
                .await;
            }
            else => break,
        }
    }
}

pub(crate) async fn process_signal(
    model: &impl QuiltModel,
    reporter: &dyn StatusReporter,
    previous_fingerprints: &mut BTreeMap<Namespace, String>,
    signal: MappingSignal,
) {
    let pkg = match model.get_installed_package(&signal.namespace).await {
        Ok(Some(pkg)) => pkg,
        Ok(None) => {
            // already uninstalled; drop any stale fingerprint
            previous_fingerprints.remove(&signal.namespace);
            return;
        }
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
    let fingerprint = status_fingerprint(&status);
    if previous_fingerprints.get(&signal.namespace) == Some(&fingerprint) {
        // Same fingerprint as last emit — spurious wake (e.g. inotify
        // `OPEN` event from the UI reading the working tree). Skip.
        return;
    }
    previous_fingerprints.insert(signal.namespace.clone(), fingerprint);
    let event = PackageStatusEvent {
        namespace: signal.namespace.to_string(),
        status: status.upstream_state.to_string(),
        has_changes: !status.changes.is_empty(),
    };
    reporter.report_status(&signal.namespace, event);
}

/// Stable digest of the parts of `InstalledPackageStatus` that the UI
/// renders. Two recompute results with the same fingerprint produce the
/// same UI, so the second one need not be re-emitted.
///
/// Format: `<upstream>;<path>:<kind>:<hash>;<path>:<kind>:<hash>;...`
/// Paths are walked in `BTreeMap` order (sorted by `PathBuf`), so the
/// output is deterministic without an explicit sort.
fn status_fingerprint(status: &quilt::lineage::InstalledPackageStatus) -> String {
    let mut out = String::new();
    let _ = write!(out, "{};", status.upstream_state);
    for (path, change) in &status.changes {
        let (kind, row) = match change {
            quilt::lineage::Change::Added(r) => ("A", r),
            quilt::lineage::Change::Modified(r) => ("M", r),
            quilt::lineage::Change::Removed(r) => ("D", r),
        };
        let _ = write!(out, "{}:{}:{};", path.to_string_lossy(), kind, row.hash,);
    }
    out
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
    let kept: BTreeSet<Namespace> = mappings.iter().map(|(ns, _)| ns.clone()).collect();
    state
        .previous_fingerprints
        .retain(|ns, _| kept.contains(ns));
    if let Err(err) = state.subscription.reconcile(mappings) {
        emit_subscriber_error(state.reporter.as_ref(), &err);
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

    use crate::autopull::reporter::test_support::RecordingReporter;
    use crate::model::MockQuiltModel;

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
        let mut prev = BTreeMap::new();

        process_signal(
            &model,
            reporter.as_ref(),
            &mut prev,
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
        let mut prev = BTreeMap::new();

        process_signal(
            &model,
            reporter.as_ref(),
            &mut prev,
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
    async fn identical_recompute_after_first_emit_is_suppressed() {
        // Two back-to-back signals where the recomputed status is byte-for-byte
        // identical (the second one came from a spurious wake — e.g. an inotify
        // OPEN event fired by the UI re-reading the working tree). The reactor
        // must emit exactly once.
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
        let mut prev = BTreeMap::new();
        let signal = MappingSignal {
            namespace: ns.clone(),
        };

        process_signal(&model, reporter.as_ref(), &mut prev, signal.clone()).await;
        process_signal(&model, reporter.as_ref(), &mut prev, signal).await;

        let statuses = reporter.statuses.lock().unwrap();
        assert_eq!(statuses.len(), 1, "second recompute should be suppressed");
        assert_eq!(statuses[0].0, ns);
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
        let mut prev = BTreeMap::new();
        let signal = MappingSignal {
            namespace: ns.clone(),
        };

        process_signal(&model, reporter.as_ref(), &mut prev, signal.clone()).await;
        process_signal(&model, reporter.as_ref(), &mut prev, signal).await;

        assert_eq!(reporter.statuses.lock().unwrap().len(), 2);
    }
}
