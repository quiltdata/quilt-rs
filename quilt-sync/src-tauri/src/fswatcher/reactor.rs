use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

use quilt_uri::Namespace;
use tauri::Manager;
use tokio::sync::mpsc;

use crate::autopull::PackageStatusEvent;
use crate::autopull::SharedAutosyncSettings;
use crate::autopull::SharedWindowMode;
use crate::autopull::StatusReporter;
use crate::autopull::Watcher;
use crate::autopull::cadence_for_mode;
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
    /// Read for the pull cadence only: the signal bound is the tick's own
    /// cadence, not a second knob nobody would keep in step with it.
    pub autosync: SharedAutosyncSettings,
    pub window_mode: SharedWindowMode,
    pub reporter: Arc<dyn StatusReporter>,
    pub signal_rx: mpsc::Receiver<MappingSignal>,
    pub subscription: Subscription,
    /// When each namespace was last recomputed *on a signal*. Pruned by
    /// reconcile, so an uninstalled package does not linger here.
    pub last_signalled: BTreeMap<Namespace, Instant>,
    /// Kind of the last `Err` returned by `reconcile`. A failed `add()`
    /// doesn't insert into the subscription's watched set, so the next
    /// reconcile retries the same namespaces and fails the same way; this
    /// marker keeps us from emitting a fresh `inotify_limit` toast every
    /// 5 s. Cleared whenever a reconcile returns `Ok`.
    pub last_reconcile_error_kind: Option<&'static str>,
}

impl ReactorState {
    /// Whether this signal may spend a status walk, and record it if so.
    ///
    /// One walk per namespace per cadence window, the *first* signal in the
    /// window taking it — so a signal advances the next walk rather than adding
    /// one, and the walk count never scales with reported filesystem activity.
    ///
    /// Bounds only the walks the *watcher* causes: it does not coordinate with
    /// the tick's own walk, so a signal just after a tick still takes one — at
    /// most one extra per window, never one per event.
    async fn claim_signal(&mut self, namespace: &Namespace, now: Instant) -> bool {
        let cadence = {
            let settings = self.autosync.read().await;
            let mode = *self.window_mode.read().await;
            cadence_for_mode(&settings.pull, mode)
        };
        if let Some(last) = self.last_signalled.get(namespace)
            && now.duration_since(*last) < cadence
        {
            trace!(
                "fswatcher: coalescing signal for {namespace} (within {}s cadence)",
                cadence.as_secs(),
            );
            return false;
        }
        self.last_signalled.insert(namespace.clone(), now);
        true
    }
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
                if !state.claim_signal(&signal.namespace, Instant::now()).await {
                    continue;
                }
                let model = app_handle.state::<Model>();
                // The watcher's clocks, so an edit moves the publish countdown
                // on the same observation that moves the list, rather than
                // leaving it to the next tick — which is 30s focused but 120s
                // unfocused and 600s closed, and editing a file means the window
                // is not focused (qhq-8mgw.54).
                //
                // Behind `claim_signal` above, necessarily: the arm time is read
                // off the status walk, which is the thing that gate exists to
                // ration. So the countdown is no more live than the list — but
                // it is no LESS live either, and the two moving on one
                // observation is what qhq-8mgw.54 was actually about. A
                // countdown armed outside the gate would have to walk the tree
                // to do it, which is the gate undone.
                let watcher = app_handle.state::<Watcher>();
                process_signal(
                    &*model,
                    state.reporter.as_ref(),
                    // `State::inner()` unwraps the guard; `shared()` is the Watcher's own.
                    Some(watcher.inner().shared()),
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
    // The watcher's shared state, when there is one. `None` in tests that are
    // only about the event this emits.
    clocks: Option<&crate::autopull::WatcherInner>,
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

    // The same observation, told to the clock. The event above moves the list,
    // which measures the tree; without this the card's countdown still waited
    // for a tick, and the two disagreed on screen (qhq-8mgw.54).
    if let Some(inner) = clocks {
        crate::autopull::arm_publish_from_status(inner, &signal.namespace, &status).await;
    }
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
    // Reconcile is the one place that learns a package is gone, so it is where
    // the per-namespace signal clock is pruned. Left alone, the map would keep
    // an entry per package ever watched in the session.
    let live: std::collections::BTreeSet<Namespace> =
        mappings.iter().map(|(ns, _)| ns.clone()).collect();
    state.last_signalled.retain(|ns, _| live.contains(ns));
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

    /// A `ReactorState` whose only interesting parts are the cadence inputs and
    /// the signal clock.
    fn state_with_cadence(secs: u64) -> ReactorState {
        let reporter: Arc<dyn StatusReporter> = Arc::new(RecordingReporter::default());
        let (tx, rx) = mpsc::channel::<MappingSignal>(8);
        let subscription = Subscription::new(Duration::from_millis(50), tx, &reporter)
            .expect("debouncer should build");
        let mut autosync = crate::autopull::AutosyncSettings::default();
        autosync.pull.focused_secs = secs;
        ReactorState {
            settings: Arc::new(tokio::sync::RwLock::new(
                crate::fswatcher::FsWatcherSettings::default(),
            )),
            autosync: Arc::new(tokio::sync::RwLock::new(autosync)),
            window_mode: Arc::new(tokio::sync::RwLock::new(
                crate::autopull::WindowMode::Focused,
            )),
            reporter,
            signal_rx: rx,
            subscription,
            last_signalled: BTreeMap::new(),
            last_reconcile_error_kind: None,
        }
    }

    /// The rate bound: a signal buys at most one walk per namespace per cadence
    /// window, and the *first* one is served — so a signal advances the next
    /// walk rather than delaying this one.
    #[tokio::test]
    async fn a_signal_buys_one_walk_per_cadence_window() {
        let mut state = state_with_cadence(30);
        let ns: Namespace = ("acme", "demo").into();
        let t0 = Instant::now();

        assert!(
            state.claim_signal(&ns, t0).await,
            "the first signal in a window must be served immediately",
        );
        assert!(
            !state
                .claim_signal(&ns, t0 + Duration::from_millis(120))
                .await,
            "a second signal 120ms later must not buy a second walk — this is the \
             debouncer's flush interval, and answering each one is the defect",
        );
        assert!(
            !state.claim_signal(&ns, t0 + Duration::from_secs(29)).await,
            "still inside the window",
        );
        assert!(
            state.claim_signal(&ns, t0 + Duration::from_secs(30)).await,
            "a full cadence later the namespace is eligible again",
        );
    }

    /// The bound is per namespace: one busy package must not starve another.
    #[tokio::test]
    async fn the_bound_is_per_namespace() {
        let mut state = state_with_cadence(30);
        let busy: Namespace = ("acme", "busy").into();
        let other: Namespace = ("acme", "other").into();
        let t0 = Instant::now();

        assert!(state.claim_signal(&busy, t0).await);
        assert!(!state.claim_signal(&busy, t0 + Duration::from_secs(1)).await);
        assert!(
            state
                .claim_signal(&other, t0 + Duration::from_secs(1))
                .await,
            "a different package's first signal must not be charged to the busy one",
        );
    }

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
        quilt::LocalDomain::new(PathBuf::new()).create_installed_package(("acme", "demo").into())
    }

    #[tokio::test]
    async fn signal_for_unknown_namespace_is_dropped() {
        let mut model = MockQuiltModel::new();
        model.expect_get_installed_package().returning(|_| Ok(None));
        let reporter = Arc::new(RecordingReporter::default());

        process_signal(
            &model,
            reporter.as_ref(),
            None,
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
            None,
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

        process_signal(&model, reporter.as_ref(), None, signal.clone()).await;
        process_signal(&model, reporter.as_ref(), None, signal).await;

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

        process_signal(&model, reporter.as_ref(), None, signal.clone()).await;
        process_signal(&model, reporter.as_ref(), None, signal).await;

        assert_eq!(reporter.statuses.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn an_edit_moves_the_publish_countdown_without_waiting_for_a_tick() {
        // qhq-8mgw.54's wiring, which is the half that was missing rather than
        // the half that was wrong: `arm_publish_from_status` is tested on its
        // own in `autopull`, and the defect was that nothing called it from
        // here. The tick wrote the arm map alone, so the countdown lagged the
        // tree by a cadence — 120s while the window is unfocused, which is what
        // editing a file makes it.
        let ns: quilt_uri::Namespace = ("acme", "demo").into();
        let mut model = MockQuiltModel::new();
        model
            .expect_get_installed_package()
            .returning(|_| Ok(Some(fresh_pkg())));
        model.expect_recompute_local_status().returning(|_, _| {
            let mut status = quilt::lineage::InstalledPackageStatus::new(
                quilt::lineage::UpstreamState::UpToDate,
                changes_with_one_file("added"),
            );
            // Just edited, so the quiet window has not elapsed and there IS a
            // future moment to count to.
            status.most_recent_mtime = Some(std::time::SystemTime::now());
            Ok(status)
        });
        let reporter = Arc::new(RecordingReporter::default());
        let mut settings = crate::autopull::AutosyncSettings::default();
        settings.push.enabled = true;
        let inner = crate::autopull::inner_for_tests(settings);

        process_signal(
            &model,
            reporter.as_ref(),
            Some(&inner),
            MappingSignal {
                namespace: ns.clone(),
            },
        )
        .await;

        assert!(
            inner.clocks.publish_arm.read().await.contains_key(&ns),
            "the file watcher heard the edit and the clock did not"
        );
    }
}
