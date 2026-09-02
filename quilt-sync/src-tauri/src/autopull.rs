use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use chrono::DateTime;
use chrono::Utc;
use quilt_uri::Host;
use quilt_uri::Namespace;
use tauri::Manager;
use tokio::sync::RwLock;
use tokio::sync::watch;

use crate::autopull::status::SyncTrayAggregator;
use crate::commands::RoleCache;
use crate::experimental_settings::SharedExperimentalSettings;
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
    /// A surgical pull found a real conflict (`PullOutcome::Blocked`): a
    /// tracked path changed on both sides. Carries the conflicting file names
    /// for the banner; the user resolves by committing → the merge page.
    PullConflict(Vec<String>),
    /// Storage refused the call with `AccessDenied`: the active role cannot
    /// reach this package's bucket. Retrying cannot help — the role has to
    /// change first — so this pauses rather than backing off forever.
    ///
    /// Carries the role name for the banner. Empty when the name could not
    /// be resolved (the role query itself failed): the pause still stands,
    /// it just cannot name the role, exactly like the roster's
    /// `AccessMark::denied` degradation.
    RoleDenied {
        role: String,
    },
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

/// The two deadlines the watcher knows and used to discard.
///
/// Neither is derivable from anything already exposed, and both belong to the
/// loop rather than to the tray: `SyncTrayAggregator` folds per-namespace state
/// for the tray icon, and parking a countdown's deadline in it would put the
/// page's clock inside the tray's fold.
#[derive(Default)]
pub(crate) struct Clocks {
    /// When the loop will next tick, recorded before it sleeps.
    ///
    /// Not derived from `last_sync + interval`, which is wrong twice over:
    /// `note_tick_ended_err` deliberately does not bump `last_sync`, so a run of
    /// failing ticks would push the derived deadline further and further into
    /// the past; and it is `None` until the first tick *completes*, so an
    /// enabled toggle would read idle for the first cadence of every session.
    pub next_pull_at: RwLock<Option<DateTime<Utc>>>,
    /// Per namespace: when its quiet window expires and autopush may publish.
    /// Written only where it is known — the deferral branch of
    /// `refresh_then_maybe_sync`, which is the one place that holds both
    /// `most_recent_mtime` and the quiet window — and cleared by any other
    /// outcome, because any other outcome means the package is not waiting.
    pub publish_arm: RwLock<BTreeMap<Namespace, DateTime<Utc>>>,
}

/// Shared, long-lived watcher state. `pub(crate)` so `tick.rs` can read
/// the maps in place without round-tripping through `Watcher` methods.
pub(crate) struct WatcherInner {
    pub settings: SharedAutosyncSettings,
    /// The experiment gate. Read per tick so the standing sync scope applies
    /// to background pulls too — a scope that only held for hand-pressed
    /// pulls would miss the case it exists for.
    pub experimental: SharedExperimentalSettings,
    pub window_mode: SharedWindowMode,
    pub publish_settings: SharedPublishSettings,
    pub paused: RwLock<BTreeMap<Namespace, PausedReason>>,
    pub backoff: RwLock<BTreeMap<Namespace, BackoffState>>,
    pub login_blocked: RwLock<BTreeMap<Namespace, Option<Host>>>,
    pub reporter: Arc<dyn StatusReporter>,
    pub aggregator: Arc<SyncTrayAggregator>,
    pub clocks: Clocks,
}

/// Everything the main page's watcher payload is derived from, read in one call.
///
/// One value, not several accessors, and that is the point: §1's constraint is
/// about **how many places decide what is true**. The payload's paused list and
/// both toggles' `paused` activity come from this struct's `paused` field, so
/// they cannot contradict each other the way the 2026-07-11 report's watcher and
/// `data.json` did.
pub struct WatcherFacts {
    pub pull_enabled: bool,
    pub publish_enabled: bool,
    /// Every paused namespace with its reason, from one read of the map.
    pub paused: Vec<(Namespace, PausedReason)>,
    pub next_pull_at: Option<DateTime<Utc>>,
    /// The **earliest** arm time across every namespace waiting out its quiet
    /// window — the next thing that will publish, which is what one countdown
    /// can honestly represent. `None` when nothing is waiting.
    pub publish_arm_at: Option<DateTime<Utc>>,
    /// The cadence the loop is actually sleeping, which depends on window mode.
    /// Not `pull_interval_secs` from the settings payload: `focused_secs`,
    /// `unfocused_secs` and `closed_secs` can differ, and a ring drawn from the
    /// wrong one of the three is a ring that finishes at the wrong time.
    pub pull_interval: Duration,
    pub publish_interval: Duration,
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
        experimental: SharedExperimentalSettings,
        reporter: Arc<dyn StatusReporter>,
    ) -> (Self, watch::Receiver<SyncTrayStatus>) {
        let (tx, rx) = watch::channel(SyncTrayStatus::default());
        let aggregator = Arc::new(SyncTrayAggregator::new(tx));
        let inner = Arc::new(WatcherInner {
            settings,
            window_mode,
            publish_settings,
            experimental,
            paused: RwLock::new(BTreeMap::new()),
            backoff: RwLock::new(BTreeMap::new()),
            login_blocked: RwLock::new(BTreeMap::new()),
            reporter,
            aggregator,
            clocks: Clocks::default(),
        });
        let task_inner = Arc::clone(&inner);
        tauri::async_runtime::spawn(async move {
            loop {
                let cadence = {
                    let settings = task_inner.settings.read().await;
                    let mode = *task_inner.window_mode.read().await;
                    cadence_for_mode(&settings.pull, mode)
                };
                // Recorded before the wait, not after: the deadline has to be
                // readable for the whole cadence, which is when the card draws it.
                arm_next_pull(&task_inner, cadence).await;
                tokio::time::sleep(cadence).await;
                task_inner.aggregator.note_tick_started();
                let model_state = app_handle.state::<Model>();
                // Same `state::<..>()` route as `Model`, and for the same
                // reason: the watcher must share the *managed* cache, not a
                // private one. A private copy would keep naming the role the
                // user switched away from — `switch_role` invalidates only
                // the managed instance.
                let roles_state = app_handle.state::<RoleCache>();
                // Box the tick future: it is the largest in the app and lives
                // on the spawn task's stack across every iteration of this
                // loop, so it exceeds the `large_futures` budget. See
                // `clippy.toml`.
                match Box::pin(run_once(&*model_state, &roles_state, &task_inner)).await {
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
        let mut login_blocked = self.inner.login_blocked.write().await;
        let namespaces: BTreeSet<Namespace> =
            paused.keys().chain(login_blocked.keys()).cloned().collect();
        paused.clear();
        login_blocked.clear();
        drop(paused);
        drop(login_blocked);
        for ns in &namespaces {
            self.inner.aggregator.note_cleared(ns);
        }
    }

    /// Forget every pause caused by a role denial. Called by `switch_role`
    /// once the switch lands.
    ///
    /// A [`PausedReason::RoleDenied`] is the one pause no other affordance
    /// can clear: `clear_paused` fires after a manual push / pull / commit,
    /// and every one of those is refused by the same role that caused the
    /// pause. Without this, the banner's "switch role to resume" would be a
    /// lie — the namespace would stay parked until the app restarts.
    ///
    /// Not host-scoped: the reason carries a role name, not a host. A switch
    /// on one host therefore also un-pauses a denial on another, which costs
    /// at most one extra attempt per package — still denied means paused
    /// again on the same tick, with the role re-read from the freshly
    /// invalidated cache.
    pub async fn clear_role_denied_pauses(&self) {
        let mut paused = self.inner.paused.write().await;
        let cleared: Vec<Namespace> = paused
            .iter()
            .filter(|(_, reason)| matches!(reason, PausedReason::RoleDenied { .. }))
            .map(|(namespace, _)| namespace.clone())
            .collect();
        for namespace in &cleared {
            paused.remove(namespace);
        }
        drop(paused);
        for namespace in &cleared {
            self.inner.aggregator.note_cleared(namespace);
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

    /// One read, for the main page's watcher payload.
    pub async fn main_page_facts(&self) -> WatcherFacts {
        let settings = self.inner.settings.read().await.clone();
        let mode = *self.inner.window_mode.read().await;
        let paused = self
            .inner
            .paused
            .read()
            .await
            .iter()
            .map(|(ns, reason)| (ns.clone(), reason.clone()))
            .collect();
        WatcherFacts {
            pull_enabled: settings.pull.enabled,
            publish_enabled: settings.push.enabled,
            paused,
            next_pull_at: *self.inner.clocks.next_pull_at.read().await,
            publish_arm_at: self
                .inner
                .clocks
                .publish_arm
                .read()
                .await
                .values()
                .min()
                .copied(),
            pull_interval: cadence_for_mode(&settings.pull, mode),
            publish_interval: Duration::from_secs(settings.push.idle_timeout_secs),
        }
    }

    /// One namespace's pause, for the per-package refresh. A lookup, not a
    /// second resolution: the map is still the only thing that decides.
    ///
    /// `expect` rather than `allow`: the per-package refresh that calls this
    /// lands next, and the expectation fails the build the moment it does.
    #[expect(dead_code, reason = "the per-package refresh calls this next")]
    pub async fn paused_reason(&self, namespace: &Namespace) -> Option<PausedReason> {
        self.inner.paused.read().await.get(namespace).cloned()
    }

    /// The shared state the tick loop reads. Lets a test drive
    /// [`tick::run_once`] against the very same watcher a command handler
    /// holds — the two halves of the role-denial pause path, which is
    /// created by a tick and released by the switch command.
    #[cfg(test)]
    pub(crate) fn inner_for_test(&self) -> &WatcherInner {
        &self.inner
    }

    #[cfg(test)]
    pub(crate) fn new_for_test(reporter: Arc<dyn StatusReporter>) -> Self {
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
                experimental: Arc::new(RwLock::new(
                    crate::experimental_settings::ExperimentalSettings::default(),
                )),
                window_mode: create_window_mode(),
                publish_settings: Arc::new(RwLock::new(
                    crate::publish_settings::PublishSettings::default(),
                )),
                paused: RwLock::new(BTreeMap::new()),
                backoff: RwLock::new(BTreeMap::new()),
                login_blocked: RwLock::new(BTreeMap::new()),
                reporter,
                aggregator,
                clocks: Clocks::default(),
            }),
        }
    }

    #[cfg(test)]
    async fn login_blocked_for_test(&self) -> BTreeMap<Namespace, Option<Host>> {
        self.inner.login_blocked.read().await.clone()
    }

    #[cfg(test)]
    pub(crate) async fn pause_for_test(&self, namespace: Namespace, reason: PausedReason) {
        self.inner.paused.write().await.insert(namespace, reason);
    }

    #[cfg(test)]
    async fn login_block_for_test(&self, namespace: Namespace, host: Option<Host>) {
        self.inner
            .login_blocked
            .write()
            .await
            .insert(namespace, host);
    }

    #[cfg(test)]
    async fn paused_count(&self) -> usize {
        self.inner.paused.read().await.len()
    }
}

/// Record when the loop will next tick.
///
/// A free function rather than a line inside `Watcher::spawn` because the spawn
/// loop needs a Tauri runtime and cannot be driven from a test, and an untested
/// rule is a rule that survives only as prose.
pub(crate) async fn arm_next_pull(inner: &WatcherInner, cadence: Duration) {
    *inner.clocks.next_pull_at.write().await = Some(Utc::now() + cadence);
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

    /// The switch has to actually resume what the denial parked — and only
    /// that. A pause with another cause is still true after a switch, so
    /// clearing it would send autosync back into a conflict the user has not
    /// resolved.
    #[tokio::test]
    async fn clear_role_denied_pauses_leaves_other_reasons_alone() {
        let watcher = Watcher::new_for_test(Arc::new(LogReporter));
        let denied: Namespace = ("acme", "denied").into();
        let diverged: Namespace = ("acme", "diverged").into();
        watcher
            .pause_for_test(
                denied.clone(),
                PausedReason::RoleDenied {
                    role: "ReadOnly".to_string(),
                },
            )
            .await;
        watcher
            .pause_for_test(diverged.clone(), PausedReason::Diverged)
            .await;

        watcher.clear_role_denied_pauses().await;

        let paused = watcher.snapshot().await.paused;
        assert_eq!(paused.len(), 1, "only the denial should clear: {paused:?}");
        assert_eq!(paused[0].namespace, diverged.to_string());
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
    async fn clear_all_paused_clears_login_blocked_only_aggregator_error() {
        // Regression: a namespace that hit `LoginRequired` lives in
        // `login_blocked` and in the aggregator's error map but never
        // enters `paused`. Re-enabling autosync must still drop the
        // aggregator error so the tray doesn't stay stuck in Error.
        let (tx, rx) = watch::channel(SyncTrayStatus::default());
        let aggregator = Arc::new(SyncTrayAggregator::new(tx));
        let watcher =
            Watcher::new_for_test_with_aggregator(Arc::new(LogReporter), aggregator.clone());
        let ns: Namespace = ("acme", "demo").into();

        watcher.login_block_for_test(ns.clone(), None).await;
        aggregator.note_login_required(&ns, None);
        assert_eq!(rx.borrow().mode, TrayMode::Error);

        watcher.clear_all_paused().await;
        assert!(watcher.login_blocked_for_test().await.is_empty());
        assert!(rx.borrow().error.is_none());
        assert_eq!(rx.borrow().mode, TrayMode::Idle);
    }

    #[tokio::test]
    async fn clear_paused_also_clears_aggregator_error() {
        let (tx, rx) = watch::channel(SyncTrayStatus::default());
        let aggregator = Arc::new(SyncTrayAggregator::new(tx));
        let watcher =
            Watcher::new_for_test_with_aggregator(Arc::new(LogReporter), aggregator.clone());
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
