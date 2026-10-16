use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;
use std::time::SystemTime;

use chrono::DateTime;
use chrono::Utc;
use quilt_uri::Host;
use quilt_uri::Namespace;
use tauri::Manager;
use tokio::sync::RwLock;
use tokio::sync::watch;

use crate::autopull::status::SyncTrayAggregator;
use crate::commands::RoleCache;
use crate::model::Model;
use crate::publish_settings::SharedPublishSettings;
use crate::telemetry::prelude::*;

pub mod activity;
pub mod last_change;
pub mod pull_toast;
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

impl PausedReason {
    /// The pause for a pull the engine refused because `paths` changed on both
    /// sides, worded for the banner.
    pub fn pull_conflict(paths: &[std::path::PathBuf]) -> Self {
        Self::PullConflict(paths.iter().map(|p| p.display().to_string()).collect())
    }
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
    /// The packages counting down to a publish. Their deadlines are not kept
    /// here: each is the stored last-change time plus the wait as it is when
    /// read, so a new wait applies at once.
    pub publish_waiting: RwLock<BTreeSet<Namespace>>,
}

/// Shared, long-lived watcher state. `pub(crate)` so `tick.rs` can read
/// the maps in place without round-tripping through `Watcher` methods.
pub(crate) struct WatcherInner {
    pub settings: SharedAutosyncSettings,
    pub publish_settings: SharedPublishSettings,
    pub paused: RwLock<BTreeMap<Namespace, PausedReason>>,
    pub backoff: RwLock<BTreeMap<Namespace, BackoffState>>,
    pub login_blocked: RwLock<BTreeMap<Namespace, Option<Host>>>,
    pub reporter: Arc<dyn StatusReporter>,
    pub aggregator: Arc<SyncTrayAggregator>,
    pub clocks: Clocks,
    pub last_change: last_change::LastChange,
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
    /// The cadence the loop is actually sleeping: the one check interval.
    pub pull_interval: Duration,
    pub publish_interval: Duration,
}

impl Watcher {
    /// Hold the apply flag for as long as the guard lives — the command
    /// layer's way in, so a hand-pressed [pull](crate::commands::package_pull)
    /// raises the same question a background one does. The tick takes its own
    /// guard further down, at the call it brackets.
    #[must_use]
    pub fn apply_guard(&self, namespace: &Namespace) -> crate::autopull::status::ApplyGuard<'_> {
        self.inner.aggregator.apply_guard(namespace)
    }

    /// Whether a pull is applying — writing working files — right now.
    /// Synchronous, because its caller is a menu/window event handler that
    /// cannot await.
    #[must_use]
    pub fn apply_in_progress(&self) -> bool {
        self.inner.aggregator.apply_in_progress()
    }

    /// The transfer autopull is running now, if any.
    #[must_use]
    pub fn activity(&self) -> Option<activity::AutopullActivity> {
        self.inner.aggregator.activity()
    }

    /// Follow the transfer autopull is running: every set and every clear.
    #[must_use]
    pub fn subscribe_activity(&self) -> watch::Receiver<Option<activity::AutopullActivity>> {
        self.inner.aggregator.subscribe_activity()
    }

    /// Spawn the background tick task and return a handle.
    ///
    /// The task pulls `Model` from `app_handle.state::<Model>()` each
    /// iteration rather than holding its own `Arc<Model>` so we don't
    /// have to refactor every existing Tauri command to switch its
    /// state type from `Model` to `Arc<Model>`.
    pub fn spawn(
        app_handle: tauri::AppHandle,
        settings: SharedAutosyncSettings,
        publish_settings: SharedPublishSettings,
        reporter: Arc<dyn StatusReporter>,
        last_change: last_change::LastChange,
    ) -> (Self, watch::Receiver<SyncTrayStatus>) {
        let (tx, rx) = watch::channel(SyncTrayStatus::default());
        let aggregator = Arc::new(SyncTrayAggregator::new(tx));
        let inner = Arc::new(WatcherInner {
            settings,
            publish_settings,
            paused: RwLock::new(BTreeMap::new()),
            backoff: RwLock::new(BTreeMap::new()),
            login_blocked: RwLock::new(BTreeMap::new()),
            reporter,
            aggregator,
            clocks: Clocks::default(),
            last_change,
        });
        let task_inner = Arc::clone(&inner);
        tauri::async_runtime::spawn(async move {
            loop {
                let cadence = pull_cadence(&task_inner.settings.read().await.pull);
                // This deadline covers the sleep, which is when the Autosync
                // card draws it. `run_once` arms again when the tick starts,
                // because by then this deadline is already in the past.
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

    /// Pause `namespace` for `reason` and announce it, exactly as the tick does
    /// when it meets the same refusal — the command layer's way in.
    ///
    /// A hand-pressed pull the engine refused for a conflict lands here, so the
    /// page shows the conflict at once rather than after the next tick. It is
    /// recorded whether or not autosync runs for the package: the pages read
    /// the paused map either way, and every route that clears a pause
    /// ([`Self::clear_paused`] after a manual push / pull / commit / publish /
    /// reset, [`Self::clear_all_paused`] when autosync is switched on) clears
    /// this one too.
    pub async fn pause(&self, namespace: &Namespace, origin: &Host, reason: PausedReason) {
        tick::pause(&self.inner, namespace, origin, reason).await;
    }

    /// Forget a pause for `namespace` — used after the user takes an
    /// explicit action (push / pull / commit / publish / reset / set
    /// remote) that resolves the underlying conflict.
    pub async fn clear_paused(&self, namespace: &Namespace) {
        self.inner.paused.write().await.remove(namespace);
        self.inner.login_blocked.write().await.remove(namespace);
        self.inner.aggregator.note_cleared(namespace);
    }

    /// Take back a [`PausedReason::PullConflict`] the pull dry run no longer
    /// finds — the conflicting edit undone, or the lineage advanced by another
    /// process — and announce it. Returns whether there was one to take back.
    ///
    /// Only a conflict: it is the one pause the dry run's verdict speaks to.
    /// Every other reason, and a login block, stays.
    ///
    /// Announced as a status event, because a clear is otherwise silent (see
    /// [`Self::clear_paused`]) and an open page would keep offering Publish for
    /// a conflict that is gone. `status` is the package's observation; its
    /// fingerprint alone would not do, since a pause moves neither the upstream
    /// state, the newer revision nor the tree, so a page may already have acted on that very
    /// observation and would drop the event as old news. The fingerprint is
    /// therefore marked as the clear's. The same observation sets the tray's
    /// change count, since a safe verdict can keep local edits in place.
    pub async fn clear_pull_conflict(
        &self,
        namespace: &Namespace,
        status: &quilt_rs::lineage::InstalledPackageStatus,
    ) -> bool {
        {
            let mut paused = self.inner.paused.write().await;
            if !matches!(paused.get(namespace), Some(PausedReason::PullConflict(_))) {
                return false;
            }
            paused.remove(namespace);
        }
        // Only the pause: a safe verdict can keep local edits in place, so the
        // tray's change count is set from the observation, not dropped.
        self.inner.aggregator.clear_error(namespace);
        self.inner
            .aggregator
            .note_status(namespace, !status.changes.is_empty());
        let mut event = PackageStatusEvent::from_status(namespace, status);
        event.fingerprint = format!("unpaused;{}", event.fingerprint);
        self.inner.reporter.report_status(namespace, event);
        true
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

    /// A package committed by hand: its push waits a full wait from now.
    pub async fn note_committed(&self, namespace: &Namespace) {
        self.inner.last_change.on_commit(namespace).await;
        if self.inner.settings.read().await.push.enabled {
            self.inner
                .clocks
                .publish_waiting
                .write()
                .await
                .insert(namespace.clone());
        }
    }

    /// A package published by hand: its countdown ends at once.
    pub async fn note_published(&self, namespace: &Namespace) {
        self.inner.last_change.on_published(namespace).await;
        self.inner
            .clocks
            .publish_waiting
            .write()
            .await
            .remove(namespace);
    }

    /// Publishing was just turned on: every package with changes waits a full
    /// wait from now, however old its edits are.
    pub async fn note_publishing_on(&self) {
        let with_changes = self.inner.aggregator.namespaces_with_changes();
        let moved = self.inner.last_change.on_publishing_on(with_changes).await;
        self.inner
            .clocks
            .publish_waiting
            .write()
            .await
            .extend(moved);
    }

    /// One read, for the main page's watcher payload.
    pub async fn main_page_facts(&self) -> WatcherFacts {
        let settings = self.inner.settings.read().await.clone();
        let wait = Duration::from_secs(settings.push.idle_timeout_secs);
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
            publish_arm_at: next_publish_at(&self.inner, wait).await,
            pull_interval: pull_cadence(&settings.pull),
            publish_interval: wait,
        }
    }

    /// One namespace's pause, for the per-package refresh. A lookup, not a
    /// second resolution: the map is still the only thing that decides.
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
                publish_settings: Arc::new(RwLock::new(
                    crate::publish_settings::PublishSettings::default(),
                )),
                paused: RwLock::new(BTreeMap::new()),
                backoff: RwLock::new(BTreeMap::new()),
                login_blocked: RwLock::new(BTreeMap::new()),
                reporter,
                aggregator,
                clocks: Clocks::default(),
                last_change: last_change::LastChange::new(
                    crate::quilt::state_store::PackageStateStore::in_memory(),
                ),
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

    /// The shared state, for the file watcher — which observes an edit within
    /// its debounce where this loop can be a whole cadence behind.
    ///
    /// Not `inner`: a `tauri::State<Watcher>` has an inherent `inner()` of its
    /// own, and the call site holds one, so that name resolves to the guard's
    /// method rather than this.
    pub(crate) fn shared(&self) -> &WatcherInner {
        &self.inner
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

/// The earliest deadline among the packages waiting to publish, with `wait` as
/// it is now. `None` when nothing waits, and for a deadline already past: that
/// package publishes at the next check, so there is nothing to count down to.
pub(crate) async fn next_publish_at(inner: &WatcherInner, wait: Duration) -> Option<DateTime<Utc>> {
    let waiting: Vec<Namespace> = inner
        .clocks
        .publish_waiting
        .read()
        .await
        .iter()
        .cloned()
        .collect();
    let now = SystemTime::now();
    let mut next: Option<SystemTime> = None;
    for namespace in &waiting {
        let deadline = publish_deadline(inner.last_change.get(namespace).await, wait);
        if let Some(at) = deadline.filter(|at| *at > now) {
            next = Some(next.map_or(at, |cur| cur.min(at)));
        }
    }
    next.map(DateTime::<Utc>::from)
}

/// Hand the file watcher's walk of `namespace` to the last-change clock, and
/// record whether the package now counts down to a publish.
///
/// For the file watcher, which hears about an edit within its debounce where the
/// tick can be a whole cadence behind. The tick stays authoritative: it sees the
/// upstream state, which a local walk cannot, and clears a countdown that turns
/// out not to apply.
pub(crate) async fn observe_walk(
    inner: &WatcherInner,
    namespace: &Namespace,
    home: Option<&std::path::Path>,
    status: &crate::quilt::lineage::InstalledPackageStatus,
    pending_commit: Option<bool>,
) {
    let changed_at = inner
        .last_change
        .observe(namespace, home, status, pending_commit)
        .await;
    let push_enabled = inner.settings.read().await.push.enabled;
    let mut waiting = inner.clocks.publish_waiting.write().await;
    // Removed, not left alone: a revert leaves a tree with nothing in it, and a
    // countdown that outlives the changes it was counting for is stale.
    if push_enabled && changed_at.is_some() {
        waiting.insert(namespace.clone());
    } else {
        waiting.remove(namespace);
    }
}

/// When a package publishes: its last change plus the wait. `None` when it has
/// no last-change time, which is a package without changes.
///
/// The one rule the tick and the Autosync card both read, so they cannot drift.
pub(crate) fn publish_deadline(
    last_change: Option<SystemTime>,
    wait: Duration,
) -> Option<SystemTime> {
    last_change.map(|at| at + wait)
}

/// A `WatcherInner` for a test, shared so the two modules that need one do not
/// keep their own copies in step by hand.
#[cfg(test)]
pub(crate) fn inner_for_tests(settings: AutosyncSettings) -> WatcherInner {
    let (tx, _rx) = tokio::sync::watch::channel(status::SyncTrayStatus::default());
    WatcherInner {
        settings: Arc::new(RwLock::new(settings)),
        publish_settings: Arc::new(RwLock::new(
            crate::publish_settings::PublishSettings::default(),
        )),
        paused: RwLock::new(BTreeMap::new()),
        backoff: RwLock::new(BTreeMap::new()),
        login_blocked: RwLock::new(BTreeMap::new()),
        reporter: Arc::new(reporter::LogReporter),
        aggregator: Arc::new(status::SyncTrayAggregator::new(tx)),
        clocks: Clocks::default(),
        last_change: last_change::LastChange::new(
            crate::quilt::state_store::PackageStateStore::in_memory(),
        ),
    }
}

/// How long the loop sleeps between checks: the one interval, whether the
/// window is open, in the background or closed to the tray.
pub fn pull_cadence(pull: &PullSettings) -> Duration {
    Duration::from_secs(pull.focused_secs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use reporter::LogReporter;
    use status::TrayMode;

    /// The retired unfocused and closed intervals are never read.
    #[test]
    fn the_cadence_is_the_one_interval() {
        let pull = settings::PullSettings {
            enabled: true,
            focused_secs: 60,
            unfocused_secs: 120,
            closed_secs: 600,
        };
        assert_eq!(pull_cadence(&pull), Duration::from_secs(60));
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

        assert_eq!(
            watcher.snapshot().await.paused,
            [] as [reporter::PausedEvent; 0]
        );

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
            .find(|p| p.namespace.to_string() == ns_a.to_string())
            .expect("acme/demo missing from snapshot");
        assert_eq!(entry_a.reason, "pendingChanges");
        assert!(entry_a.message.is_none());

        let entry_b = snapshot
            .paused
            .iter()
            .find(|p| p.namespace.to_string() == ns_b.to_string())
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
        assert_eq!(paused[0].namespace.to_string(), diverged.to_string());
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

    // A hand-pressed Pull applies the same delta the tick does, so a quit
    // during one must ask the same question. The command layer reaches the flag
    // through the watcher it already holds, so the guard has to be available
    // there and not only inside the tick.
    #[tokio::test]
    async fn the_watcher_lends_its_apply_guard_to_a_hand_pressed_pull() {
        let (tx, _rx) = watch::channel(SyncTrayStatus::default());
        let aggregator = Arc::new(SyncTrayAggregator::new(tx));
        let watcher =
            Watcher::new_for_test_with_aggregator(Arc::new(LogReporter), aggregator.clone());
        let ns: Namespace = ("acme", "demo").into();

        assert!(!aggregator.apply_in_progress(), "nothing applying yet");
        {
            let _applying = watcher.apply_guard(&ns);
            assert!(
                aggregator.apply_in_progress(),
                "a pull driven from the command layer must raise the same flag"
            );
        }
        assert!(
            !aggregator.apply_in_progress(),
            "and drop it when the pull returns"
        );
    }

    // The window's activity line reads the watcher it already holds, both to
    // hydrate and to follow, so the aggregator's value has to be reachable there.
    #[tokio::test]
    async fn the_watcher_reads_the_aggregators_activity() {
        let watcher = Watcher::new_for_test(Arc::new(LogReporter));
        let ns: Namespace = ("acme", "demo").into();
        let mut following = watcher.subscribe_activity();

        assert_eq!(watcher.activity(), None, "nothing running yet");
        {
            let _pulling = watcher
                .inner
                .aggregator
                .activity_guard(activity::ActivityOp::Pull, &ns);
            assert_eq!(
                watcher.activity(),
                Some(activity::AutopullActivity {
                    op: activity::ActivityOp::Pull,
                    namespace: ns.clone(),
                })
            );
            assert!(
                following.has_changed().unwrap(),
                "the follower sees the set"
            );
            assert_eq!(
                following.borrow_and_update().as_ref().map(|a| a.op),
                Some(activity::ActivityOp::Pull)
            );
        }
        assert_eq!(watcher.activity(), None, "cleared when the transfer ends");
        assert!(
            following.has_changed().unwrap(),
            "the follower sees the clear"
        );
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
    async fn main_page_facts_puts_each_clock_in_its_own_slot() {
        // The state-to-payload path itself. Every payload test starts from a
        // hand-built `WatcherFacts`, so a transposition here — the pull deadline
        // into the publish slot, or `.max()` where the rule is `.min()` — would
        // pass all of them. Every value below is distinguishable from every other
        // for exactly that reason.
        let watcher = Watcher::new_for_test(Arc::new(LogReporter));
        {
            let mut settings = watcher.inner_for_test().settings.write().await;
            settings.pull.enabled = true;
            settings.push.enabled = false;
            settings.pull.focused_secs = 11;
            settings.pull.unfocused_secs = 22;
            settings.pull.closed_secs = 33;
            settings.push.idle_timeout_secs = 44;
        }
        let pull_at = Utc::now() + Duration::from_mins(15);
        *watcher.inner_for_test().clocks.next_pull_at.write().await = Some(pull_at);
        // Two namespaces waiting, changed at different moments: the earliest
        // deadline is the next thing that will publish, so `.max()` cannot pass
        // this. Each deadline is the last change plus the wait of 44 s.
        let early: Namespace = ("acme", "early").into();
        let late: Namespace = ("acme", "late").into();
        let early_change = crate::autopull::last_change::now();
        let inner = watcher.inner_for_test();
        inner
            .last_change
            .store_for_test()
            .record_change(
                &early,
                early_change,
                crate::quilt::state_store::ChangeSource::Edit,
            )
            .await;
        inner
            .last_change
            .store_for_test()
            .record_change(
                &late,
                early_change + Duration::from_secs(600),
                crate::quilt::state_store::ChangeSource::Edit,
            )
            .await;
        inner
            .clocks
            .publish_waiting
            .write()
            .await
            .extend([early, late]);
        let earliest = DateTime::<Utc>::from(early_change + Duration::from_secs(44));

        let facts = watcher.main_page_facts().await;
        assert!(facts.pull_enabled, "the pull setting, not the push one");
        assert!(!facts.publish_enabled, "the push setting, not the pull one");
        assert_eq!(
            facts.next_pull_at,
            Some(pull_at),
            "the pull clock belongs in the pull slot"
        );
        assert_eq!(
            facts.publish_arm_at,
            Some(earliest),
            "the earliest deadline across every waiting namespace, not the last"
        );
        assert_eq!(
            facts.pull_interval,
            Duration::from_secs(11),
            "the one interval, not a retired one"
        );
        assert_eq!(
            facts.publish_interval,
            Duration::from_secs(44),
            "the wait, from the push settings"
        );
    }

    #[tokio::test]
    async fn a_manual_publish_ends_the_countdown_at_once() {
        let watcher = Watcher::new_for_test(Arc::new(LogReporter));
        let ns: Namespace = ("acme", "demo").into();
        let inner = watcher.inner_for_test();
        inner.last_change.on_signal(&ns).await;
        inner
            .clocks
            .publish_waiting
            .write()
            .await
            .insert(ns.clone());

        watcher.note_published(&ns).await;

        assert_eq!(inner.last_change.get(&ns).await, None);
        assert!(inner.clocks.publish_waiting.read().await.is_empty());
        assert_eq!(watcher.main_page_facts().await.publish_arm_at, None);
    }

    #[tokio::test]
    async fn a_manual_commit_starts_the_countdown_to_its_push() {
        let watcher = Watcher::new_for_test(Arc::new(LogReporter));
        let ns: Namespace = ("acme", "demo").into();
        let inner = watcher.inner_for_test();
        {
            let mut settings = inner.settings.write().await;
            settings.push.enabled = true;
            settings.push.idle_timeout_secs = 600;
        }
        let before = DateTime::<Utc>::from(last_change::now());

        watcher.note_committed(&ns).await;

        let deadline = watcher
            .main_page_facts()
            .await
            .publish_arm_at
            .expect("the card counts down to the push");
        assert!(deadline >= before + Duration::from_secs(600));
    }

    #[tokio::test]
    async fn turning_publishing_on_waits_a_full_wait_from_now() {
        // Edits from yesterday must not publish at the next check.
        let watcher = Watcher::new_for_test(Arc::new(LogReporter));
        let ns: Namespace = ("acme", "demo").into();
        let inner = watcher.inner_for_test();
        inner.settings.write().await.push.idle_timeout_secs = 600;
        inner
            .last_change
            .store_for_test()
            .record_change(
                &ns,
                SystemTime::now() - Duration::from_hours(24),
                crate::quilt::state_store::ChangeSource::Edit,
            )
            .await;
        let unseen: Namespace = ("acme", "unseen").into();
        inner.aggregator.note_status(&unseen, true);
        let before = DateTime::<Utc>::from(last_change::now());

        watcher.note_publishing_on().await;

        for namespace in [&ns, &unseen] {
            let at = inner.last_change.get(namespace).await.expect("moved");
            assert!(
                DateTime::<Utc>::from(at) >= before,
                "{namespace} moved to now"
            );
        }
        let deadline = watcher
            .main_page_facts()
            .await
            .publish_arm_at
            .expect("the card counts down at once");
        assert!(deadline >= before + Duration::from_secs(600));
    }

    #[tokio::test]
    async fn nothing_waiting_to_publish_arms_nothing() {
        // The `None` half of the same rule: an empty arm map is "nothing is
        // waiting", which is what makes the publish toggle read idle rather than
        // counting down to a moment with no meaning.
        let watcher = Watcher::new_for_test(Arc::new(LogReporter));
        let facts = watcher.main_page_facts().await;
        assert_eq!(facts.publish_arm_at, None);
        assert_eq!(facts.next_pull_at, None);
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

    /// Taking back a conflict clears only the pause: a safe pull that keeps
    /// local changes leaves edited files in place, so the tray still counts
    /// the package as having changes.
    #[tokio::test]
    async fn clear_pull_conflict_keeps_the_package_counted_as_changed() {
        let (tx, rx) = watch::channel(SyncTrayStatus::default());
        let aggregator = Arc::new(SyncTrayAggregator::new(tx));
        let watcher =
            Watcher::new_for_test_with_aggregator(Arc::new(LogReporter), aggregator.clone());
        let ns: Namespace = ("acme", "demo").into();
        watcher
            .pause_for_test(ns.clone(), PausedReason::PullConflict(vec!["a.csv".into()]))
            .await;
        aggregator.note_paused(&ns, "conflict");
        aggregator.note_status(&ns, true);
        let status = crate::quilt::lineage::InstalledPackageStatus::new(
            crate::quilt::lineage::UpstreamState::Behind,
            BTreeMap::from([(
                std::path::PathBuf::from("a.csv"),
                crate::quilt::lineage::Change::Modified(
                    crate::quilt::manifest::ManifestRow::default(),
                ),
            )]),
        );

        assert!(watcher.clear_pull_conflict(&ns, &status).await);

        let after = rx.borrow().clone();
        assert!(after.error.is_none());
        assert_eq!(after.mode, TrayMode::Idle);
        assert_eq!(after.pending_changes, 1);
    }
}
