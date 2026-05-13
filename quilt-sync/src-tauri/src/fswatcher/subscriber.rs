use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fmt;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use notify::ErrorKind;
use notify::RecommendedWatcher;
use notify::RecursiveMode;
use notify_debouncer_full::Debouncer;
use notify_debouncer_full::RecommendedCache;
use notify_debouncer_full::new_debouncer;
use quilt_uri::Namespace;
use tokio::sync::mpsc;

use crate::autopull::StatusReporter;
use crate::autopull::reporter::SubscriberErrorEvent;
use crate::fswatcher::filter;
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

/// One signal per debounce-flush per affected namespace. The reactor consumes
/// these from an mpsc channel.
#[derive(Debug, Clone)]
pub struct MappingSignal {
    pub namespace: Namespace,
}

/// Owns the OS-level subscription and the namespace ↔ root mapping. The
/// debouncer lives behind a `Mutex` because `notify-debouncer-full` runs its
/// own thread that calls back into the event handler; we need shared access
/// from both the `watch`/`unwatch` Tauri thread and the event handler thread.
pub struct Subscription {
    debouncer: Debouncer<RecommendedWatcher, RecommendedCache>,
    watched: Arc<Mutex<BTreeMap<Namespace, PathBuf>>>,
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
        let watched: Arc<Mutex<BTreeMap<Namespace, PathBuf>>> =
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
        let existing_path = {
            let watched = self.watched.lock().unwrap();
            watched.get(&namespace).cloned()
        };
        if let Some(existing) = existing_path.as_ref() {
            if existing == &package_home {
                return Ok(());
            }
            let _ = self.debouncer.unwatch(existing);
        }
        match self
            .debouncer
            .watch(&package_home, RecursiveMode::Recursive)
        {
            Ok(()) => {
                self.watched.lock().unwrap().insert(namespace, package_home);
                Ok(())
            }
            Err(err) => Err(classify(err, Some(&namespace))),
        }
    }

    pub fn remove(&mut self, namespace: &Namespace) {
        let path = self.watched.lock().unwrap().remove(namespace);
        if let Some(path) = path {
            let _ = self.debouncer.unwatch(&path);
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
/// All event kinds — including `Access(_)` and `Modify(Metadata)` that
/// Linux inotify emits on plain reads — are forwarded. The reactor
/// dedupes spurious wakes by fingerprinting the recomputed status; a wake
/// here only costs one extra recompute, never a UI repaint.
fn affected_namespaces(
    events: &[notify_debouncer_full::DebouncedEvent],
    watched: &BTreeMap<Namespace, PathBuf>,
) -> BTreeSet<Namespace> {
    let mut touched = BTreeSet::new();
    for event in events {
        for path in &event.paths {
            if filter::is_ignored(path) {
                continue;
            }
            if let Some(ns) = namespace_for(path, watched) {
                touched.insert(ns.clone());
            }
        }
    }
    touched
}

fn namespace_for<'a>(
    path: &Path,
    watched: &'a BTreeMap<Namespace, PathBuf>,
) -> Option<&'a Namespace> {
    watched
        .iter()
        .find(|(_, root)| path.starts_with(root))
        .map(|(ns, _)| ns)
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
    watched: &Arc<Mutex<BTreeMap<Namespace, PathBuf>>>,
    reporter: &dyn StatusReporter,
    err: notify::Error,
) {
    warn!("fswatcher: async subscriber error: {err}");
    let affected: Vec<Namespace> = {
        let mut watched = watched.lock().unwrap();
        let to_drop: Vec<Namespace> = watched
            .iter()
            .filter(|(_, root)| err.paths.iter().any(|p| p.starts_with(root)))
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
    use tempfile::TempDir;
    use tokio::time::Duration;

    fn test_reporter() -> Arc<dyn StatusReporter> {
        Arc::new(RecordingReporter::default())
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
