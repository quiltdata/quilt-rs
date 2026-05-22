use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use quilt_uri::Host;
use quilt_uri::Namespace;
use tauri::Manager;
use tokio::sync::RwLock;
use tokio::sync::watch;

use crate::autopull::status::SyncTrayAggregator;
use crate::model::Model;
use crate::publish_settings::SharedPublishSettings;
use crate::telemetry::prelude::*;

pub mod reporter;
pub mod settings;
pub mod status;
pub mod tick;

pub use reporter::PackageStatusEvent;
pub use reporter::StatusReporter;
pub use settings::AutosyncSettings;
pub use settings::PullSettings;
pub use settings::PushSettings;
pub use settings::SharedAutosyncSettings;
pub use settings::init as init_settings;
pub use status::SyncTrayStatus;
pub use status::TrayMode;

use tick::BackoffState;
use tick::run_once;

/// Where the OS thinks the main window is. The watcher reads this each
/// tick to pick a cadence. Default is `Focused` so the very first tick
/// uses the tightest cadence — the OS will overwrite this from window
/// events as soon as they arrive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowMode {
    Focused,
    Unfocused,
    /// Window has been closed but the app stays alive via the tray icon.
    /// Set when the user closes the main window with `close_to_tray` on.
    Closed,
}

pub type SharedWindowMode = Arc<RwLock<WindowMode>>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PausedReason {
    PendingChanges,
    PendingCommit,
    Diverged,
    /// Catch-all for non-transient errors we haven't enumerated:
    /// workflow validation failures, hash mismatches, remote
    /// configuration drift. The string travels to the UI in the
    /// `autosync-paused` event payload (see
    /// [`crate::autopull::reporter::PausedEvent`]) and is rendered in
    /// the per-package status banner so the user knows what to fix
    /// before clearing the pause.
    Other(String),
}

/// Public handle to the watcher. Holds an `Arc` so command handlers can
/// poke `clear_paused` without taking ownership of the background task.
pub struct Watcher {
    inner: Arc<WatcherInner>,
}

/// Shared, long-lived watcher state. `pub(crate)` so `tick.rs` can read
/// the maps in place without round-tripping through `Watcher` methods.
pub(crate) struct WatcherInner {
    pub settings: SharedAutosyncSettings,
    pub window_mode: SharedWindowMode,
    pub publish_settings: SharedPublishSettings,
    pub paused: RwLock<BTreeMap<Namespace, PausedReason>>,
    pub backoff: RwLock<BTreeMap<Namespace, BackoffState>>,
    pub login_blocked: RwLock<BTreeMap<Namespace, Option<Host>>>,
    pub reporter: Arc<dyn StatusReporter>,
    pub aggregator: Arc<SyncTrayAggregator>,
}

pub fn create_window_mode() -> SharedWindowMode {
    Arc::new(RwLock::new(WindowMode::Focused))
}

impl Watcher {
    /// Spawn the background tick task and return a handle.
    ///
    /// The task pulls `Model` from `app_handle.state::<Model>()` each
    /// iteration rather than holding its own `Arc<Model>` so we don't
    /// have to refactor every existing Tauri command to switch its
    /// state type from `Model` to `Arc<Model>`.
    pub fn spawn(
        app_handle: tauri::AppHandle,
        settings: SharedAutosyncSettings,
        window_mode: SharedWindowMode,
        publish_settings: SharedPublishSettings,
        reporter: Arc<dyn StatusReporter>,
    ) -> (Self, watch::Receiver<SyncTrayStatus>) {
        let (tx, rx) = watch::channel(SyncTrayStatus::default());
        let aggregator = Arc::new(SyncTrayAggregator::new(tx));
        let inner = Arc::new(WatcherInner {
            settings,
            window_mode,
            publish_settings,
            paused: RwLock::new(BTreeMap::new()),
            backoff: RwLock::new(BTreeMap::new()),
            login_blocked: RwLock::new(BTreeMap::new()),
            reporter,
            aggregator,
        });
        let task_inner = Arc::clone(&inner);
        tauri::async_runtime::spawn(async move {
            loop {
                let cadence = {
                    let settings = task_inner.settings.read().await;
                    let mode = *task_inner.window_mode.read().await;
                    cadence_for_mode(&settings.pull, mode)
                };
                tokio::time::sleep(cadence).await;
                task_inner.aggregator.note_tick_started();
                let model_state = app_handle.state::<Model>();
                match run_once(&*model_state, &task_inner).await {
                    Ok(()) => task_inner.aggregator.note_tick_ended_ok(),
                    Err(err) => {
                        warn!("autosync: tick error: {err}");
                        task_inner.aggregator.note_tick_ended_err();
                    }
                }
            }
        });
        (Self { inner }, rx)
    }

    pub async fn set_window_mode(&self, mode: WindowMode) {
        *self.inner.window_mode.write().await = mode;
    }

    /// Forget a pause for `namespace` — used after the user takes an
    /// explicit action (push / pull / commit / publish / reset / set
    /// remote) that resolves the underlying conflict.
    pub async fn clear_paused(&self, namespace: &Namespace) {
        self.inner.paused.write().await.remove(namespace);
        self.inner.login_blocked.write().await.remove(namespace);
        self.inner.aggregator.note_cleared(namespace);
    }

    /// Drop the entire paused set. Called when `update_autosync_settings`
    /// flips `enabled` from false to true (M3).
    pub async fn clear_all_paused(&self) {
        let mut paused = self.inner.paused.write().await;
        let namespaces: Vec<Namespace> = paused.keys().cloned().collect();
        paused.clear();
        drop(paused);
        self.inner.login_blocked.write().await.clear();
        for ns in namespaces {
            self.inner.aggregator.note_cleared(&ns);
        }
    }

    /// Point-in-time view of the paused set, used by the
    /// `get_autosync_snapshot` Tauri command so the UI can re-hydrate
    /// per-page banners on navigation. Read-only — the lock is released
    /// before the function returns.
    pub async fn snapshot(&self) -> reporter::WatcherSnapshot {
        let paused = self
            .inner
            .paused
            .read()
            .await
            .iter()
            .map(|(ns, reason)| reporter::PausedEvent::from_reason(ns, reason))
            .collect();
        reporter::WatcherSnapshot { paused }
    }

    #[cfg(test)]
    fn new_for_test(reporter: Arc<dyn StatusReporter>) -> Self {
        let (tx, _) = watch::channel(SyncTrayStatus::default());
        Self::new_for_test_with_aggregator(reporter, Arc::new(SyncTrayAggregator::new(tx)))
    }

    #[cfg(test)]
    fn new_for_test_with_aggregator(
        reporter: Arc<dyn StatusReporter>,
        aggregator: Arc<SyncTrayAggregator>,
    ) -> Self {
        Self {
            inner: Arc::new(WatcherInner {
                settings: Arc::new(RwLock::new(AutosyncSettings::default())),
                window_mode: create_window_mode(),
                publish_settings: Arc::new(RwLock::new(
                    crate::publish_settings::PublishSettings::default(),
                )),
                paused: RwLock::new(BTreeMap::new()),
                backoff: RwLock::new(BTreeMap::new()),
                login_blocked: RwLock::new(BTreeMap::new()),
                reporter,
                aggregator,
            }),
        }
    }

    #[cfg(test)]
    async fn login_blocked_for_test(&self) -> BTreeMap<Namespace, Option<Host>> {
        self.inner.login_blocked.read().await.clone()
    }

    #[cfg(test)]
    async fn pause_for_test(&self, namespace: Namespace, reason: PausedReason) {
        self.inner.paused.write().await.insert(namespace, reason);
    }

    #[cfg(test)]
    async fn paused_count(&self) -> usize {
        self.inner.paused.read().await.len()
    }
}

pub fn cadence_for_mode(pull: &PullSettings, mode: WindowMode) -> Duration {
    let secs = match mode {
        WindowMode::Focused => pull.focused_secs,
        WindowMode::Unfocused => pull.unfocused_secs,
        WindowMode::Closed => pull.closed_secs,
    };
    Duration::from_secs(secs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use reporter::LogReporter;
    use status::TrayMode;

    #[test]
    fn cadence_picks_per_mode_secs() {
        let pull = settings::PullSettings {
            enabled: true,
            focused_secs: 1,
            unfocused_secs: 2,
            closed_secs: 3,
        };
        assert_eq!(
            cadence_for_mode(&pull, WindowMode::Focused),
            Duration::from_secs(1),
        );
        assert_eq!(
            cadence_for_mode(&pull, WindowMode::Unfocused),
            Duration::from_secs(2),
        );
        assert_eq!(
            cadence_for_mode(&pull, WindowMode::Closed),
            Duration::from_secs(3),
        );
    }

    #[tokio::test]
    async fn clear_paused_removes_one_namespace() {
        let watcher = Watcher::new_for_test(Arc::new(LogReporter));
        let ns_a: Namespace = ("acme", "demo").into();
        let ns_b: Namespace = ("acme", "other").into();
        watcher
            .pause_for_test(ns_a.clone(), PausedReason::PendingChanges)
            .await;
        watcher
            .pause_for_test(ns_b.clone(), PausedReason::Diverged)
            .await;
        assert_eq!(watcher.paused_count().await, 2);
        watcher.clear_paused(&ns_a).await;
        assert_eq!(watcher.paused_count().await, 1);
    }

    #[tokio::test]
    async fn snapshot_reports_all_paused_entries() {
        let watcher = Watcher::new_for_test(Arc::new(LogReporter));
        let ns_a: Namespace = ("acme", "demo").into();
        let ns_b: Namespace = ("acme", "other").into();

        assert!(watcher.snapshot().await.paused.is_empty());

        watcher
            .pause_for_test(ns_a.clone(), PausedReason::PendingChanges)
            .await;
        watcher
            .pause_for_test(
                ns_b.clone(),
                PausedReason::Other("workflow rejected".to_string()),
            )
            .await;

        let snapshot = watcher.snapshot().await;
        assert_eq!(snapshot.paused.len(), 2);

        let entry_a = snapshot
            .paused
            .iter()
            .find(|p| p.namespace == ns_a.to_string())
            .expect("acme/demo missing from snapshot");
        assert_eq!(entry_a.reason, "pendingChanges");
        assert!(entry_a.message.is_none());

        let entry_b = snapshot
            .paused
            .iter()
            .find(|p| p.namespace == ns_b.to_string())
            .expect("acme/other missing from snapshot");
        assert_eq!(entry_b.reason, "other");
        let msg_b = entry_b
            .message
            .as_deref()
            .expect("Other should carry a message");
        assert!(
            msg_b.starts_with("workflow rejected"),
            "raw error should lead the message, got: {msg_b}"
        );
    }

    #[tokio::test]
    async fn clear_all_paused_empties_set() {
        let watcher = Watcher::new_for_test(Arc::new(LogReporter));
        let ns: Namespace = ("acme", "demo").into();
        watcher.pause_for_test(ns, PausedReason::Diverged).await;
        assert_eq!(watcher.paused_count().await, 1);
        watcher.clear_all_paused().await;
        assert_eq!(watcher.paused_count().await, 0);
    }

    #[tokio::test]
    async fn new_for_test_starts_with_idle_status() {
        let (tx, rx) = watch::channel(SyncTrayStatus::default());
        let watcher = Watcher::new_for_test_with_aggregator(
            Arc::new(LogReporter),
            Arc::new(SyncTrayAggregator::new(tx)),
        );
        let status = rx.borrow().clone();
        assert_eq!(status.mode, TrayMode::Idle);
        assert!(watcher.login_blocked_for_test().await.is_empty());
    }

    #[tokio::test]
    async fn clear_paused_also_clears_aggregator_error() {
        let (tx, rx) = watch::channel(SyncTrayStatus::default());
        let aggregator = Arc::new(SyncTrayAggregator::new(tx));
        let watcher = Watcher::new_for_test_with_aggregator(
            Arc::new(LogReporter),
            aggregator.clone(),
        );
        let ns: Namespace = ("acme", "demo").into();
        watcher
            .pause_for_test(ns.clone(), PausedReason::Diverged)
            .await;
        aggregator.note_paused(&ns, "diverged");
        assert_eq!(rx.borrow().mode, TrayMode::Paused);

        watcher.clear_paused(&ns).await;
        assert!(rx.borrow().error.is_none());
        assert_eq!(rx.borrow().mode, TrayMode::Idle);
    }
}
