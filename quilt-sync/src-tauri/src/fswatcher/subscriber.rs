use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::HashSet;
use std::fmt;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

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
    pub kind: SignalKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalKind {
    /// Something in the package changed.
    Changed,
    /// The OS lost events that may cover the package (a queue overflow, a
    /// rescan): walk it, but nothing is known to have changed.
    Rescan,
    /// The package's watch is gone; the next reconcile makes it again.
    Lost,
}

impl MappingSignal {
    pub fn changed(namespace: Namespace) -> Self {
        Self {
            namespace,
            kind: SignalKind::Changed,
        }
    }
}

/// A package to watch: its home, the user-wide ignore file, and what it has.
#[derive(Clone)]
pub struct WatchedPackage {
    pub home: PathBuf,
    pub user_rules: PathBuf,
    /// The package's files and every directory holding one, relative to `home`.
    tracked: HashSet<PathBuf>,
}

impl WatchedPackage {
    /// `files` are the package's logical keys.
    pub fn new(
        home: PathBuf,
        user_rules: PathBuf,
        files: impl IntoIterator<Item = PathBuf>,
    ) -> Self {
        let mut tracked = HashSet::new();
        for file in files {
            for dir in file.ancestors().skip(1) {
                if dir.as_os_str().is_empty() || !tracked.insert(dir.to_path_buf()) {
                    break;
                }
            }
            tracked.insert(file);
        }
        WatchedPackage {
            home,
            user_rules,
            tracked,
        }
    }
}

/// A watched package: where it lives, its ignore rules, and what it has.
///
/// The rules are cached rather than loaded per event. `add` refreshes them and
/// the tracked set, and so does an event on either rules file.
struct WatchedRoot {
    path: PathBuf,
    user_rules: PathBuf,
    rules: Option<quilt::quiltignore::Rules>,
    tracked: HashSet<PathBuf>,
}

impl WatchedRoot {
    fn new(path: PathBuf, package: WatchedPackage) -> Self {
        let rules = load_rules(&path, &package.user_rules);
        WatchedRoot {
            path,
            user_rules: package.user_rules,
            rules,
            tracked: package.tracked,
        }
    }

    fn reload_rules(&mut self) {
        self.rules = load_rules(&self.path, &self.user_rules);
    }

    /// Whether the ignore rules hide `path`, as the status walk would.
    ///
    /// Never the ignore file itself: editing it moves the changeset, so it is
    /// always a real change.
    fn excludes(&self, path: &Path) -> bool {
        let Some(rules) = self.rules.as_ref() else {
            return false;
        };
        // The matcher panics on a path outside its root, so the relative key
        // has to come from a checked strip, not from the prefix test that
        // resolved the namespace.
        let Ok(relative) = path.strip_prefix(&self.path) else {
            return false;
        };
        if relative == Path::new(QUILTIGNORE_FILE) {
            return false;
        }
        // A tracked directory is one holding a tracked file.
        let tracked = self.tracked.contains(relative);
        // The matcher tests every parent as a directory, so `cache/` matches
        // `cache/x.tmp` without a stat. Only the path itself needs one, and a
        // removed path reads as a file.
        if path.is_dir() {
            rules.dir_ignored_by(relative, tracked).is_some()
        } else {
            rules.file_ignored_by(relative, tracked).is_some()
        }
    }
}

/// A failure to read the rules leaves them absent rather than failing the
/// watch: not screening is the safe direction. It costs walks, where wrongly
/// screening would drop real changes.
fn load_rules(package_home: &Path, user_rules: &Path) -> Option<quilt::quiltignore::Rules> {
    quilt::quiltignore::Rules::load(package_home, user_rules)
        .inspect_err(|err| {
            warn!(
                "fswatcher: ignore rules for {} unreadable, not screening ignored paths: {err}",
                package_home.display(),
            );
        })
        .ok()
}

/// The user-wide ignore file, which lives outside every package home and so
/// gets its own watch on its directory.
#[derive(Clone, PartialEq)]
struct UserRules {
    /// The file, canonical like the package roots, for matching events.
    file: PathBuf,
    /// The directory this subscription watches for it. `None` when a package
    /// watch already covers that directory recursively.
    dir: Option<PathBuf>,
}

/// Owns the OS-level subscription and the namespace ↔ root mapping. The
/// debouncer lives behind a `Mutex` because `notify-debouncer-full` runs its
/// own thread that calls back into the event handler; we need shared access
/// from both the `watch`/`unwatch` Tauri thread and the event handler thread.
pub struct Subscription {
    debouncer: Debouncer<RecommendedWatcher, RecommendedCache>,
    watched: Arc<Mutex<BTreeMap<Namespace, WatchedRoot>>>,
    user_rules: Arc<Mutex<Option<UserRules>>>,
}

impl Subscription {
    /// Build a fresh subscription with no watched roots yet.
    ///
    /// `signal_tx` is the channel the reactor reads from. It is unbounded so
    /// that no signal is dropped: there is at most one per package per
    /// debounce flush, and the reactor handles each without a walk.
    pub fn new(
        debounce: Duration,
        signal_tx: mpsc::UnboundedSender<MappingSignal>,
        reporter: &Arc<dyn StatusReporter>,
    ) -> Result<Self, SubscriberError> {
        let watched: Arc<Mutex<BTreeMap<Namespace, WatchedRoot>>> =
            Arc::new(Mutex::new(BTreeMap::new()));
        let watched_for_cb = Arc::clone(&watched);
        let user_rules: Arc<Mutex<Option<UserRules>>> = Arc::new(Mutex::new(None));
        let user_rules_for_cb = Arc::clone(&user_rules);
        let reporter_for_cb = Arc::clone(reporter);
        let debouncer = new_debouncer(
            debounce,
            None,
            move |result: notify_debouncer_full::DebounceEventResult| {
                let signals = match result {
                    Ok(events) => {
                        let user_file = user_rules_for_cb
                            .lock()
                            .unwrap()
                            .as_ref()
                            .map(|rules| rules.file.clone());
                        let mut watched = watched_for_cb.lock().unwrap();
                        reload_edited_rules(&events, &mut watched, user_file.as_deref());
                        let changed = affected_namespaces(&events, &watched, user_file.as_deref())
                            .into_iter()
                            .map(MappingSignal::changed);
                        let rescan =
                            rescanned_namespaces(&events, &watched)
                                .into_iter()
                                .map(|namespace| MappingSignal {
                                    namespace,
                                    kind: SignalKind::Rescan,
                                });
                        let lost = drop_removed_roots(&events, &mut watched).into_iter().map(
                            |namespace| MappingSignal {
                                namespace,
                                kind: SignalKind::Lost,
                            },
                        );
                        changed.chain(rescan).chain(lost).collect::<Vec<_>>()
                    }
                    Err(errors) => errors
                        .into_iter()
                        .flat_map(|err| {
                            handle_async_error(
                                &watched_for_cb,
                                &user_rules_for_cb,
                                reporter_for_cb.as_ref(),
                                err,
                            )
                        })
                        .collect(),
                };
                for signal in signals {
                    // Fails only once the reactor is gone.
                    if signal_tx.send(signal).is_err() {
                        debug!("fswatcher: reactor stopped, dropping signals");
                        return;
                    }
                }
            },
        )
        .map_err(|err| classify(err, None))?;
        Ok(Self {
            debouncer,
            watched,
            user_rules,
        })
    }

    /// Idempotent: a duplicate `add` for the same namespace + root is a no-op.
    /// A different root for the same namespace tears the old watch down first.
    pub fn add(
        &mut self,
        namespace: Namespace,
        package: WatchedPackage,
    ) -> Result<(), SubscriberError> {
        let package_home = package.home.clone();
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
        let root = WatchedRoot::new(package_home.clone(), package);
        let existing_path = {
            let watched = self.watched.lock().unwrap();
            watched.get(&namespace).map(|w| w.path.clone())
        };
        if let Some(existing) = existing_path.as_ref() {
            if existing == &package_home {
                // The OS watch is already right, so this stays a no-op for
                // `notify`, but the rules and the tracked set are replaced.
                if let Some(entry) = self.watched.lock().unwrap().get_mut(&namespace) {
                    *entry = root;
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
                self.watched.lock().unwrap().insert(namespace, root);
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

    /// Watch the user-wide ignore file, or stop watching it with `None`.
    ///
    /// The file may not exist yet, so its directory is watched instead. A
    /// failure is only logged: package watches do not depend on it, and the
    /// next reconcile retries.
    fn set_user_rules(&mut self, file: Option<PathBuf>) {
        let wanted = file.and_then(|file| {
            let dir = file.parent()?.to_path_buf();
            let name = file.file_name()?.to_owned();
            // Same canonical form as the package roots, so events match.
            let dir = dunce::canonicalize(&dir).unwrap_or(dir);
            let file = dir.join(name);
            // A second watch on a path inside a recursive one would share its
            // OS watch on some backends, and unwatching it would break the
            // package watch.
            let covered = self
                .watched
                .lock()
                .unwrap()
                .values()
                .any(|root| dir.starts_with(&root.path));
            Some(UserRules {
                file,
                dir: (!covered).then_some(dir),
            })
        });
        let current = self.user_rules.lock().unwrap().clone();
        if current == wanted {
            return;
        }
        let current_dir = current.and_then(|rules| rules.dir);
        let wanted_dir = wanted.as_ref().and_then(|rules| rules.dir.clone());
        let mut dir_ok = true;
        if current_dir != wanted_dir {
            // A package watch that now covers the directory may have taken
            // over the OS watch on it, so that one is left in place.
            let covered = wanted.as_ref().is_some_and(|rules| rules.dir.is_none());
            if let Some(dir) = current_dir.as_ref()
                && !covered
            {
                let _ = self.debouncer.unwatch(dir);
            }
            if let Some(dir) = wanted_dir.as_ref()
                && let Err(err) = self.debouncer.watch(dir, RecursiveMode::NonRecursive)
            {
                warn!(
                    "fswatcher: cannot watch {} for user-wide ignore rules: {err}",
                    dir.display(),
                );
                dir_ok = false;
            }
        }
        *self.user_rules.lock().unwrap() = if dir_ok { wanted } else { None };
    }

    /// Incrementally reconcile the watched set with `desired`. Drops
    /// namespaces no longer present; adds (or updates the root of)
    /// namespaces that are. `Subscription::add` is already idempotent
    /// for an unchanged (namespace, path) pair, so a no-op reconcile is
    /// cheap.
    ///
    /// `user_rules` is the user-wide ignore file; see `set_user_rules`.
    pub fn reconcile(
        &mut self,
        desired: Vec<(Namespace, WatchedPackage)>,
        user_rules: Option<PathBuf>,
    ) -> Result<(), SubscriberError> {
        let desired_map: BTreeMap<Namespace, WatchedPackage> = desired.into_iter().collect();
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
        for (namespace, package) in desired_map {
            let ns_display = namespace.to_string();
            if let Err(err) = self.add(namespace, package) {
                warn!("fswatcher: reconcile add({ns_display}) failed: {err}");
                if first_err.is_none() {
                    first_err = Some(err);
                }
            }
        }
        // After the packages, so it knows which directories they cover.
        self.set_user_rules(user_rules);
        match first_err {
            Some(err) => Err(err),
            None => Ok(()),
        }
    }

    /// Whether the user-wide ignore file is watched, so an edit of it reaches
    /// every package.
    pub fn user_rules_watched(&self) -> bool {
        self.user_rules.lock().unwrap().is_some()
    }

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
/// A path the ignore rules hide is dropped too. The walk skips it as well, so
/// such an event could only ever buy a recompute that finds nothing, and an
/// ignored path is outside the model in both directions: neither reported, nor
/// a cause of reporting.
///
/// A change to the user-wide ignore file (`user_rules`) affects every watched
/// package. Its siblings are not package files, so they resolve to nothing.
fn affected_namespaces(
    events: &[notify_debouncer_full::DebouncedEvent],
    watched: &BTreeMap<Namespace, WatchedRoot>,
    user_rules: Option<&Path>,
) -> BTreeSet<Namespace> {
    let mut touched = BTreeSet::new();
    for event in events {
        if is_non_content_event(event.kind) || event.need_rescan() {
            continue;
        }
        for path in &event.paths {
            if user_rules == Some(path.as_path()) {
                touched.extend(watched.keys().cloned());
                continue;
            }
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

/// Reload the rules of every package whose rules file an event touched, so
/// that the rest of the batch and later events are screened by the new rules.
fn reload_edited_rules(
    events: &[notify_debouncer_full::DebouncedEvent],
    watched: &mut BTreeMap<Namespace, WatchedRoot>,
    user_rules: Option<&Path>,
) {
    let mut edited = BTreeSet::new();
    for path in events
        .iter()
        .filter(|event| !is_non_content_event(event.kind))
        .flat_map(|event| &event.paths)
    {
        if user_rules == Some(path.as_path()) {
            edited.extend(watched.keys().cloned());
        } else if let Some((namespace, root)) = namespace_for(path, watched)
            && path.strip_prefix(&root.path).ok() == Some(Path::new(QUILTIGNORE_FILE))
        {
            edited.insert(namespace.clone());
        }
    }
    for namespace in edited {
        if let Some(root) = watched.get_mut(&namespace) {
            root.reload_rules();
        }
    }
}

/// The namespaces whose events the OS says it lost: those under a rescan
/// event's paths, or every one for a rescan with no path.
fn rescanned_namespaces(
    events: &[notify_debouncer_full::DebouncedEvent],
    watched: &BTreeMap<Namespace, WatchedRoot>,
) -> BTreeSet<Namespace> {
    let mut rescan = BTreeSet::new();
    for event in events.iter().filter(|event| event.need_rescan()) {
        if event.paths.is_empty() {
            rescan.extend(watched.keys().cloned());
        }
        for path in &event.paths {
            rescan.extend(
                watched
                    .iter()
                    .filter(|(_, root)| path.starts_with(&root.path) || root.path.starts_with(path))
                    .map(|(ns, _)| ns.clone()),
            );
        }
    }
    rescan
}

/// Drop the namespaces whose root was removed or moved, even if it is back:
/// the OS watch went with the old folder, and the next reconcile makes a new
/// one.
fn drop_removed_roots(
    events: &[notify_debouncer_full::DebouncedEvent],
    watched: &mut BTreeMap<Namespace, WatchedRoot>,
) -> Vec<Namespace> {
    let removed: Vec<Namespace> = watched
        .iter()
        .filter(|(_, root)| {
            events.iter().any(|event| {
                matches!(
                    event.kind,
                    EventKind::Remove(_) | EventKind::Modify(ModifyKind::Name(_))
                ) && event.paths.iter().any(|p| root.path.starts_with(p))
            })
        })
        .map(|(ns, _)| ns.clone())
        .collect();
    for ns in &removed {
        watched.remove(ns);
    }
    removed
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
///
/// Returns the signals for the reactor: `Lost` for each dropped namespace, or,
/// for an error with no path, `Rescan` for every watched one.
fn handle_async_error(
    watched: &Arc<Mutex<BTreeMap<Namespace, WatchedRoot>>>,
    user_rules: &Arc<Mutex<Option<UserRules>>>,
    reporter: &dyn StatusReporter,
    err: notify::Error,
) -> Vec<MappingSignal> {
    warn!("fswatcher: async subscriber error: {err}");
    {
        let mut user_rules = user_rules.lock().unwrap();
        if user_rules
            .as_ref()
            .and_then(|rules| rules.dir.as_ref())
            .is_some_and(|dir| err.paths.contains(dir))
        {
            // Forget the lost watch so the next reconcile sets it up again.
            *user_rules = None;
        }
    }
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
    let signals = if err.paths.is_empty() {
        watched
            .lock()
            .unwrap()
            .keys()
            .map(|namespace| MappingSignal {
                namespace: namespace.clone(),
                kind: SignalKind::Rescan,
            })
            .collect()
    } else {
        affected
            .iter()
            .map(|namespace| MappingSignal {
                namespace: namespace.clone(),
                kind: SignalKind::Lost,
            })
            .collect()
    };
    let first_ns = affected.into_iter().next();
    let classified = classify(err, first_ns.as_ref());
    reporter.report_subscriber_error(SubscriberErrorEvent {
        kind: classified.kind_str().to_string(),
        message: classified.message(),
        namespace: classified.namespace().map(ToString::to_string),
    });
    signals
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
            user_rules: PathBuf::new(),
            rules: None,
            tracked: HashSet::new(),
        }
    }

    /// A package with no files yet and no user-wide rules file.
    fn untracked(home: &Path) -> WatchedPackage {
        WatchedPackage::new(home.to_path_buf(), home.with_extension("user-rules"), [])
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
            affected_namespaces(
                &[debounced(EventKind::Access(AccessKind::Read))],
                &watched,
                None
            )
            .is_empty()
        );
        for metadata in metadata_kinds {
            assert!(
                affected_namespaces(
                    &[debounced(EventKind::Modify(ModifyKind::Metadata(metadata)))],
                    &watched,
                    None,
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
                None,
            ),
            BTreeSet::from([namespace])
        );
    }

    fn modified(path: &str) -> notify_debouncer_full::DebouncedEvent {
        notify_debouncer_full::DebouncedEvent::new(
            notify::Event {
                kind: EventKind::Modify(ModifyKind::Data(DataChange::Content)),
                paths: vec![PathBuf::from(path)],
                attrs: notify::event::EventAttributes::default(),
            },
            std::time::Instant::now(),
        )
    }

    #[test]
    fn the_user_rules_file_wakes_every_package_and_its_siblings_wake_none() {
        let a: Namespace = ("acme", "a").into();
        let b: Namespace = ("acme", "b").into();
        let watched = BTreeMap::from([
            (a.clone(), unignored("/domain/a")),
            (b.clone(), unignored("/elsewhere/b")),
        ]);
        let user_rules = Some(Path::new("/domain/.quiltignore"));

        assert_eq!(
            affected_namespaces(&[modified("/domain/.quiltignore")], &watched, user_rules),
            BTreeSet::from([a.clone(), b]),
        );
        assert!(
            affected_namespaces(&[modified("/domain/notes.txt")], &watched, user_rules).is_empty(),
        );
        assert_eq!(
            affected_namespaces(&[modified("/domain/a/file.txt")], &watched, user_rules),
            BTreeSet::from([a]),
            "package events are unchanged",
        );
    }

    #[tokio::test]
    async fn editing_the_user_rules_file_signals_every_package()
    -> Result<(), Box<dyn std::error::Error>> {
        let domain = TempDir::new()?;
        let elsewhere = TempDir::new()?;
        tokio::fs::create_dir(domain.path().join("a")).await?;
        let a: Namespace = ("acme", "a").into();
        let b: Namespace = ("acme", "b").into();

        let (tx, mut rx) = mpsc::unbounded_channel::<MappingSignal>();
        let mut sub = Subscription::new(Duration::from_millis(50), tx, &test_reporter())?;
        let user_file = domain.path().join(".quiltignore");
        let desired = vec![
            (
                a.clone(),
                WatchedPackage::new(domain.path().join("a"), user_file.clone(), []),
            ),
            (
                b.clone(),
                WatchedPackage::new(elsewhere.path().to_path_buf(), user_file.clone(), []),
            ),
        ];
        let user_rules = Some(user_file);
        sub.reconcile(desired.clone(), user_rules.clone())?;
        // A repeat reconcile keeps the same watch.
        sub.reconcile(desired, user_rules)?;
        tokio::time::sleep(Duration::from_millis(400)).await;
        while rx.try_recv().is_ok() {}

        tokio::fs::write(domain.path().join("notes.txt"), b"x").await?;
        tokio::time::sleep(Duration::from_millis(400)).await;
        assert!(
            rx.try_recv().is_err(),
            "another file next to the user-wide rules is not a package change",
        );

        tokio::fs::write(domain.path().join(".quiltignore"), b"*.log\n").await?;
        let mut signalled = BTreeSet::new();
        while signalled.len() < 2 {
            let signal = tokio::time::timeout(Duration::from_secs(2), rx.recv())
                .await?
                .expect("editing the user-wide rules must signal");
            signalled.insert(signal.namespace);
        }
        assert_eq!(signalled, BTreeSet::from([a, b]));
        Ok(())
    }

    /// A package home above the domain root already watches the directory
    /// recursively, so the subscription must not add (and later drop) its own.
    #[tokio::test]
    async fn the_user_rules_directory_is_not_watched_twice()
    -> Result<(), Box<dyn std::error::Error>> {
        let home = TempDir::new()?;
        tokio::fs::create_dir(home.path().join("domain")).await?;
        let (tx, _rx) = mpsc::unbounded_channel::<MappingSignal>();
        let mut sub = Subscription::new(Duration::from_millis(50), tx, &test_reporter())?;
        sub.reconcile(
            vec![(("acme", "demo").into(), untracked(home.path()))],
            Some(home.path().join("domain/.quiltignore")),
        )?;
        let rules = sub
            .user_rules
            .lock()
            .unwrap()
            .clone()
            .expect("rules are known");
        assert!(rules.file.ends_with("domain/.quiltignore"));
        assert_eq!(rules.dir, None);
        Ok(())
    }

    /// A rescan is not a change: it walks the packages it may cover and moves
    /// no last-change time. One with no path covers every package.
    #[test]
    fn a_rescan_is_reported_apart_from_changes() {
        let demo: Namespace = ("acme", "demo").into();
        let other: Namespace = ("acme", "other").into();
        let watched = BTreeMap::from([
            (demo.clone(), unignored("/pkg")),
            (other.clone(), unignored("/other")),
        ]);
        let rescan = |paths: Vec<PathBuf>| {
            notify_debouncer_full::DebouncedEvent::new(
                notify::Event {
                    kind: EventKind::Other,
                    paths,
                    attrs: notify::event::EventAttributes::default(),
                }
                .set_flag(notify::event::Flag::Rescan),
                std::time::Instant::now(),
            )
        };

        let under_one = [rescan(vec![PathBuf::from("/pkg/sub")])];
        assert!(affected_namespaces(&under_one, &watched, None).is_empty());
        assert_eq!(
            rescanned_namespaces(&under_one, &watched),
            BTreeSet::from([demo.clone()])
        );
        assert_eq!(
            rescanned_namespaces(&[rescan(Vec::new())], &watched),
            BTreeSet::from([demo, other])
        );
    }

    #[test]
    fn a_removed_root_is_dropped_from_the_watch() {
        let namespace: Namespace = ("acme", "demo").into();
        let mut watched = BTreeMap::from([(namespace.clone(), unignored("/pkg"))]);
        let removed = notify_debouncer_full::DebouncedEvent::new(
            notify::Event {
                kind: EventKind::Remove(notify::event::RemoveKind::Folder),
                paths: vec![PathBuf::from("/pkg")],
                attrs: notify::event::EventAttributes::default(),
            },
            std::time::Instant::now(),
        );

        assert_eq!(
            drop_removed_roots(&[removed], &mut watched),
            vec![namespace]
        );
        assert!(watched.is_empty());
    }

    #[tokio::test]
    async fn add_and_fire_signal() -> Result<(), Box<dyn std::error::Error>> {
        let dir = TempDir::new()?;
        let (tx, mut rx) = mpsc::unbounded_channel::<MappingSignal>();
        let mut sub = Subscription::new(Duration::from_millis(50), tx, &test_reporter())?;
        let ns: Namespace = ("acme", "demo").into();
        sub.add(ns.clone(), untracked(dir.path()))?;

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

        let (tx, mut rx) = mpsc::unbounded_channel::<MappingSignal>();
        let mut sub = Subscription::new(Duration::from_millis(50), tx, &test_reporter())?;
        sub.add(("acme", "demo").into(), untracked(dir.path()))?;
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

        let (tx, mut rx) = mpsc::unbounded_channel::<MappingSignal>();
        let mut sub = Subscription::new(Duration::from_millis(50), tx, &test_reporter())?;
        let ns: Namespace = ("acme", "demo").into();
        sub.add(ns.clone(), untracked(dir.path()))?;
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

    /// The built-in rules hide only paths the package does not have, as in the
    /// status walk; the package's own rules hide any path.
    #[test]
    fn the_ignore_rules_screen_events_as_the_walk_does() -> Result<(), Box<dyn std::error::Error>> {
        let dir = TempDir::new()?;
        std::fs::write(dir.path().join(".quiltignore"), "cache/\n")?;
        std::fs::create_dir(dir.path().join(".git"))?;
        let ns: Namespace = ("acme", "demo").into();
        let package = WatchedPackage::new(
            dir.path().to_path_buf(),
            dir.path().with_extension("user-rules"),
            [".DS_Store", ".github/ci.yml", "cache/kept.csv"].map(PathBuf::from),
        );
        let watched = BTreeMap::from([(
            ns.clone(),
            WatchedRoot::new(dir.path().to_path_buf(), package),
        )]);
        let signals = |rel: &str| {
            let path = dir.path().join(rel);
            !affected_namespaces(&[modified(path.to_str().unwrap())], &watched, None).is_empty()
        };

        for rel in [".git", ".git/index", "node_modules/x.js", "sub/.DS_Store"] {
            assert!(
                !signals(rel),
                "{rel} is untracked and hidden by the defaults"
            );
        }
        for rel in [
            ".DS_Store",
            ".github",
            ".github/ci.yml",
            "notes.txt",
            ".quiltignore",
        ] {
            assert!(signals(rel), "{rel} must signal");
        }
        assert!(
            !signals(".github/other.yml"),
            "an untracked sibling stays hidden"
        );
        for rel in ["cache/x.tmp", "cache/kept.csv"] {
            assert!(!signals(rel), "the package's rules hide {rel}");
        }
        Ok(())
    }

    #[test]
    fn an_edited_user_rules_file_is_picked_up_at_once() -> Result<(), Box<dyn std::error::Error>> {
        let dir = TempDir::new()?;
        let user_file = dir.path().with_extension("user-rules");
        let ns: Namespace = ("acme", "demo").into();
        let package = WatchedPackage::new(dir.path().to_path_buf(), user_file.clone(), []);
        let mut watched = BTreeMap::from([(
            ns.clone(),
            WatchedRoot::new(dir.path().to_path_buf(), package),
        )]);
        let log = dir.path().join("run.log");
        let log = log.to_str().unwrap();
        assert_eq!(
            affected_namespaces(&[modified(log)], &watched, Some(&user_file)),
            BTreeSet::from([ns]),
        );

        std::fs::write(&user_file, "*.log\n")?;
        reload_edited_rules(
            &[modified(user_file.to_str().unwrap())],
            &mut watched,
            Some(&user_file),
        );
        assert!(affected_namespaces(&[modified(log)], &watched, Some(&user_file)).is_empty());
        Ok(())
    }

    /// A busy git folder must not signal, or each write would push back the
    /// next autopublish; a file the package has signals whatever its name.
    #[tokio::test]
    async fn an_untracked_git_folder_does_not_signal_but_a_tracked_dotfile_does()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = TempDir::new()?;
        tokio::fs::create_dir(dir.path().join(".git")).await?;
        tokio::fs::write(dir.path().join(".DS_Store"), b"seed").await?;

        let (tx, mut rx) = mpsc::unbounded_channel::<MappingSignal>();
        let mut sub = Subscription::new(Duration::from_millis(50), tx, &test_reporter())?;
        let ns: Namespace = ("acme", "demo").into();
        let package = WatchedPackage::new(
            dir.path().to_path_buf(),
            dir.path().with_extension("user-rules"),
            [PathBuf::from(".DS_Store")],
        );
        sub.add(ns.clone(), package)?;
        tokio::time::sleep(Duration::from_millis(400)).await;
        while rx.try_recv().is_ok() {}

        tokio::fs::write(dir.path().join(".git/index"), b"x").await?;
        tokio::time::sleep(Duration::from_millis(400)).await;
        assert!(
            rx.try_recv().is_err(),
            "an untracked .git/ write must not signal"
        );

        tokio::fs::write(dir.path().join(".DS_Store"), b"edited").await?;
        let signal = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await?
            .expect("a tracked file must signal");
        assert_eq!(signal.namespace, ns);
        Ok(())
    }

    #[tokio::test]
    async fn add_is_idempotent_same_root() -> Result<(), Box<dyn std::error::Error>> {
        let dir = TempDir::new()?;
        let (tx, _rx) = mpsc::unbounded_channel::<MappingSignal>();
        let mut sub = Subscription::new(Duration::from_millis(50), tx, &test_reporter())?;
        let ns: Namespace = ("acme", "demo").into();
        sub.add(ns.clone(), untracked(dir.path()))?;
        sub.add(ns.clone(), untracked(dir.path()))?;
        assert_eq!(sub.watched_namespaces(), vec![ns]);
        Ok(())
    }

    #[tokio::test]
    async fn remove_drops_watch() -> Result<(), Box<dyn std::error::Error>> {
        let dir = TempDir::new()?;
        let (tx, _rx) = mpsc::unbounded_channel::<MappingSignal>();
        let mut sub = Subscription::new(Duration::from_millis(50), tx, &test_reporter())?;
        let ns: Namespace = ("acme", "demo").into();
        sub.add(ns.clone(), untracked(dir.path()))?;
        sub.remove(&ns);
        assert_eq!(sub.watched_namespaces(), [] as [Namespace; 0]);
        Ok(())
    }
}
