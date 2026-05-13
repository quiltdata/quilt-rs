use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use quilt_uri::Namespace;
use tauri::Manager;
use tokio::sync::RwLock;

use crate::model::Model;
use crate::telemetry::prelude::*;

pub mod reporter;
pub mod settings;
pub mod tick;

#[allow(unused_imports)]
pub use reporter::LogReporter;
#[allow(unused_imports)]
pub use reporter::PackageStatusEvent;
pub use reporter::StatusReporter;
pub use settings::AutopullSettings;
pub use settings::SharedAutopullSettings;
pub use settings::init as init_settings;

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
    /// Wired in milestone 4 (`05-trayicon.md`); kept here so the cadence
    /// table and the on-disk settings file are forward-compatible.
    #[allow(dead_code)]
    Closed,
}

pub type SharedWindowMode = Arc<RwLock<WindowMode>>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PausedReason {
    PendingChanges,
    PendingCommit,
    Diverged,
}

/// Public handle to the watcher. Holds an `Arc` so command handlers can
/// poke `clear_paused` without taking ownership of the background task.
pub struct Watcher {
    inner: Arc<WatcherInner>,
}

/// Shared, long-lived watcher state. `pub(crate)` so `tick.rs` can read
/// the maps in place without round-tripping through `Watcher` methods.
pub(crate) struct WatcherInner {
    pub settings: SharedAutopullSettings,
    pub window_mode: SharedWindowMode,
    pub paused: RwLock<BTreeMap<Namespace, PausedReason>>,
    pub backoff: RwLock<BTreeMap<Namespace, BackoffState>>,
    pub reporter: Arc<dyn StatusReporter>,
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
        settings: SharedAutopullSettings,
        window_mode: SharedWindowMode,
        reporter: Arc<dyn StatusReporter>,
    ) -> Self {
        let inner = Arc::new(WatcherInner {
            settings,
            window_mode,
            paused: RwLock::new(BTreeMap::new()),
            backoff: RwLock::new(BTreeMap::new()),
            reporter,
        });
        let task_inner = Arc::clone(&inner);
        tauri::async_runtime::spawn(async move {
            loop {
                let cadence = {
                    let settings = task_inner.settings.read().await;
                    let mode = *task_inner.window_mode.read().await;
                    cadence_for_mode(&settings, mode)
                };
                tokio::time::sleep(cadence).await;
                let model_state = app_handle.state::<Model>();
                if let Err(err) = run_once(&*model_state, &task_inner).await {
                    warn!("autopull: tick error: {err}");
                }
            }
        });
        Self { inner }
    }

    pub async fn set_window_mode(&self, mode: WindowMode) {
        *self.inner.window_mode.write().await = mode;
    }

    /// Forget a pause for `namespace` — used after the user takes an
    /// explicit action (push / pull / commit / publish / reset / set
    /// remote) that resolves the underlying conflict.
    pub async fn clear_paused(&self, namespace: &Namespace) {
        self.inner.paused.write().await.remove(namespace);
    }

    /// Drop the entire paused set. Called when `update_autopull_settings`
    /// flips `enabled` from false to true (M3).
    pub async fn clear_all_paused(&self) {
        self.inner.paused.write().await.clear();
    }

    #[cfg(test)]
    fn new_for_test(reporter: Arc<dyn StatusReporter>) -> Self {
        Self {
            inner: Arc::new(WatcherInner {
                settings: Arc::new(RwLock::new(AutopullSettings::default())),
                window_mode: create_window_mode(),
                paused: RwLock::new(BTreeMap::new()),
                backoff: RwLock::new(BTreeMap::new()),
                reporter,
            }),
        }
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

pub fn cadence_for_mode(settings: &AutopullSettings, mode: WindowMode) -> Duration {
    let secs = match mode {
        WindowMode::Focused => settings.focused_secs,
        WindowMode::Unfocused => settings.unfocused_secs,
        WindowMode::Closed => settings.closed_secs,
    };
    Duration::from_secs(secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cadence_picks_per_mode_secs() {
        let s = AutopullSettings {
            enabled: true,
            focused_secs: 1,
            unfocused_secs: 2,
            closed_secs: 3,
        };
        assert_eq!(cadence_for_mode(&s, WindowMode::Focused), Duration::from_secs(1));
        assert_eq!(
            cadence_for_mode(&s, WindowMode::Unfocused),
            Duration::from_secs(2)
        );
        assert_eq!(cadence_for_mode(&s, WindowMode::Closed), Duration::from_secs(3));
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
    async fn clear_all_paused_empties_set() {
        let watcher = Watcher::new_for_test(Arc::new(LogReporter));
        let ns: Namespace = ("acme", "demo").into();
        watcher
            .pause_for_test(ns, PausedReason::Diverged)
            .await;
        assert_eq!(watcher.paused_count().await, 1);
        watcher.clear_all_paused().await;
        assert_eq!(watcher.paused_count().await, 0);
    }
}
