use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

use chrono::DateTime;
use chrono::Utc;
use quilt_uri::Host;
use quilt_uri::Namespace;
use serde::Deserialize;
use serde::Serialize;
use tokio::sync::watch;

/// App-wide autosync state, folded across namespaces. Pushed to the
/// tray via a `tokio::sync::watch` channel owned by the watcher.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncTrayStatus {
    pub mode: TrayMode,
    pub last_sync: Option<DateTime<Utc>>,
    /// Count of namespaces with `has_changes = true` after the most
    /// recent observation. Cheaper than counting files; for v1 the
    /// tray surfaces it as "N package(s) with changes" in the tooltip.
    pub pending_changes: u32,
    /// Latest non-empty error message across namespaces.
    /// Sticky per namespace — cleared when the namespace clears.
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TrayMode {
    #[default]
    Idle,
    Syncing,
    Paused,
    Error,
}

/// Per-namespace error category that contributes to the tray mode.
/// Insertion order is preserved by recording an incrementing `seq` so
/// the latest non-empty error wins regardless of `BTreeMap` ordering.
#[derive(Clone, Debug)]
struct ErrorEntry {
    kind: ErrorKind,
    message: String,
    seq: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ErrorKind {
    /// A namespace is in `WatcherInner.paused` — known autosync refusal.
    Paused,
    /// A namespace is login-blocked or hit a hard error. Maps to
    /// `TrayMode::Error`.
    Hard,
}

/// Folds per-namespace events into a single `SyncTrayStatus` and
/// broadcasts updates via a `watch::Sender`. Owned by `WatcherInner`.
pub struct SyncTrayAggregator {
    tx: watch::Sender<SyncTrayStatus>,
    state: Mutex<AggregatorState>,
    /// True while a tick is in flight. Surfaced as `TrayMode::Syncing`
    /// when no errors / paused entries dominate.
    tick_in_progress: AtomicBool,
    /// Namespaces with a pull applying — writing working files — right now.
    /// Narrower than `tick_in_progress`, which also spans the read-only
    /// classify and manifest fetch, and deliberately absent from the tray
    /// mode: its readers are the quit prompt and the tick's pause gate, and
    /// neither should act when nothing is at risk.
    ///
    /// Per-namespace rather than a flag, because the two readers ask different
    /// questions: the prompt asks whether *anything* is being written, the
    /// tick asks about the one package it just classified.
    ///
    /// Counted, not a set: a manual pull and the tick's can write the same
    /// package at once — nothing serializes them — and with one entry apiece
    /// whichever finished first would clear a mark the other still needed.
    ///
    /// Each entry also carries an **epoch**, bumped whenever a write starts,
    /// and the entry outlives the writes it counted. "Is one running now?" is
    /// not enough for a reader that has to judge work it did *earlier*: a
    /// short apply can overlap that work and finish before the question is
    /// asked. Comparing epochs across the span answers "did one happen?".
    applying: Mutex<BTreeMap<Namespace, ApplyState>>,
}

/// Marks a namespace as being written until dropped — on return, on `?`, or on
/// unwind.
pub struct ApplyGuard<'a> {
    applying: &'a Mutex<BTreeMap<Namespace, ApplyState>>,
    namespace: Namespace,
}

impl Drop for ApplyGuard<'_> {
    fn drop(&mut self) {
        let mut applying = self.applying.lock().expect("aggregator lock");
        if let Some(state) = applying.get_mut(&self.namespace) {
            state.active = state.active.saturating_sub(1);
        }
        // The entry stays: its epoch is the record that a write happened, which
        // a reader judging earlier work still needs. It is swept when the
        // package stops being tracked — see `retain_namespaces` — so the map
        // follows the installed set rather than every namespace ever written.
    }
}

/// Per-namespace write tracking: how many writes are in flight, and how many
/// have ever started.
#[derive(Default)]
struct ApplyState {
    active: usize,
    epoch: u64,
}

#[derive(Default)]
struct AggregatorState {
    errors: BTreeMap<Namespace, ErrorEntry>,
    dirty: BTreeMap<Namespace, bool>,
    last_sync: Option<DateTime<Utc>>,
    next_seq: u64,
}

impl SyncTrayAggregator {
    pub fn new(tx: watch::Sender<SyncTrayStatus>) -> Self {
        Self {
            tx,
            state: Mutex::new(AggregatorState::default()),
            tick_in_progress: AtomicBool::new(false),
            applying: Mutex::new(BTreeMap::new()),
        }
    }

    /// Whether *any* package is being written right now — the quit prompt's
    /// question. Synchronous, because its caller is a menu/window event
    /// handler that cannot await.
    pub fn apply_in_progress(&self) -> bool {
        self.applying
            .lock()
            .expect("aggregator lock")
            .values()
            .any(|state| state.active > 0)
    }

    /// Whether *this* package is being written right now — the tick's
    /// question, asked of the namespace whose verdict it is about to act on.
    pub fn is_applying(&self, namespace: &Namespace) -> bool {
        self.applying
            .lock()
            .expect("aggregator lock")
            .get(namespace)
            .is_some_and(|state| state.active > 0)
    }

    /// How many writes to this package have ever started. Sample it before the
    /// work whose result depends on a still tree, compare it after: unchanged
    /// *and* nothing running now is the only case where nothing wrote
    /// underneath you.
    pub fn apply_epoch(&self, namespace: &Namespace) -> u64 {
        self.applying
            .lock()
            .expect("aggregator lock")
            .get(namespace)
            .map_or(0, |state| state.epoch)
    }

    /// Bracket the apply for as long as the guard lives. A guard rather than a
    /// pair of calls because the clear has to survive an unwind: a flag left
    /// set would prompt on every quit thereafter, which is a worse failure than
    /// never prompting at all.
    ///
    /// No `publish()`: this flag is deliberately not part of the tray mode —
    /// adding it would change what the icon shows for a reason that has nothing
    /// to do with the icon.
    pub fn apply_guard(&self, namespace: &Namespace) -> ApplyGuard<'_> {
        {
            let mut applying = self.applying.lock().expect("aggregator lock");
            let state = applying.entry(namespace.clone()).or_default();
            state.active += 1;
            state.epoch += 1;
        }
        ApplyGuard {
            applying: &self.applying,
            namespace: namespace.clone(),
        }
    }

    pub fn note_tick_started(&self) {
        self.tick_in_progress.store(true, Ordering::SeqCst);
        self.publish();
    }

    pub fn note_tick_ended_ok(&self) {
        self.tick_in_progress.store(false, Ordering::SeqCst);
        {
            let mut state = self.state.lock().expect("aggregator lock");
            state.last_sync = Some(Utc::now());
        }
        self.publish();
    }

    pub fn note_tick_ended_err(&self) {
        // The loop logs the err. Don't bump last_sync; mode falls back
        // to whatever the per-namespace state dictates.
        self.tick_in_progress.store(false, Ordering::SeqCst);
        self.publish();
    }

    /// Update the per-namespace dirty bit. Does **not** touch the error
    /// map — call [`Self::clear_error`] on the success path or
    /// [`Self::note_paused`] / [`Self::note_login_required`] alongside
    /// it on the failure path.
    pub fn note_status(&self, ns: &Namespace, has_changes: bool) {
        {
            let mut state = self.state.lock().expect("aggregator lock");
            state.dirty.insert(ns.clone(), has_changes);
        }
        self.publish();
    }

    pub fn note_paused(&self, ns: &Namespace, message: &str) {
        self.upsert_error(ns, ErrorKind::Paused, message.to_string());
    }

    pub fn note_login_required(&self, ns: &Namespace, host: Option<Host>) {
        let message = match host {
            Some(h) => format!("Login required for {h}"),
            None => "Login required".to_string(),
        };
        self.upsert_error(ns, ErrorKind::Hard, message);
    }

    /// Drop only the per-namespace error entry. Use this on the
    /// success path; the dirty count is left intact so a clean refresh
    /// of a package with local changes still surfaces them in the
    /// tooltip.
    pub fn clear_error(&self, ns: &Namespace) {
        {
            let mut state = self.state.lock().expect("aggregator lock");
            state.errors.remove(ns);
        }
        self.publish();
    }

    /// Drop both the error and dirty entries for a namespace. Used by
    /// the user-initiated unpause path.
    pub fn note_cleared(&self, ns: &Namespace) {
        {
            let mut state = self.state.lock().expect("aggregator lock");
            state.errors.remove(ns);
            state.dirty.remove(ns);
        }
        self.publish();
    }

    /// Drop every namespace that is **not** in `keep`. Called once per
    /// tick so the aggregator's per-namespace maps stay in lockstep
    /// with the installed package set — without this, an uninstalled
    /// package would leave the tray stuck in `Paused`/`Error` mode or
    /// keep its dirty bit counted in `pending_changes`.
    pub fn retain_namespaces(&self, keep: &std::collections::BTreeSet<Namespace>) {
        {
            let mut state = self.state.lock().expect("aggregator lock");
            state.errors.retain(|ns, _| keep.contains(ns));
            state.dirty.retain(|ns, _| keep.contains(ns));
        }
        // Apply history goes the same way, except never while a write is in
        // flight: an entry is kept past its guard so the epoch survives, but a
        // package the user has uninstalled has no reader left. A pruned epoch
        // reads as 0, which fails any comparison against a sampled non-zero
        // one — so the sweep can only ever make a reader distrust a verdict,
        // never trust a stale one.
        self.applying
            .lock()
            .expect("aggregator lock")
            .retain(|ns, state| state.active > 0 || keep.contains(ns));
        self.publish();
    }

    fn upsert_error(&self, ns: &Namespace, kind: ErrorKind, message: String) {
        {
            let mut state = self.state.lock().expect("aggregator lock");
            let seq = state.next_seq;
            state.next_seq = state.next_seq.saturating_add(1);
            state
                .errors
                .insert(ns.clone(), ErrorEntry { kind, message, seq });
        }
        self.publish();
    }

    fn publish(&self) {
        let state = self.state.lock().expect("aggregator lock");
        let any_hard = state.errors.values().any(|e| e.kind == ErrorKind::Hard);
        let any_paused = state.errors.values().any(|e| e.kind == ErrorKind::Paused);
        let mode = if any_hard {
            TrayMode::Error
        } else if any_paused {
            TrayMode::Paused
        } else if self.tick_in_progress.load(Ordering::SeqCst) {
            TrayMode::Syncing
        } else {
            TrayMode::Idle
        };
        let error = state
            .errors
            .values()
            .max_by_key(|e| e.seq)
            .map(|e| e.message.clone());
        let pending_changes =
            u32::try_from(state.dirty.values().filter(|v| **v).count()).unwrap_or(u32::MAX);
        let status = SyncTrayStatus {
            mode,
            last_sync: state.last_sync,
            pending_changes,
            error,
        };
        // `send` only fails if all receivers were dropped — we hold one
        // inside the watcher / tray controller, so just log.
        let _ = self.tx.send(status);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use quilt_uri::Host;
    use quilt_uri::Namespace;
    use tokio::sync::watch;

    fn new_aggregator() -> (SyncTrayAggregator, watch::Receiver<SyncTrayStatus>) {
        let (tx, rx) = watch::channel(SyncTrayStatus::default());
        (SyncTrayAggregator::new(tx), rx)
    }

    #[test]
    fn sync_tray_status_default_is_idle() {
        let s = SyncTrayStatus::default();
        assert_eq!(s.mode, TrayMode::Idle);
        assert!(s.last_sync.is_none());
        assert_eq!(s.pending_changes, 0);
        assert!(s.error.is_none());
    }

    #[test]
    fn sync_tray_status_serializes_camel_case() {
        let s = SyncTrayStatus {
            mode: TrayMode::Syncing,
            last_sync: None,
            pending_changes: 3,
            error: None,
        };
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains(r#""mode":"syncing""#), "got: {json}");
        assert!(json.contains(r#""pendingChanges":3"#), "got: {json}");
        assert!(json.contains(r#""lastSync":null"#), "got: {json}");
    }

    #[test]
    fn aggregator_starts_idle() {
        let (_, rx) = new_aggregator();
        let s = rx.borrow().clone();
        assert_eq!(s.mode, TrayMode::Idle);
    }

    #[test]
    fn tick_started_then_ended_clean_returns_to_idle_and_sets_last_sync() {
        let (agg, rx) = new_aggregator();
        agg.note_tick_started();
        assert_eq!(rx.borrow().mode, TrayMode::Syncing);
        agg.note_tick_ended_ok();
        let after = rx.borrow().clone();
        assert_eq!(after.mode, TrayMode::Idle);
        assert!(after.last_sync.is_some());
    }

    #[test]
    fn note_paused_with_known_reason_promotes_mode_to_paused() {
        let (agg, rx) = new_aggregator();
        let ns: Namespace = ("acme", "demo").into();
        agg.note_paused(&ns, "pending changes");
        let after = rx.borrow().clone();
        assert_eq!(after.mode, TrayMode::Paused);
        assert_eq!(after.error.as_deref(), Some("pending changes"));
    }

    #[test]
    fn note_login_required_promotes_mode_to_error() {
        let (agg, rx) = new_aggregator();
        let ns: Namespace = ("acme", "demo").into();
        let host: Host = "catalog.dev".parse().unwrap();
        agg.note_login_required(&ns, Some(host));
        let after = rx.borrow().clone();
        assert_eq!(after.mode, TrayMode::Error);
        assert!(after.error.as_deref().unwrap().contains("catalog.dev"));
    }

    #[test]
    fn error_precedence_beats_paused_beats_syncing_beats_idle() {
        let (agg, rx) = new_aggregator();
        let ns_a: Namespace = ("acme", "a").into();
        let ns_b: Namespace = ("acme", "b").into();
        agg.note_paused(&ns_a, "pending changes");
        agg.note_login_required(&ns_b, None);
        // Error wins over Paused, even though tick is not running.
        assert_eq!(rx.borrow().mode, TrayMode::Error);
        // Starting a tick while errors exist does NOT downgrade the mode.
        agg.note_tick_started();
        assert_eq!(rx.borrow().mode, TrayMode::Error);
    }

    #[test]
    fn note_cleared_drops_namespace_and_recomputes() {
        let (agg, rx) = new_aggregator();
        let ns: Namespace = ("acme", "demo").into();
        agg.note_paused(&ns, "pending changes");
        assert_eq!(rx.borrow().mode, TrayMode::Paused);
        agg.note_cleared(&ns);
        assert_eq!(rx.borrow().mode, TrayMode::Idle);
        assert!(rx.borrow().error.is_none());
    }

    #[test]
    fn note_status_sets_pending_changes_count() {
        let (agg, rx) = new_aggregator();
        let ns_a: Namespace = ("acme", "a").into();
        let ns_b: Namespace = ("acme", "b").into();
        let ns_c: Namespace = ("acme", "c").into();
        agg.note_status(&ns_a, true);
        agg.note_status(&ns_b, true);
        agg.note_status(&ns_c, false);
        assert_eq!(rx.borrow().pending_changes, 2);
        // Flipping ns_b back to clean decrements.
        agg.note_status(&ns_b, false);
        assert_eq!(rx.borrow().pending_changes, 1);
    }

    #[test]
    fn latest_error_wins() {
        let (agg, rx) = new_aggregator();
        let ns_a: Namespace = ("acme", "a").into();
        let ns_b: Namespace = ("acme", "b").into();
        agg.note_paused(&ns_a, "first");
        agg.note_paused(&ns_b, "second");
        assert_eq!(rx.borrow().error.as_deref(), Some("second"));
        // Clearing ns_b falls back to ns_a's message.
        agg.note_cleared(&ns_b);
        assert_eq!(rx.borrow().error.as_deref(), Some("first"));
    }

    #[test]
    fn note_status_does_not_drop_pause() {
        // The Conflict arm in tick.rs calls note_paused followed by
        // note_status. note_status must update the dirty count without
        // clearing the sticky pause set just before it.
        let (agg, rx) = new_aggregator();
        let ns: Namespace = ("acme", "demo").into();
        agg.note_paused(&ns, "stuck");
        agg.note_status(&ns, true);
        let after = rx.borrow().clone();
        assert_eq!(after.mode, TrayMode::Paused);
        assert_eq!(after.error.as_deref(), Some("stuck"));
        assert_eq!(after.pending_changes, 1);
    }

    #[test]
    fn clear_error_drops_error_but_keeps_dirty() {
        let (agg, rx) = new_aggregator();
        let ns: Namespace = ("acme", "demo").into();
        agg.note_paused(&ns, "stuck");
        agg.note_status(&ns, true);
        agg.clear_error(&ns);
        let after = rx.borrow().clone();
        assert_eq!(after.mode, TrayMode::Idle);
        assert!(after.error.is_none());
        assert_eq!(
            after.pending_changes, 1,
            "dirty bit must survive a clear_error call",
        );
    }

    // The apply epoch has to outlive the guard that bumped it, so entries are
    // not removed when a write finishes. That is not a licence to keep them
    // forever: a package the user uninstalls should take its entry with it,
    // the same way its error and dirty entries go.
    #[test]
    fn retain_namespaces_drops_a_finished_apply_but_keeps_one_still_writing() {
        use std::collections::BTreeSet;

        let (agg, _rx) = new_aggregator();
        let gone: Namespace = ("acme", "uninstalled").into();
        let writing: Namespace = ("acme", "mid-write").into();

        drop(agg.apply_guard(&gone));
        let _in_flight = agg.apply_guard(&writing);
        assert_ne!(
            agg.apply_epoch(&gone),
            0,
            "the finished write left its epoch"
        );

        agg.retain_namespaces(&BTreeSet::from([writing.clone()]));

        assert_eq!(
            agg.apply_epoch(&gone),
            0,
            "an uninstalled package must not keep its apply history"
        );
        assert!(
            agg.is_applying(&writing),
            "a write still in flight must survive the sweep"
        );
    }

    #[test]
    fn retain_namespaces_drops_orphan_errors_and_dirty() {
        use std::collections::BTreeSet;

        let (agg, rx) = new_aggregator();
        let ns_keep: Namespace = ("acme", "keep").into();
        let ns_paused_drop: Namespace = ("acme", "uninstalled-paused").into();
        let ns_dirty_drop: Namespace = ("acme", "uninstalled-dirty").into();

        agg.note_paused(&ns_keep, "still here");
        agg.note_status(&ns_keep, true);
        agg.note_paused(&ns_paused_drop, "stale");
        agg.note_status(&ns_dirty_drop, true);
        assert_eq!(rx.borrow().pending_changes, 2);
        assert_eq!(rx.borrow().mode, TrayMode::Paused);

        let mut keep = BTreeSet::new();
        keep.insert(ns_keep.clone());
        agg.retain_namespaces(&keep);

        let after = rx.borrow().clone();
        assert_eq!(
            after.error.as_deref(),
            Some("still here"),
            "uninstalled namespaces must not leak their pause message",
        );
        assert_eq!(
            after.pending_changes, 1,
            "orphan dirty entries must be dropped on reconciliation",
        );
    }
}
