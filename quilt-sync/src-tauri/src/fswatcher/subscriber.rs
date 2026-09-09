use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fmt;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use ignore::gitignore::Gitignore;
use notify::ErrorKind;
use notify::EventKind;
use notify::RecommendedWatcher;
use notify::RecursiveMode;
use notify::event::ModifyKind;
use notify_debouncer_full::Debouncer;
use notify_debouncer_full::RecommendedCache;
use notify_debouncer_full::new_debouncer;
use quilt_uri::Namespace;
use tokio::sync::mpsc;

use crate::autopull::StatusReporter;
use crate::autopull::reporter::SubscriberErrorEvent;
use crate::fswatcher::filter;
use crate::quilt;
use crate::telemetry::prelude::*;

/// Categorized error from the OS-level subscription.
#[derive(Debug)]
pub enum SubscriberError {
    /// `notify` could not start a recursive watch — probably the
    /// `fs.inotify.max_user_watches` limit on Linux. Surfaced to the UI
    /// once per subscribe attempt.
    InotifyLimit(notify::Error),
    /// A watch we previously held was dropped (unmount, deleted root).
    WatchLost {
        namespace: Namespace,
        error: notify::Error,
    },
    /// Anything else.
    Other(notify::Error),
}

impl SubscriberError {
    pub fn kind_str(&self) -> &'static str {
        match self {
            Self::InotifyLimit(_) => "inotify_limit",
            Self::WatchLost { .. } => "watch_lost",
            Self::Other(_) => "other",
        }
    }

    pub fn namespace(&self) -> Option<&Namespace> {
        match self {
            Self::WatchLost { namespace, .. } => Some(namespace),
            _ => None,
        }
    }

    pub fn message(&self) -> String {
        match self {
            Self::InotifyLimit(e) | Self::Other(e) => e.to_string(),
            Self::WatchLost { error, .. } => error.to_string(),
        }
    }
}

impl fmt::Display for SubscriberError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InotifyLimit(e) => write!(f, "inotify limit: {e}"),
            Self::WatchLost { namespace, error } => {
                write!(f, "watch lost for {namespace}: {error}")
            }
            Self::Other(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for SubscriberError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InotifyLimit(e) | Self::Other(e) => Some(e),
            Self::WatchLost { error, .. } => Some(error),
        }
    }
}

fn classify(err: notify::Error, namespace: Option<&Namespace>) -> SubscriberError {
    let is_inotify_limit = matches!(&err.kind, ErrorKind::Io(io) if io.raw_os_error() == Some(28))
        || err.to_string().to_lowercase().contains("no space left");
    if is_inotify_limit {
        return SubscriberError::InotifyLimit(err);
    }
    match namespace {
        Some(ns) => SubscriberError::WatchLost {
            namespace: ns.clone(),
            error: err,
        },
        None => SubscriberError::Other(err),
    }
}

/// The ignore file's name, which is the one path it can never exclude.
const QUILTIGNORE_FILE: &str = ".quiltignore";

/// One signal per debounce-flush per affected namespace. The reactor consumes
/// these from an mpsc channel.
#[derive(Debug, Clone)]
pub struct MappingSignal {
    pub namespace: Namespace,
}

/// A watched package: where it lives, and what it asks to have ignored.
///
/// The matcher is cached rather than loaded per event; `add` refreshes it, so a
/// reconcile is the pickup point for an edited `.quiltignore`.
struct WatchedRoot {
    path: PathBuf,
    quiltignore: Option<Gitignore>,
}

impl WatchedRoot {
    /// Whether `path` is excluded by this package's `.quiltignore`.
    ///
    /// Never the ignore file itself: editing it moves the changeset, so it is
    /// always a real change — and that is what keeps a stale matcher
    /// self-correcting.
    fn excludes(&self, path: &Path) -> bool {
        let Some(gi) = self.quiltignore.as_ref() else {
            return false;
        };
        // `matched_path_or_any_parents` panics on a path outside the matcher's
        // root, so the relative key has to come from a checked strip, not from
        // the prefix test that resolved the namespace.
        let Ok(relative) = path.strip_prefix(&self.path) else {
            return false;
        };
        if relative == Path::new(QUILTIGNORE_FILE) {
            return false;
        }
        // `is_dir: false` is safe for a leaf: the matcher tests every *parent*
        // as a directory, so `cache/` matches `cache/x.tmp` without a stat —
        // which a removed path could not supply anyway.
        quilt::quiltignore::is_ignored(gi, relative, false)
    }
}

/// Owns the OS-level subscription and the namespace ↔ root mapping. The
/// debouncer lives behind a `Mutex` because `notify-debouncer-full` runs its
/// own thread that calls back into the event handler; we need shared access
/// from both the `watch`/`unwatch` Tauri thread and the event handler thread.
pub struct Subscription {
    debouncer: Debouncer<RecommendedWatcher, RecommendedCache>,
    watched: Arc<Mutex<BTreeMap<Namespace, WatchedRoot>>>,
}

impl Subscription {
    /// Build a fresh subscription with no watched roots yet.
    ///
    /// `signal_tx` is the mpsc the reactor reads from; signals are forwarded
    /// using `try_send`, so if the reactor falls behind the latest signal is
    /// dropped (acceptable — the next debounce flush will re-emit).
    pub fn new(
        debounce: Duration,
        signal_tx: mpsc::Sender<MappingSignal>,
        reporter: &Arc<dyn StatusReporter>,
    ) -> Result<Self, SubscriberError> {
        let watched: Arc<Mutex<BTreeMap<Namespace, WatchedRoot>>> =
            Arc::new(Mutex::new(BTreeMap::new()));
        let watched_for_cb = Arc::clone(&watched);
        let reporter_for_cb = Arc::clone(reporter);
        let debouncer = new_debouncer(
            debounce,
            None,
            move |result: notify_debouncer_full::DebounceEventResult| match result {
                Ok(events) => {
                    let touched = affected_namespaces(&events, &watched_for_cb.lock().unwrap());
                    for namespace in touched {
                        let signal = MappingSignal {
                            namespace: namespace.clone(),
                        };
                        if let Err(err) = signal_tx.try_send(signal) {
                            debug!("fswatcher: drop signal for {namespace}: {err}");
                        }
                    }
                }
                Err(errors) => {
                    for err in errors {
                        handle_async_error(&watched_for_cb, reporter_for_cb.as_ref(), err);
                    }
                }
            },
        )
        .map_err(|err| classify(err, None))?;
        Ok(Self { debouncer, watched })
    }

    /// Idempotent: a duplicate `add` for the same namespace + root is a no-op.
    /// A different root for the same namespace tears the old watch down first.
    pub fn add(
        &mut self,
        namespace: Namespace,
        package_home: PathBuf,
    ) -> Result<(), SubscriberError> {
        // macOS FSEvents reports canonical paths (/private/var/...); store the
        // canonical root so namespace_for() can match those events. `dunce` is
        // used instead of `std::fs::canonicalize` to avoid `\\?\` UNC prefixes
        // on Windows.
        let package_home = dunce::canonicalize(&package_home).unwrap_or_else(|err| {
            debug!(
                "fswatcher: canonicalize({}) failed, using path as-is: {err}",
                package_home.display(),
            );
            package_home
        });
        // A failure to read `.quiltignore` leaves the matcher absent rather
        // than failing the watch: not screening is the safe direction — it
        // costs walks, where wrongly screening would drop real changes.
        let quiltignore = quilt::quiltignore::load(&package_home).unwrap_or_else(|err| {
            warn!(
                "fswatcher: {}/{QUILTIGNORE_FILE} unreadable, not screening ignored paths: {err}",
                package_home.display(),
            );
            None
        });
        let existing_path = {
            let watched = self.watched.lock().unwrap();
            watched.get(&namespace).map(|w| w.path.clone())
        };
        if let Some(existing) = existing_path.as_ref() {
            if existing == &package_home {
                // The OS watch is already right, so this stays a no-op for
                // `notify` — but the matcher is replaced, which is what makes
                // a reconcile the refresh point for an edited `.quiltignore`.
                if let Some(entry) = self.watched.lock().unwrap().get_mut(&namespace) {
                    entry.quiltignore = quiltignore;
                }
                return Ok(());
            }
            let _ = self.debouncer.unwatch(existing);
        }
        match self
            .debouncer
            .watch(&package_home, RecursiveMode::Recursive)
        {
            Ok(()) => {
                self.watched.lock().unwrap().insert(
                    namespace,
                    WatchedRoot {
                        path: package_home,
                        quiltignore,
                    },
                );
                Ok(())
            }
            Err(err) => Err(classify(err, Some(&namespace))),
        }
    }

    pub fn remove(&mut self, namespace: &Namespace) {
        let entry = self.watched.lock().unwrap().remove(namespace);
        if let Some(entry) = entry {
            let _ = self.debouncer.unwatch(&entry.path);
        }
    }

    /// Incrementally reconcile the watched set with `desired`. Drops
    /// namespaces no longer present; adds (or updates the root of)
    /// namespaces that are. `Subscription::add` is already idempotent
    /// for an unchanged (namespace, path) pair, so a no-op reconcile is
    /// cheap.
    pub fn reconcile(&mut self, desired: Vec<(Namespace, PathBuf)>) -> Result<(), SubscriberError> {
        let desired_map: BTreeMap<Namespace, PathBuf> = desired.into_iter().collect();
        let stale: Vec<Namespace> = self
            .watched
            .lock()
            .unwrap()
            .keys()
            .filter(|ns| !desired_map.contains_key(ns))
            .cloned()
            .collect();
        for ns in &stale {
            self.remove(ns);
        }
        let mut first_err: Option<SubscriberError> = None;
        for (namespace, package_home) in desired_map {
            let ns_display = namespace.to_string();
            if let Err(err) = self.add(namespace, package_home) {
                warn!("fswatcher: reconcile add({ns_display}) failed: {err}");
                if first_err.is_none() {
                    first_err = Some(err);
                }
            }
        }
        match first_err {
            Some(err) => Err(err),
            None => Ok(()),
        }
    }

    #[cfg(test)]
    pub fn watched_namespaces(&self) -> Vec<Namespace> {
        self.watched.lock().unwrap().keys().cloned().collect()
    }
}

/// Resolve which namespaces are affected by a batch of debounced events.
///
/// Status computation reads package files and metadata. Some backends report
/// those reads as `Access` or metadata-only `Modify` events; forwarding them
/// makes the watcher schedule the same expensive status walk again. Quilt
/// manifests track file content and paths, not access times, permissions,
/// ownership, or extended attributes, so every metadata-only variant is safe
/// to ignore. Unknown top-level event kinds still pass through because they may
/// be a backend's only representation of a content mutation.
///
/// A path the package's own `.quiltignore` excludes is dropped too. The walk
/// already skips ignored subtrees, so such an event could only ever buy a
/// recompute that finds nothing — and an ignored path is outside the model in
/// both directions: neither reported, nor a cause of reporting.
fn affected_namespaces(
    events: &[notify_debouncer_full::DebouncedEvent],
    watched: &BTreeMap<Namespace, WatchedRoot>,
) -> BTreeSet<Namespace> {
    let mut touched = BTreeSet::new();
    for event in events {
        if is_non_content_event(event.kind) {
            continue;
        }
        for path in &event.paths {
            if filter::is_ignored(path) {
                continue;
            }
            let Some((namespace, root)) = namespace_for(path, watched) else {
                continue;
            };
            if root.excludes(path) {
                continue;
            }
            touched.insert(namespace.clone());
        }
    }
    touched
}

fn is_non_content_event(kind: EventKind) -> bool {
    matches!(
        kind,
        EventKind::Access(_) | EventKind::Modify(ModifyKind::Metadata(_))
    )
}

fn namespace_for<'a>(
    path: &Path,
    watched: &'a BTreeMap<Namespace, WatchedRoot>,
) -> Option<(&'a Namespace, &'a WatchedRoot)> {
    watched
        .iter()
        .find(|(_, root)| path.starts_with(&root.path))
}

/// Handle an error that arrived through `notify-debouncer-full`'s async
/// callback (mid-session watch losses, OS-level queue overflow, etc.).
/// Two responsibilities:
/// 1. Drop the affected namespace(s) from `watched` so the next periodic
///    `reconcile_from_model` re-adds the watch. Without this, an unmount
///    or directory deletion would silently stop monitoring until restart.
/// 2. Forward a `SubscriberErrorEvent` to the reporter so the UI can
///    react (currently it only toasts `inotify_limit`; the others are
///    logged to the console).
fn handle_async_error(
    watched: &Arc<Mutex<BTreeMap<Namespace, WatchedRoot>>>,
    reporter: &dyn StatusReporter,
    err: notify::Error,
) {
    warn!("fswatcher: async subscriber error: {err}");
    let affected: Vec<Namespace> = {
        let mut watched = watched.lock().unwrap();
        let to_drop: Vec<Namespace> = watched
            .iter()
            .filter(|(_, root)| err.paths.iter().any(|p| p.starts_with(&root.path)))
            .map(|(ns, _)| ns.clone())
            .collect();
        for ns in &to_drop {
            watched.remove(ns);
        }
        to_drop
    };
    let first_ns = affected.into_iter().next();
    let classified = classify(err, first_ns.as_ref());
    reporter.report_subscriber_error(SubscriberErrorEvent {
        kind: classified.kind_str().to_string(),
        message: classified.message(),
        namespace: classified.namespace().map(ToString::to_string),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::autopull::reporter::test_support::RecordingReporter;
    use notify::event::{AccessKind, DataChange, MetadataKind};
    use tempfile::TempDir;
    use tokio::time::Duration;

    fn test_reporter() -> Arc<dyn StatusReporter> {
        Arc::new(RecordingReporter::default())
    }

    /// A watched root with no `.quiltignore`, for the event-kind tests: they
    /// are about which *kinds* reach a namespace, so the ignore screen is not
    /// the variable under test.
    fn unignored(path: &str) -> WatchedRoot {
        WatchedRoot {
            path: PathBuf::from(path),
            quiltignore: None,
        }
    }

    fn debounced(kind: EventKind) -> notify_debouncer_full::DebouncedEvent {
        notify_debouncer_full::DebouncedEvent::new(
            notify::Event {
                kind,
                paths: vec![PathBuf::from("/pkg/file.txt")],
                attrs: notify::event::EventAttributes::default(),
            },
            std::time::Instant::now(),
        )
    }

    #[test]
    fn status_reads_and_all_metadata_variants_do_not_wake_the_watcher() {
        let namespace: Namespace = ("acme", "demo").into();
        let watched = BTreeMap::from([(namespace, unignored("/pkg"))]);
        let metadata_kinds = [
            MetadataKind::Any,
            MetadataKind::AccessTime,
            MetadataKind::WriteTime,
            MetadataKind::Permissions,
            MetadataKind::Ownership,
            MetadataKind::Extended,
            MetadataKind::Other,
        ];

        assert!(
            affected_namespaces(&[debounced(EventKind::Access(AccessKind::Read))], &watched)
                .is_empty()
        );
        for metadata in metadata_kinds {
            assert!(
                affected_namespaces(
                    &[debounced(EventKind::Modify(ModifyKind::Metadata(metadata)))],
                    &watched,
                )
                .is_empty(),
                "metadata event {metadata:?} must not trigger a status walk"
            );
        }
    }

    #[test]
    fn content_mutations_still_wake_the_watcher() {
        let namespace: Namespace = ("acme", "demo").into();
        let watched = BTreeMap::from([(namespace.clone(), unignored("/pkg"))]);

        assert_eq!(
            affected_namespaces(
                &[debounced(EventKind::Modify(ModifyKind::Data(
                    DataChange::Content,
                )))],
                &watched,
            ),
            BTreeSet::from([namespace])
        );
    }

    #[tokio::test]
    async fn add_and_fire_signal() -> Result<(), Box<dyn std::error::Error>> {
        let dir = TempDir::new()?;
        let (tx, mut rx) = mpsc::channel::<MappingSignal>(16);
        let mut sub = Subscription::new(Duration::from_millis(50), tx, &test_reporter())?;
        let ns: Namespace = ("acme", "demo").into();
        sub.add(ns.clone(), dir.path().to_path_buf())?;

        // Give notify a moment to attach the watch before writing.
        tokio::time::sleep(Duration::from_millis(100)).await;
        tokio::fs::write(dir.path().join("file.txt"), b"hello").await?;

        let signal = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await?
            .expect("subscription should emit a signal");
        assert_eq!(signal.namespace, ns);
        Ok(())
    }

    /// The regression test for the defect this screen exists to stop, stated as
    /// the loop rather than as an event taxonomy: the app **reads the tree it
    /// watches** — that is what a status walk is — so a screen that admits
    /// reads makes the walk re-trigger itself, and the app's own bookkeeping
    /// becomes its workload.
    ///
    /// Deliberately not written against `EventKind`: a unit assertion that
    /// `Access(_)` is filtered passes on a platform whose backend spells a read
    /// differently, which is exactly the case the assertion is meant to cover.
    /// This drives the real watcher instead and requires silence.
    #[tokio::test]
    async fn reading_the_watched_tree_does_not_signal() -> Result<(), Box<dyn std::error::Error>> {
        let dir = TempDir::new()?;
        tokio::fs::create_dir(dir.path().join("nested")).await?;
        for name in ["a.md", "nested/b.md"] {
            tokio::fs::write(dir.path().join(name), b"# seed\n").await?;
        }

        let (tx, mut rx) = mpsc::channel::<MappingSignal>(64);
        let mut sub = Subscription::new(Duration::from_millis(50), tx, &test_reporter())?;
        sub.add(("acme", "demo").into(), dir.path().to_path_buf())?;
        // Let the watch attach and the seeding events drain.
        tokio::time::sleep(Duration::from_millis(400)).await;
        while rx.try_recv().is_ok() {}

        // The only thing that happens from here is a read of the tree, shaped
        // like the walk: a listing of every directory, then every file's bytes.
        for _ in 0..3 {
            let mut queue = vec![dir.path().to_path_buf()];
            while let Some(current) = queue.pop() {
                let mut entries = tokio::fs::read_dir(&current).await?;
                while let Some(entry) = entries.next_entry().await? {
                    if entry.file_type().await?.is_dir() {
                        queue.push(entry.path());
                    } else {
                        let _ = tokio::fs::read(entry.path()).await?;
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }

        assert!(
            rx.try_recv().is_err(),
            "reading the watched tree must not signal — a read is not a change, \
             and a walk that re-arms its own trigger never lets the app go idle",
        );
        Ok(())
    }

    /// The `.quiltignore` half: an ignored path is outside the model in both
    /// directions, so it is neither reported nor a cause of reporting — while
    /// the ignore file itself always is, since editing it moves the changeset.
    #[tokio::test]
    async fn ignored_paths_do_not_signal_but_the_ignore_file_does()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = TempDir::new()?;
        tokio::fs::write(dir.path().join(".quiltignore"), b".rumdl_cache/\n").await?;
        tokio::fs::create_dir(dir.path().join(".rumdl_cache")).await?;

        let (tx, mut rx) = mpsc::channel::<MappingSignal>(64);
        let mut sub = Subscription::new(Duration::from_millis(50), tx, &test_reporter())?;
        let ns: Namespace = ("acme", "demo").into();
        sub.add(ns.clone(), dir.path().to_path_buf())?;
        tokio::time::sleep(Duration::from_millis(400)).await;
        while rx.try_recv().is_ok() {}

        // A write deep inside the ignored directory: the matcher has to reach
        // it through the *parent* pattern, with no walk context to lean on.
        tokio::fs::write(dir.path().join(".rumdl_cache/deep.json"), b"{}").await?;
        tokio::time::sleep(Duration::from_millis(400)).await;
        assert!(
            rx.try_recv().is_err(),
            "a write inside an ignored directory must not cost a status walk",
        );

        // The ignore file is the one path that can never be ignored.
        tokio::fs::write(dir.path().join(".quiltignore"), b".rumdl_cache/\nnotes/\n").await?;
        let signal = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await?
            .expect("editing .quiltignore changes the changeset, so it must signal");
        assert_eq!(signal.namespace, ns);
        Ok(())
    }

    #[tokio::test]
    async fn add_is_idempotent_same_root() -> Result<(), Box<dyn std::error::Error>> {
        let dir = TempDir::new()?;
        let (tx, _rx) = mpsc::channel::<MappingSignal>(16);
        let mut sub = Subscription::new(Duration::from_millis(50), tx, &test_reporter())?;
        let ns: Namespace = ("acme", "demo").into();
        sub.add(ns.clone(), dir.path().to_path_buf())?;
        sub.add(ns.clone(), dir.path().to_path_buf())?;
        assert_eq!(sub.watched_namespaces(), vec![ns]);
        Ok(())
    }

    #[tokio::test]
    async fn remove_drops_watch() -> Result<(), Box<dyn std::error::Error>> {
        let dir = TempDir::new()?;
        let (tx, _rx) = mpsc::channel::<MappingSignal>(16);
        let mut sub = Subscription::new(Duration::from_millis(50), tx, &test_reporter())?;
        let ns: Namespace = ("acme", "demo").into();
        sub.add(ns.clone(), dir.path().to_path_buf())?;
        sub.remove(&ns);
        assert!(sub.watched_namespaces().is_empty());
        Ok(())
    }
}
