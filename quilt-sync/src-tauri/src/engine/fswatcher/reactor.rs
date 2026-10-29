use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use futures_util::stream::FuturesUnordered;
use quilt_uri::Namespace;
use tokio::sync::mpsc;
use tokio::time::Instant;

use crate::engine::autopull::PackageStatusEvent;
use crate::engine::autopull::StatusReporter;
use crate::engine::autopull::WatcherInner;
use crate::engine::autopull::last_change::PendingCommit;
use crate::engine::autopull::reporter::SubscriberErrorEvent;
use crate::engine::fswatcher::settings::SharedFsWatcherSettings;
use crate::engine::fswatcher::subscriber::MappingSignal;
use crate::engine::fswatcher::subscriber::SignalKind;
use crate::engine::fswatcher::subscriber::SubscriberError;
use crate::engine::fswatcher::subscriber::Subscription;
use crate::engine::fswatcher::subscriber::WatchedPackage;
use crate::engine::model::Model;
use crate::engine::model::QuiltModel;
use crate::engine::quilt;
use crate::engine::telemetry::prelude::*;

/// How often the reactor re-snapshots the installed-packages list and
/// reconciles its subscription set. Bounds how long a freshly-installed
/// package waits before its first edit fires an event, and how long a
/// just-uninstalled package keeps a (harmless) live watch.
const RECONCILE_INTERVAL: Duration = Duration::from_secs(5);

/// How long a package must be quiet after a signal before it is walked.
const QUIET: Duration = Duration::from_secs(2);

pub(crate) struct ReactorState {
    pub settings: SharedFsWatcherSettings,
    pub reporter: Arc<dyn StatusReporter>,
    pub signal_rx: mpsc::UnboundedReceiver<MappingSignal>,
    pub subscription: Subscription,
    pub trailing: TrailingWalks,
    /// Whether each watched home is on a local volume, read once per home.
    pub local_volume: BTreeMap<Namespace, (PathBuf, bool)>,
    /// Kind of the last `Err` returned by `reconcile`. A failed `add()`
    /// doesn't insert into the subscription's watched set, so the next
    /// reconcile retries the same namespaces and fails the same way; this
    /// marker keeps us from emitting a fresh `inotify_limit` toast every
    /// 5 s. Cleared whenever a reconcile returns `Ok`.
    pub last_reconcile_error_kind: Option<&'static str>,
}

/// When each package is walked next: a quiet time after its last signal, and
/// never while it is already being walked. A signal during a walk sets the
/// next one after it, so a burst gives one walk and none is lost.
pub(crate) struct TrailingWalks {
    quiet: Duration,
    due: BTreeMap<Namespace, Instant>,
    walking: BTreeSet<Namespace>,
}

impl Default for TrailingWalks {
    fn default() -> Self {
        Self::with_quiet(QUIET)
    }
}

impl TrailingWalks {
    pub(crate) fn with_quiet(quiet: Duration) -> Self {
        Self {
            quiet,
            due: BTreeMap::new(),
            walking: BTreeSet::new(),
        }
    }

    pub(crate) fn on_signal(&mut self, namespace: &Namespace, now: Instant) {
        self.due.insert(namespace.clone(), now + self.quiet);
    }

    /// The earliest walk that can start, if any.
    pub(crate) fn next_due(&self) -> Option<Instant> {
        self.due
            .iter()
            .filter(|(ns, _)| !self.walking.contains(*ns))
            .map(|(_, at)| *at)
            .min()
    }

    /// The packages to walk now, marked as walking.
    pub(crate) fn take_due(&mut self, now: Instant) -> Vec<Namespace> {
        let ready: Vec<Namespace> = self
            .due
            .iter()
            .filter(|(ns, at)| **at <= now && !self.walking.contains(*ns))
            .map(|(ns, _)| ns.clone())
            .collect();
        for namespace in &ready {
            self.due.remove(namespace);
            self.walking.insert(namespace.clone());
        }
        ready
    }

    pub(crate) fn walked(&mut self, namespace: &Namespace) {
        self.walking.remove(namespace);
    }

    /// Drop the pending walks; the ones running finish.
    fn clear(&mut self) {
        self.due.clear();
    }

    fn retain(&mut self, live: &BTreeSet<Namespace>) {
        self.due.retain(|ns, _| live.contains(ns));
    }
}

pub(crate) async fn run(mut state: ReactorState, model: Arc<Model>, watcher: Arc<WatcherInner>) {
    drive(&mut state, &*model, &watcher).await;
}

/// The reactor's loop: reconcile the watches, take signals, and walk each
/// package once its signals stop. Walks run beside the loop, so a signal is
/// taken while another package, or the same one, is being walked.
async fn drive(state: &mut ReactorState, model: &impl QuiltModel, inner: &WatcherInner) {
    let mut reconcile_tick = tokio::time::interval(RECONCILE_INTERVAL);
    // The first tick fires immediately; drop it because `FsWatcher::spawn`
    // already did a synchronous initial reconcile, so the next periodic
    // reconcile should run after a full interval.
    reconcile_tick.tick().await;
    sync_vouched(state, inner);
    let mut walks = FuturesUnordered::new();
    loop {
        let next_due = state.trailing.next_due();
        tokio::select! {
            biased;
            _ = reconcile_tick.tick() => {
                let enabled = state.settings.read().await.enabled;
                if enabled {
                    reconcile_from_model(state, model, inner).await;
                } else {
                    // Disabled: drain any live watches so the OS releases
                    // the inotify slots. Especially important when the user
                    // toggles off in response to the inotify-limit toast.
                    // `reconcile(Vec::new())` is a no-op if already empty.
                    if let Err(err) = state.subscription.reconcile(Vec::new(), None) {
                        emit_subscriber_error(state.reporter.as_ref(), &err);
                    }
                    inner.walks.clear();
                    state.trailing.clear();
                    // Re-enabling later should be able to surface the
                    // inotify-limit toast again if the limit still applies.
                    state.last_reconcile_error_kind = None;
                }
            }
            Some(namespace) = walks.next(), if !walks.is_empty() => {
                state.trailing.walked(&namespace);
            }
            Some(signal) = state.signal_rx.recv() => {
                if !state.settings.read().await.enabled {
                    inner.walks.forget(&signal.namespace);
                    continue;
                }
                handle_signal(state, inner, &signal, Instant::now()).await;
            }
            () = tokio::time::sleep_until(next_due.unwrap_or_else(Instant::now)), if next_due.is_some() => {
                if !state.settings.read().await.enabled {
                    state.trailing.clear();
                    continue;
                }
                for namespace in state.trailing.take_due(Instant::now()) {
                    let reporter = Arc::clone(&state.reporter);
                    walks.push(async move {
                        process_signal(
                            model,
                            reporter.as_ref(),
                            Some(inner),
                            MappingSignal::changed(namespace.clone()),
                        )
                        .await;
                        namespace
                    });
                }
            }
            else => break,
        }
    }
}

/// One signal. A change moves the package's last-change time at once, with
/// no walk; a change or a rescan sets its trailing walk; a screened-out path
/// leaves the walk to the tick; a rescan or a lost watch stops the tick from
/// trusting the package's walks.
async fn handle_signal(
    state: &mut ReactorState,
    inner: &WatcherInner,
    signal: &MappingSignal,
    now: Instant,
) {
    match signal.kind {
        SignalKind::Changed => {
            inner.last_change.on_signal(&signal.namespace).await;
            inner.walks.on_signal(&signal.namespace);
            state.trailing.on_signal(&signal.namespace, now);
        }
        // Not trusted until its watch is made again and walked after.
        SignalKind::Rescan => {
            inner.walks.forget(&signal.namespace);
            state.trailing.on_signal(&signal.namespace, now);
        }
        SignalKind::Stale => inner.walks.on_signal(&signal.namespace),
        SignalKind::Lost => inner.walks.forget(&signal.namespace),
    }
}

pub(crate) async fn process_signal(
    model: &impl QuiltModel,
    reporter: &dyn StatusReporter,
    // The watcher's shared state, when there is one. `None` in tests that are
    // only about the event this emits.
    clocks: Option<&WatcherInner>,
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
    // Begun before the lineage read and the walk, so a signal that lands
    // during either makes this walk stale for the tick.
    let kept = clocks.map(|inner| (inner, inner.walks.begin(&signal.namespace)));
    let lineage = match kept {
        Some(_) => model.get_installed_package_lineage(&pkg).await.ok(),
        None => None,
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

    // The same walk starts or ends the publish countdown. Without this the
    // card's countdown waits for the next tick while the list already shows
    // the edit.
    if let Some(inner) = clocks {
        let home = if inner.last_change.first_walk_pending(&signal.namespace) {
            pkg.package_home().await.ok()
        } else {
            None
        };
        // Only a tree with no changes needs it: then a commit waiting to be
        // pushed is all that keeps the package's time. The tick reads the
        // commit on its own walk.
        let pending_commit = match lineage.as_ref() {
            Some(lineage) if status.changes.is_empty() => PendingCommit::of(lineage),
            _ => PendingCommit::Unknown,
        };
        if let (Some((_, start)), Some(lineage)) = (kept, lineage) {
            inner
                .walks
                .record(&signal.namespace, start, lineage, &status);
        }
        crate::engine::autopull::observe_walk(
            inner,
            &signal.namespace,
            home.as_deref(),
            &status,
            pending_commit,
        )
        .await;
    }
}

/// What the subscription should watch.
pub(crate) struct Mappings {
    /// Each installed package's namespace, home and files.
    pub packages: Vec<(Namespace, WatchedPackage)>,
    /// The user-wide ignore file. `None` with no packages, since then there is
    /// nothing for it to re-check.
    pub user_rules: Option<PathBuf>,
}

/// Snapshot the current installed-packages list and read each one's
/// `package_home` and files from its lineage. Returns `None` if the model itself fails (so the caller
/// skips reconciliation rather than unwatching every namespace on a
/// transient error); empty `packages` is distinct and means "no
/// installed packages".
pub(crate) async fn snapshot_mappings(model: &impl QuiltModel) -> Option<Mappings> {
    let pkgs = match model.get_installed_packages_list().await {
        Ok(list) => list,
        Err(err) => {
            warn!("fswatcher: snapshot failed: {err}");
            return None;
        }
    };
    let mut out = Vec::with_capacity(pkgs.len());
    for pkg in &pkgs {
        match pkg.lineage.read(&pkg.storage).await {
            Ok((home, lineage)) => out.push((
                pkg.namespace.clone(),
                WatchedPackage::new(
                    home,
                    pkg.paths.user_quiltignore(),
                    lineage.paths.into_keys(),
                ),
            )),
            Err(err) => warn!(
                "fswatcher: cannot read the lineage of {}: {err}",
                pkg.namespace
            ),
        }
    }
    // Every package shares the one domain, so any of them knows the path.
    let user_rules = pkgs.first().map(|pkg| pkg.paths.user_quiltignore());
    Some(Mappings {
        packages: out,
        user_rules,
    })
}

async fn reconcile_from_model(
    state: &mut ReactorState,
    model: &impl QuiltModel,
    inner: &WatcherInner,
) {
    let Some(mappings) = snapshot_mappings(model).await else {
        return;
    };
    // Reconcile is the one place that learns a package is gone, so it is where
    // the per-namespace state is pruned. Left alone, it would keep an entry per
    // package ever watched in the session.
    let live: BTreeSet<Namespace> = mappings.packages.iter().map(|(ns, _)| ns.clone()).collect();
    state.trailing.retain(&live);
    note_volumes(state, &mappings.packages);
    match state
        .subscription
        .reconcile(mappings.packages, mappings.user_rules)
    {
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
    sync_vouched(state, inner);
}

/// Read whether each new home is on a local volume, and forget the rest.
pub(crate) fn note_volumes(state: &mut ReactorState, mappings: &[(Namespace, WatchedPackage)]) {
    state
        .local_volume
        .retain(|ns, _| mappings.iter().any(|(live, _)| live == ns));
    for (namespace, package) in mappings {
        // Canonical like the subscription's root, for the mount table.
        let home = &dunce::canonicalize(&package.home).unwrap_or_else(|_| package.home.clone());
        if state
            .local_volume
            .get(namespace)
            .is_none_or(|(known, _)| known != home)
        {
            let local = quilt::file_stamp::on_local_volume(home);
            if !local {
                info!("fswatcher: {namespace} is not on a local volume, the check walks it");
            }
            state
                .local_volume
                .insert(namespace.clone(), (home.clone(), local));
        }
    }
}

/// Vouch for the packages that are watched and on a local volume, with no
/// network or FUSE volume mounted inside, and only for them. The mount table
/// is read each time, so a mount made later is seen.
fn sync_vouched(state: &ReactorState, inner: &WatcherInner) {
    // An unwatched user-wide ignore file could change what every package
    // holds without a signal.
    let rules_watched = state.subscription.user_rules_watched();
    let vouched: BTreeMap<Namespace, PathBuf> = state
        .subscription
        .watched_namespaces()
        .into_iter()
        .filter(|_| rules_watched)
        .filter_map(|ns| match state.local_volume.get(&ns) {
            Some((home, true)) if !quilt::file_stamp::remote_mount_under(home) => {
                Some((ns, home.clone()))
            }
            _ => None,
        })
        .collect();
    inner.walks.vouch_for(&vouched);
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

    use std::sync::Mutex;

    use crate::engine::autopull::reporter::test_support::RecordingReporter;
    use crate::engine::model::MockQuiltModel;
    use crate::engine::quilt;

    fn ns() -> Namespace {
        ("acme", "demo").into()
    }

    fn state() -> (ReactorState, mpsc::UnboundedSender<MappingSignal>) {
        let reporter: Arc<dyn StatusReporter> = Arc::new(RecordingReporter::default());
        let (tx, rx) = mpsc::unbounded_channel::<MappingSignal>();
        // The subscription's own sender: these tests send signals by hand.
        let (sub_tx, _) = mpsc::unbounded_channel::<MappingSignal>();
        let subscription = Subscription::new(Duration::from_millis(50), sub_tx, &reporter)
            .expect("debouncer should build");
        let state = ReactorState {
            settings: Arc::new(tokio::sync::RwLock::new(
                crate::engine::fswatcher::FsWatcherSettings::default(),
            )),
            reporter,
            signal_rx: rx,
            subscription,
            trailing: TrailingWalks::default(),
            local_volume: BTreeMap::new(),
            last_reconcile_error_kind: None,
        };
        (state, tx)
    }

    #[test]
    fn a_burst_of_signals_gives_one_walk_after_the_last() {
        let mut trailing = TrailingWalks::default();
        let t0 = Instant::now();
        for ms in (0..5_000).step_by(100) {
            trailing.on_signal(&ns(), t0 + Duration::from_millis(ms));
        }
        let last = t0 + Duration::from_millis(4_900);
        assert!(trailing.take_due(last + QUIET / 2).is_empty(), "still busy");
        assert_eq!(trailing.next_due(), Some(last + QUIET));
        assert_eq!(trailing.take_due(last + QUIET), vec![ns()]);
        trailing.walked(&ns());
        assert_eq!(trailing.next_due(), None, "one walk for the whole burst");
    }

    #[test]
    fn signals_during_a_walk_give_one_more_walk_after_it() {
        let mut trailing = TrailingWalks::default();
        let t0 = Instant::now();
        trailing.on_signal(&ns(), t0);
        assert_eq!(trailing.take_due(t0 + QUIET), vec![ns()]);

        // The walk takes long; signals keep coming.
        for s in 1..20 {
            trailing.on_signal(&ns(), t0 + QUIET + Duration::from_millis(s * 10));
        }
        let late = t0 + QUIET * 3;
        assert_eq!(trailing.next_due(), None, "never two walks of one package");
        assert_eq!(trailing.take_due(late), Vec::<Namespace>::new());

        trailing.walked(&ns());
        assert_eq!(
            trailing.take_due(late),
            vec![ns()],
            "the signals are not lost"
        );
        trailing.walked(&ns());
        assert_eq!(trailing.next_due(), None);
    }

    #[test]
    fn edits_a_minute_apart_each_get_a_walk() {
        let mut trailing = TrailingWalks::default();
        let t0 = Instant::now();
        for minute in 0..3 {
            let at = t0 + Duration::from_secs(60 * minute);
            trailing.on_signal(&ns(), at);
            assert_eq!(trailing.take_due(at + QUIET), vec![ns()]);
            trailing.walked(&ns());
        }
    }

    #[test]
    fn one_busy_package_does_not_hold_back_another() {
        let mut trailing = TrailingWalks::default();
        let other: Namespace = ("acme", "other").into();
        let t0 = Instant::now();
        trailing.on_signal(&ns(), t0);
        trailing.on_signal(&other, t0);
        for s in 1..10 {
            trailing.on_signal(&ns(), t0 + Duration::from_secs(s));
        }
        assert_eq!(trailing.take_due(t0 + QUIET), vec![other]);
    }

    /// A change moves the last-change time and makes the kept walk stale, with
    /// no walk: `handle_signal` has no model to walk with.
    #[tokio::test]
    async fn a_change_moves_the_last_change_and_schedules_a_walk() {
        let (mut state, _tx) = state();
        let inner = crate::engine::autopull::inner_for_tests(
            crate::engine::autopull::AutosyncSettings::default(),
        );
        inner
            .walks
            .vouch_for(&BTreeMap::from([(ns(), PathBuf::from("/pkg"))]));
        let start = inner.walks.begin(&ns());
        inner.walks.record(
            &ns(),
            start,
            quilt::lineage::PackageLineage::default(),
            &quilt::lineage::InstalledPackageStatus::default(),
        );
        let now = Instant::now();

        handle_signal(&mut state, &inner, &MappingSignal::changed(ns()), now).await;

        assert!(inner.last_change.get(&ns()).await.is_some());
        assert!(inner.walks.reusable(&ns()).is_none());
        assert_eq!(state.trailing.next_due(), Some(now + QUIET));
    }

    #[tokio::test]
    async fn a_rescan_walks_without_moving_the_last_change() {
        let (mut state, _tx) = state();
        let inner = crate::engine::autopull::inner_for_tests(
            crate::engine::autopull::AutosyncSettings::default(),
        );
        let signal = MappingSignal {
            namespace: ns(),
            kind: SignalKind::Rescan,
        };
        let now = Instant::now();

        handle_signal(&mut state, &inner, &signal, now).await;

        assert!(inner.last_change.get(&ns()).await.is_none());
        assert_eq!(state.trailing.next_due(), Some(now + QUIET));
    }

    #[tokio::test]
    async fn a_lost_watch_stops_the_check_trusting_the_package() {
        let (mut state, _tx) = state();
        let inner = crate::engine::autopull::inner_for_tests(
            crate::engine::autopull::AutosyncSettings::default(),
        );
        inner
            .walks
            .vouch_for(&BTreeMap::from([(ns(), PathBuf::from("/pkg"))]));
        let start = inner.walks.begin(&ns());
        inner.walks.record(
            &ns(),
            start,
            quilt::lineage::PackageLineage::default(),
            &quilt::lineage::InstalledPackageStatus::default(),
        );
        let signal = MappingSignal {
            namespace: ns(),
            kind: SignalKind::Lost,
        };

        handle_signal(&mut state, &inner, &signal, Instant::now()).await;

        assert!(inner.walks.reusable(&ns()).is_none());
        assert_eq!(state.trailing.next_due(), None);
    }

    /// A model whose walks return `statuses` in turn and count themselves.
    fn walking_model(
        statuses: Vec<BTreeMap<PathBuf, quilt::lineage::Change>>,
        walks: Arc<Mutex<usize>>,
    ) -> MockQuiltModel {
        let mut model = MockQuiltModel::new();
        model
            .expect_get_installed_packages_list()
            .returning(|| Ok(Vec::new()));
        model
            .expect_get_installed_package()
            .returning(|_| Ok(Some(fresh_pkg())));
        model
            .expect_get_installed_package_lineage()
            .returning(|_| Ok(quilt::lineage::PackageLineage::default()));
        let statuses = Mutex::new(statuses.into_iter());
        model
            .expect_recompute_local_status()
            .returning(move |_, _| {
                *walks.lock().unwrap() += 1;
                let changes = statuses.lock().unwrap().next().unwrap_or_default();
                Ok(quilt::lineage::InstalledPackageStatus::new(
                    quilt::lineage::UpstreamState::UpToDate,
                    changes,
                ))
            });
        model
    }

    /// Runs the reactor beside `script` until the script ends.
    async fn drive_with(
        state: &mut ReactorState,
        model: &MockQuiltModel,
        inner: &WatcherInner,
        script: impl std::future::Future<Output = ()>,
    ) {
        tokio::select! {
            () = drive(state, model, inner) => panic!("the reactor stopped"),
            () = script => {}
        }
    }

    /// Short enough for a test in real time; the burst tests send faster.
    const TEST_QUIET: Duration = Duration::from_millis(200);

    /// The owner's case: an edit, then an undo. Each shows on the page a
    /// quiet time later, not at the next check.
    #[tokio::test]
    async fn an_edit_and_its_undo_each_show_within_seconds() {
        let (mut state, tx) = state();
        state.trailing = TrailingWalks::with_quiet(TEST_QUIET);
        let walks = Arc::new(Mutex::new(0));
        let model = walking_model(
            vec![changes_with_one_file("added"), BTreeMap::new()],
            Arc::clone(&walks),
        );
        let reporter = Arc::new(RecordingReporter::default());
        state.reporter = reporter.clone();
        let inner = crate::engine::autopull::inner_for_tests(
            crate::engine::autopull::AutosyncSettings::default(),
        );

        drive_with(&mut state, &model, &inner, async {
            tx.send(MappingSignal::changed(ns())).unwrap();
            tokio::time::sleep(TEST_QUIET * 4).await;
            {
                let statuses = reporter.statuses.lock().unwrap();
                assert_eq!(statuses.len(), 1, "the edit shows");
                assert!(statuses[0].1.has_changes);
            }
            tx.send(MappingSignal::changed(ns())).unwrap();
            tokio::time::sleep(TEST_QUIET * 4).await;
            let statuses = reporter.statuses.lock().unwrap();
            assert_eq!(statuses.len(), 2, "the undo shows");
            assert!(!statuses[1].1.has_changes);
        })
        .await;
        assert_eq!(*walks.lock().unwrap(), 2);
    }

    #[tokio::test]
    async fn a_burst_of_many_events_gives_one_walk() {
        let (mut state, tx) = state();
        state.trailing = TrailingWalks::with_quiet(TEST_QUIET);
        let walks = Arc::new(Mutex::new(0));
        let model = walking_model(Vec::new(), Arc::clone(&walks));
        let inner = crate::engine::autopull::inner_for_tests(
            crate::engine::autopull::AutosyncSettings::default(),
        );

        drive_with(&mut state, &model, &inner, async {
            for _ in 0..30 {
                tx.send(MappingSignal::changed(ns())).unwrap();
                tokio::time::sleep(TEST_QUIET / 10).await;
            }
            tokio::time::sleep(TEST_QUIET * 4).await;
        })
        .await;
        assert_eq!(*walks.lock().unwrap(), 1);
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
            MappingSignal::changed(("acme", "demo").into()),
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
            MappingSignal::changed(ns.clone()),
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
        let signal = MappingSignal::changed(ns.clone());

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
        let signal = MappingSignal::changed(ns.clone());

        process_signal(&model, reporter.as_ref(), None, signal.clone()).await;
        process_signal(&model, reporter.as_ref(), None, signal).await;

        assert_eq!(reporter.statuses.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn an_edit_moves_the_publish_countdown_without_waiting_for_a_tick() {
        // `observe_walk` is tested on its own in `autopull`; this tests that
        // `process_signal` calls it. Without the call only the tick starts the
        // countdown, so it lags the tree by a cadence.
        let ns: quilt_uri::Namespace = ("acme", "demo").into();
        let mut model = MockQuiltModel::new();
        model
            .expect_get_installed_package()
            .returning(|_| Ok(Some(fresh_pkg())));
        model
            .expect_get_installed_package_lineage()
            .returning(|_| Ok(quilt::lineage::PackageLineage::default()));
        model.expect_recompute_local_status().returning(|_, _| {
            Ok(quilt::lineage::InstalledPackageStatus::new(
                quilt::lineage::UpstreamState::UpToDate,
                changes_with_one_file("added"),
            ))
        });
        let reporter = Arc::new(RecordingReporter::default());
        let mut settings = crate::engine::autopull::AutosyncSettings::default();
        settings.push.enabled = true;
        let inner = crate::engine::autopull::inner_for_tests(settings);

        process_signal(
            &model,
            reporter.as_ref(),
            Some(&inner),
            MappingSignal::changed(ns.clone()),
        )
        .await;

        assert!(
            inner.clocks.publish_waiting.read().await.contains(&ns),
            "the file watcher heard the edit and the clock did not"
        );
    }
}
