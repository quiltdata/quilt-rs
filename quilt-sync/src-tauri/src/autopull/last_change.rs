//! Each package's last-change time: when the app last saw it change.
//!
//! Publish time is this plus the current wait. It is the app's own time rather
//! than the files' edit times, which are wrong for a deletion, for a copy that
//! keeps old times, for "publishing was just turned on" and for "the app was
//! just opened". Every gap in what the app knows counts as "first seen", which
//! waits a full wait: later, never earlier.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::SystemTime;

use quilt_uri::Namespace;

use crate::quilt::lineage::InstalledPackageStatus;
use crate::quilt::state_store::ChangeSource;
use crate::quilt::state_store::PackageStateStore;

/// Now, to the millisecond the store keeps, so a time handed out equals the
/// one read back.
pub(crate) fn now() -> SystemTime {
    let since = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    SystemTime::UNIX_EPOCH
        + std::time::Duration::from_millis(u64::try_from(since.as_millis()).unwrap_or(u64::MAX))
}

pub(crate) struct LastChange {
    store: PackageStateStore,
    /// Packages walked since launch. The first walk of each one asks whether it
    /// changed while the app was closed.
    walked: Mutex<BTreeSet<Namespace>>,
    /// What each package's last walk found changed, so a walk that finds
    /// something new restarts the wait even when no watcher signal came.
    last_seen: Mutex<BTreeMap<Namespace, String>>,
    /// Packages with a watcher signal since their last walk: that signal
    /// already set the time for what the walk finds.
    signalled: Mutex<BTreeSet<Namespace>>,
}

impl LastChange {
    pub(crate) fn new(store: PackageStateStore) -> Self {
        Self {
            store,
            walked: Mutex::new(BTreeSet::new()),
            last_seen: Mutex::new(BTreeMap::new()),
            signalled: Mutex::new(BTreeSet::new()),
        }
    }

    /// Record that the app is running now. Called by each check.
    pub(crate) async fn note_alive(&self) {
        self.store.set_last_alive_at(now()).await;
    }

    /// The folder watcher heard a change in `namespace`.
    pub(crate) async fn on_signal(&self, namespace: &Namespace) {
        self.signalled
            .lock()
            .expect("signalled lock")
            .insert(namespace.clone());
        self.store
            .record_change(namespace, now(), ChangeSource::Edit)
            .await;
    }

    /// Take a walk's result for `namespace` and return its last-change time,
    /// `None` when it has no changes.
    ///
    /// `home` is the package's folder, for the restart check; `None` when it
    /// could not be resolved, which counts as changed.
    pub(crate) async fn observe(
        &self,
        namespace: &Namespace,
        home: Option<&Path>,
        status: &InstalledPackageStatus,
    ) -> Option<SystemTime> {
        let signalled = self
            .signalled
            .lock()
            .expect("signalled lock")
            .remove(namespace);
        let seen = crate::autopull::reporter::changes_fingerprint(&status.changes);
        let before = self
            .last_seen
            .lock()
            .expect("last_seen lock")
            .insert(namespace.clone(), seen.clone());
        if status.changes.is_empty() {
            self.store.forget(namespace).await;
            return None;
        }
        let now = now();
        // Changes no signal announced: the watcher is off, or can't see this
        // folder.
        if !signalled && before.is_some_and(|before| before != seen) {
            self.store
                .record_change(namespace, now, ChangeSource::Edit)
                .await;
            return Some(now);
        }
        let first_walk = self
            .walked
            .lock()
            .expect("walked lock")
            .insert(namespace.clone());
        if self.store.record_first_seen(namespace, now).await {
            return Some(now);
        }
        // A store that can't answer counts as just seen.
        let Some(saved) = self.store.last_change(namespace).await else {
            return Some(now);
        };
        // The saved time is when the app last saw this package change, so a
        // changed file with a later ctime changed while the app was not
        // looking: closed, or before a session that never walked it.
        if first_walk
            && home.is_none_or(|home| {
                changed_while_closed(home, status.changes.keys(), saved.last_change_at)
            })
        {
            self.store
                .record_change(namespace, now, ChangeSource::FirstSeen)
                .await;
            return Some(now);
        }
        Some(saved.last_change_at)
    }

    /// Whether the next walk of `namespace` is its first since launch, the one
    /// that needs the package's folder.
    pub(crate) fn first_walk_pending(&self, namespace: &Namespace) -> bool {
        !self.walked.lock().expect("walked lock").contains(namespace)
    }

    /// The stored last-change time, without a walk.
    pub(crate) async fn get(&self, namespace: &Namespace) -> Option<SystemTime> {
        self.store
            .last_change(namespace)
            .await
            .map(|a| a.last_change_at)
    }

    /// The package published.
    pub(crate) async fn on_published(&self, namespace: &Namespace) {
        self.store.forget(namespace).await;
    }

    /// Publishing was turned on. `with_changes` is what the last walks found;
    /// every package with a stored time is added to it. Returns the packages
    /// whose time moved.
    pub(crate) async fn on_publishing_on(
        &self,
        with_changes: BTreeSet<Namespace>,
    ) -> BTreeSet<Namespace> {
        let mut moved = with_changes;
        moved.extend(self.store.namespaces().await);
        let now = now();
        for namespace in &moved {
            self.store
                .record_change(namespace, now, ChangeSource::PublishingOn)
                .await;
        }
        moved
    }

    #[cfg(test)]
    pub(crate) fn store_for_test(&self) -> &PackageStateStore {
        &self.store
    }

    /// A clock on which `namespace` changed `ago` and was already walked this
    /// session, so the next walk with changes keeps that time.
    #[cfg(test)]
    pub(crate) async fn changed_for_test(namespace: &Namespace, ago: std::time::Duration) -> Self {
        let clock = Self::new(PackageStateStore::in_memory());
        clock
            .store
            .record_change(namespace, now() - ago, ChangeSource::Edit)
            .await;
        clock
            .walked
            .lock()
            .expect("walked lock")
            .insert(namespace.clone());
        clock
    }
}

/// Whether any of `changed` (paths relative to `home`) changed after `since`.
///
/// ctime, because no tool can set it: the copies that keep old edit times
/// (`cp -p`, `rsync -a`, an unzip) still get a new one. A deleted file has none,
/// so its nearest existing folder answers, since removing an entry changes it.
pub(crate) fn changed_while_closed<'a>(
    home: &Path,
    changed: impl IntoIterator<Item = &'a PathBuf>,
    since: SystemTime,
) -> bool {
    changed.into_iter().any(|relative| {
        let mut path = home.join(relative);
        loop {
            if let Ok(meta) = std::fs::symlink_metadata(&path) {
                return change_time(&meta).is_none_or(|at| at > since);
            }
            if path == home || !path.pop() {
                return true;
            }
        }
    })
}

#[cfg(unix)]
fn change_time(meta: &std::fs::Metadata) -> Option<SystemTime> {
    use std::os::unix::fs::MetadataExt;
    let secs = u64::try_from(meta.ctime()).ok()?;
    let nanos = u32::try_from(meta.ctime_nsec()).ok()?;
    // To the millisecond, as `last_alive_at` is kept.
    let millis = nanos / 1_000_000;
    Some(SystemTime::UNIX_EPOCH + std::time::Duration::new(secs, millis * 1_000_000))
}

/// Windows has no ctime in the standard library, and copy tools can restore
/// both the creation and the edit time, so every changed file counts as changed.
#[cfg(not(unix))]
fn change_time(_meta: &std::fs::Metadata) -> Option<SystemTime> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::time::Duration;

    use crate::quilt::lineage::Change;
    use crate::quilt::manifest::ManifestRow;

    fn ns() -> Namespace {
        ("acme", "demo").into()
    }

    fn status_with(paths: &[&str]) -> InstalledPackageStatus {
        let mut status = InstalledPackageStatus::default();
        for path in paths {
            status
                .changes
                .insert(PathBuf::from(path), Change::Added(ManifestRow::default()));
        }
        status
    }

    fn hour() -> Duration {
        Duration::from_hours(1)
    }

    #[tokio::test]
    async fn a_walk_with_changes_and_no_time_is_first_seen() {
        let clock = LastChange::new(PackageStateStore::in_memory());
        let before = now();
        let at = clock.observe(&ns(), None, &status_with(&["a"])).await;
        assert!(at.is_some_and(|at| at >= before));
        assert_eq!(clock.get(&ns()).await, at);
    }

    #[tokio::test]
    async fn a_clean_walk_deletes_the_time() {
        let clock = LastChange::new(PackageStateStore::in_memory());
        clock.on_signal(&ns()).await;
        assert_eq!(clock.observe(&ns(), None, &status_with(&[])).await, None);
        assert_eq!(clock.get(&ns()).await, None);
    }

    #[tokio::test]
    async fn a_later_walk_keeps_the_signal_time() {
        let store = PackageStateStore::in_memory();
        let old = now() - hour();
        store.record_change(&ns(), old, ChangeSource::Edit).await;
        let clock = LastChange::new(store);
        // Not the first walk: the restart check is behind us.
        clock.walked.lock().unwrap().insert(ns());
        assert_eq!(
            clock.observe(&ns(), None, &status_with(&["a"])).await,
            Some(old)
        );
    }

    #[tokio::test]
    async fn a_signal_moves_the_time_to_now() {
        let store = PackageStateStore::in_memory();
        store
            .record_change(&ns(), now() - hour(), ChangeSource::Edit)
            .await;
        let clock = LastChange::new(store);
        let before = now();
        clock.on_signal(&ns()).await;
        assert!(clock.get(&ns()).await.is_some_and(|at| at >= before));
    }

    /// ctime is Unix only; on Windows every changed file counts as changed.
    #[cfg(unix)]
    #[tokio::test]
    async fn after_a_restart_an_untouched_package_keeps_its_time() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("a"), b"x").unwrap();
        let store = PackageStateStore::in_memory();
        // The app saw the change after the file was written.
        let saved = now() + Duration::from_millis(5);
        store.record_change(&ns(), saved, ChangeSource::Edit).await;
        let clock = LastChange::new(store);
        assert_eq!(
            clock
                .observe(&ns(), Some(dir.path()), &status_with(&["a"]))
                .await,
            Some(saved)
        );
    }

    #[tokio::test]
    async fn after_a_restart_a_package_changed_while_closed_waits_from_now() {
        let dir = tempfile::TempDir::new().unwrap();
        let store = PackageStateStore::in_memory();
        // The app last saw a change an hour ago; the file changed since.
        store
            .record_change(&ns(), now() - hour(), ChangeSource::Edit)
            .await;
        std::fs::write(dir.path().join("a"), b"x").unwrap();
        // Later sessions ran, without walking this package.
        store.set_last_alive_at(now()).await;
        let clock = LastChange::new(store);
        let before = now();
        let at = clock
            .observe(&ns(), Some(dir.path()), &status_with(&["a"]))
            .await;
        assert!(at.is_some_and(|at| at >= before));
    }

    #[tokio::test]
    async fn a_package_whose_folder_is_unknown_waits_from_now() {
        let store = PackageStateStore::in_memory();
        store
            .record_change(&ns(), now() - hour(), ChangeSource::Edit)
            .await;
        let clock = LastChange::new(store);
        let before = now();
        let at = clock.observe(&ns(), None, &status_with(&["a"])).await;
        assert!(at.is_some_and(|at| at >= before));
    }

    #[cfg(unix)]
    #[test]
    fn a_deleted_file_is_judged_by_its_folder() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        let gone = [PathBuf::from("sub/gone.txt")];
        assert!(changed_while_closed(dir.path(), &gone, now() - hour()));
        assert!(!changed_while_closed(dir.path(), &gone, now() + hour()));
    }

    #[tokio::test]
    async fn a_walk_after_a_signal_keeps_the_signal_time() {
        // The walk limit coalesced the signal; the next walk finds what it
        // announced and must not restart the wait again.
        let clock = LastChange::changed_for_test(&ns(), hour()).await;
        assert!(
            clock
                .observe(&ns(), None, &status_with(&["a"]))
                .await
                .is_some()
        );
        clock.on_signal(&ns()).await;
        let signalled = clock.get(&ns()).await;
        tokio::time::sleep(Duration::from_millis(5)).await;
        assert_eq!(
            clock.observe(&ns(), None, &status_with(&["a", "b"])).await,
            signalled
        );
    }

    #[tokio::test]
    async fn publishing_on_moves_every_package_with_changes_or_a_time() {
        let store = PackageStateStore::in_memory();
        let stored: Namespace = ("acme", "stored").into();
        store
            .record_change(&stored, now() - hour(), ChangeSource::Edit)
            .await;
        let clock = LastChange::new(store);
        let before = now();
        let moved = clock.on_publishing_on(BTreeSet::from([ns()])).await;
        assert_eq!(moved, BTreeSet::from([ns(), stored.clone()]));
        for namespace in [ns(), stored] {
            assert!(clock.get(&namespace).await.is_some_and(|at| at >= before));
        }
    }

    #[tokio::test]
    async fn a_publish_deletes_the_time() {
        let clock = LastChange::new(PackageStateStore::in_memory());
        clock.on_signal(&ns()).await;
        clock.on_published(&ns()).await;
        assert_eq!(clock.get(&ns()).await, None);
    }

    #[tokio::test]
    async fn a_walk_that_finds_new_changes_restarts_the_wait() {
        // The watcher is off: only walks see the edits.
        let clock = LastChange::changed_for_test(&ns(), hour()).await;
        let old = clock.get(&ns()).await;
        assert_eq!(clock.observe(&ns(), None, &status_with(&["a"])).await, old);
        assert_eq!(
            clock.observe(&ns(), None, &status_with(&["a"])).await,
            old,
            "the same changes keep the time"
        );
        let before = now();
        let at = clock.observe(&ns(), None, &status_with(&["a", "b"])).await;
        assert!(at.is_some_and(|at| at >= before));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn an_edit_the_app_saw_is_not_one_made_while_closed() {
        let dir = tempfile::TempDir::new().unwrap();
        let store = PackageStateStore::new(dir.path().join("state.db"));
        let clock = LastChange::new(store);
        std::fs::write(dir.path().join("a"), b"x").unwrap();
        clock.on_signal(&ns()).await;
        let saved = clock.get(&ns()).await;
        drop(clock);

        // Relaunch: the store is the same file.
        let clock = LastChange::new(PackageStateStore::new(dir.path().join("state.db")));
        assert_eq!(
            clock
                .observe(&ns(), Some(dir.path()), &status_with(&["a"]))
                .await,
            saved
        );
    }

    /// Windows refuses to delete a file that is open.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_store_deleted_while_running_waits_a_full_wait() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("state.db");
        let clock = LastChange::new(PackageStateStore::new(path.clone()));
        clock.walked.lock().unwrap().insert(ns());
        let old = now() - hour();
        clock
            .store
            .record_change(&ns(), old, ChangeSource::Edit)
            .await;
        std::fs::remove_file(&path).unwrap();
        let before = now();
        let at = clock.observe(&ns(), None, &status_with(&["a"])).await;
        assert!(at.is_some_and(|at| at >= before));
    }
}
