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
use std::sync::Arc;
use std::sync::Mutex;
use std::time::SystemTime;

use quilt_uri::Namespace;

use crate::quilt::lineage::InstalledPackageStatus;
use crate::quilt::state_store::ChangeSource;
use crate::quilt::state_store::PackageStateStore;

/// Now, to the millisecond the store keeps, so a time handed out equals the
/// one read back.
pub(crate) fn now() -> SystemTime {
    floor_millis(SystemTime::now())
}

fn floor_millis(at: SystemTime) -> SystemTime {
    let since = at
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    SystemTime::UNIX_EPOCH
        + std::time::Duration::from_millis(u64::try_from(since.as_millis()).unwrap_or(u64::MAX))
}

/// What a walk knows of the package's commit that waits to be pushed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingCommit {
    None,
    /// One waits, made at this time.
    At(SystemTime),
    /// The lineage could not be read.
    Unknown,
}

impl PendingCommit {
    #[must_use]
    pub fn of(lineage: &crate::quilt::lineage::PackageLineage) -> Self {
        lineage
            .commit
            .as_ref()
            .map_or(Self::None, |commit| Self::At(commit.timestamp.into()))
    }
}

pub(crate) struct LastChange {
    store: Arc<PackageStateStore>,
    /// Packages walked since launch. The first walk of each one asks whether it
    /// changed while the app was closed.
    walked: Mutex<BTreeSet<Namespace>>,
    /// What each package's last walk found changed, so a walk that finds
    /// something new restarts the wait even when no watcher signal came.
    last_seen: Mutex<BTreeMap<Namespace, String>>,
    /// The time of each pending commit known to be the app's own.
    own_commits: Mutex<BTreeMap<Namespace, SystemTime>>,
    /// Packages whose own commit no walk has seen yet: the next walk finds
    /// the committed files gone from the changes, and that is no new edit.
    fresh_commits: Mutex<BTreeSet<Namespace>>,
}

impl LastChange {
    #[cfg(test)]
    pub(crate) fn new(store: PackageStateStore) -> Self {
        Self::shared(Arc::new(store))
    }

    /// A clock on a store the app's walks also use, for their stat cache.
    pub(crate) fn shared(store: Arc<PackageStateStore>) -> Self {
        Self {
            store,
            walked: Mutex::new(BTreeSet::new()),
            last_seen: Mutex::new(BTreeMap::new()),
            own_commits: Mutex::new(BTreeMap::new()),
            fresh_commits: Mutex::new(BTreeSet::new()),
        }
    }

    /// The folder watcher heard a change in `namespace`.
    pub(crate) async fn on_signal(&self, namespace: &Namespace) {
        self.store
            .record_change(namespace, now(), ChangeSource::Edit)
            .await;
    }

    /// The app left `commit` pending on `namespace`: by its own commit, or by
    /// a publish whose push failed. A walk that finds it is not finding a commit
    /// made elsewhere.
    pub(crate) fn on_own_commit(&self, namespace: &Namespace, commit: PendingCommit) {
        if let PendingCommit::At(at) = commit {
            self.own_commits
                .lock()
                .expect("own_commits lock")
                .insert(namespace.clone(), at);
            self.fresh_commits
                .lock()
                .expect("fresh_commits lock")
                .insert(namespace.clone());
        }
    }

    fn own_commit(&self, namespace: &Namespace, at: SystemTime) -> bool {
        self.own_commits
            .lock()
            .expect("own_commits lock")
            .get(namespace)
            == Some(&at)
    }

    /// The package was committed in the app. The commit is a change: its push
    /// waits a full wait from now.
    pub(crate) async fn on_commit(&self, namespace: &Namespace, commit: PendingCommit) {
        self.on_own_commit(namespace, commit);
        self.store
            .record_change(namespace, now(), ChangeSource::Commit)
            .await;
    }

    /// Take a walk's result for `namespace` and return its last-change time,
    /// `None` when it has nothing to publish.
    ///
    /// `home` is the package's folder, for the restart check; `None` when it
    /// could not be resolved, which counts as changed. An unknown
    /// `pending_commit` keeps a stored time.
    pub(crate) async fn observe(
        &self,
        namespace: &Namespace,
        home: Option<&Path>,
        status: &InstalledPackageStatus,
        pending_commit: PendingCommit,
    ) -> Option<SystemTime> {
        // Marked before any early return: the restart check belongs to the
        // first walk, whatever that walk found.
        let first_walk = self
            .walked
            .lock()
            .expect("walked lock")
            .insert(namespace.clone());
        let seen = crate::autopull::reporter::changes_fingerprint(&status.changes);
        let before = self
            .last_seen
            .lock()
            .expect("last_seen lock")
            .insert(namespace.clone(), seen.clone());
        let now = now();
        if matches!(pending_commit, PendingCommit::None) {
            self.own_commits
                .lock()
                .expect("own_commits lock")
                .remove(namespace);
        }
        let own_commit = match pending_commit {
            PendingCommit::At(at) => self.own_commit(namespace, at),
            PendingCommit::None | PendingCommit::Unknown => false,
        };
        if status.changes.is_empty() {
            match pending_commit {
                PendingCommit::None => {
                    self.store.forget(namespace).await;
                    return None;
                }
                // Not clean: the commit still has to be pushed, and waits from
                // the last change, the commit included.
                PendingCommit::At(_) => {}
                PendingCommit::Unknown => return self.get(namespace).await,
            }
        }
        // Different changes from the last walk move the time, signal or not: a
        // walk can't tell which edits an earlier signal covered. This is also
        // what restarts the wait when the watcher is off. Changes that moved
        // into the app's own commit are not new.
        let fresh = self
            .fresh_commits
            .lock()
            .expect("fresh_commits lock")
            .remove(namespace);
        let into_own_commit = own_commit && fresh && status.changes.is_empty();
        if !into_own_commit && before.is_some_and(|before| before != seen) {
            self.store
                .record_change(namespace, now, ChangeSource::Edit)
                .await;
            return Some(now);
        }
        if self.store.record_first_seen(namespace, now).await {
            return Some(now);
        }
        // A store that can't answer counts as just seen.
        let Some(saved) = self.store.last_change(namespace).await else {
            return Some(now);
        };
        // The saved time is when the app last saw this package change, so a
        // later commit was made where the app could not see it, such as
        // `quilt commit` in a terminal.
        if let PendingCommit::At(at) = pending_commit
            && floor_millis(at) > saved.last_change_at
            && !own_commit
        {
            // Known from now on, so a commit dated ahead of the clock restarts
            // the wait once, not on every walk.
            self.own_commits
                .lock()
                .expect("own_commits lock")
                .insert(namespace.clone(), at);
            self.store
                .record_change(namespace, now, ChangeSource::FirstSeen)
                .await;
            return Some(now);
        }
        // Likewise a changed file with a later ctime changed while the app was
        // not looking: closed, or before a session that never walked it.
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
        self.own_commits
            .lock()
            .expect("own_commits lock")
            .remove(namespace);
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
/// The change time, because no tool can set it: the copies that keep old edit
/// times (`cp -p`, `rsync -a`, an unzip) still get a new one. A deleted file has
/// none, so its nearest existing folder answers, since removing an entry
/// changes it. A file system without change times counts as changed.
pub(crate) fn changed_while_closed<'a>(
    home: &Path,
    changed: impl IntoIterator<Item = &'a PathBuf>,
    since: SystemTime,
) -> bool {
    changed.into_iter().any(|relative| {
        let mut path = home.join(relative);
        loop {
            if let Ok(stamp) = crate::quilt::file_stamp::stamp(&path) {
                // To the millisecond, as the store keeps times.
                return stamp.changed.is_none_or(|at| floor_millis(at) > since);
            }
            if path == home || !path.pop() {
                return true;
            }
        }
    })
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
        let at = clock
            .observe(&ns(), None, &status_with(&["a"]), PendingCommit::None)
            .await;
        assert!(at.is_some_and(|at| at >= before));
        assert_eq!(clock.get(&ns()).await, at);
    }

    #[tokio::test]
    async fn a_clean_walk_deletes_the_time() {
        let clock = LastChange::new(PackageStateStore::in_memory());
        clock.on_signal(&ns()).await;
        assert_eq!(
            clock
                .observe(&ns(), None, &status_with(&[]), PendingCommit::None)
                .await,
            None
        );
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
            clock
                .observe(&ns(), None, &status_with(&["a"]), PendingCommit::None)
                .await,
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
                .observe(
                    &ns(),
                    Some(dir.path()),
                    &status_with(&["a"]),
                    PendingCommit::None
                )
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
        let clock = LastChange::new(store);
        let before = now();
        let at = clock
            .observe(
                &ns(),
                Some(dir.path()),
                &status_with(&["a"]),
                PendingCommit::None,
            )
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
        let at = clock
            .observe(&ns(), None, &status_with(&["a"]), PendingCommit::None)
            .await;
        assert!(at.is_some_and(|at| at >= before));
    }

    #[test]
    fn a_deleted_file_is_judged_by_its_folder() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        let gone = [PathBuf::from("sub/gone.txt")];
        assert!(changed_while_closed(dir.path(), &gone, now() - hour()));
        assert!(!changed_while_closed(dir.path(), &gone, now() + hour()));
    }

    #[tokio::test]
    async fn a_walk_with_new_changes_moves_the_time_even_after_a_signal() {
        // The walk can't tell whether an edit came after the signal, so it
        // moves the time.
        let clock = LastChange::changed_for_test(&ns(), hour()).await;
        assert!(
            clock
                .observe(&ns(), None, &status_with(&["a"]), PendingCommit::None)
                .await
                .is_some()
        );
        clock.on_signal(&ns()).await;
        let signalled = clock.get(&ns()).await.expect("signalled");
        tokio::time::sleep(Duration::from_millis(5)).await;
        let at = clock
            .observe(&ns(), None, &status_with(&["a", "b"]), PendingCommit::None)
            .await;
        assert!(at.is_some_and(|at| at > signalled));
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
    async fn a_commit_sets_the_time_and_keeps_it_until_the_push() {
        let clock = LastChange::changed_for_test(&ns(), hour()).await;
        let before = now();
        clock
            .on_commit(&ns(), PendingCommit::At(SystemTime::UNIX_EPOCH))
            .await;
        let committed = clock.get(&ns()).await;
        assert!(committed.is_some_and(|at| at >= before));
        // The commit took every change: the tree is clean, the commit unpushed.
        assert_eq!(
            clock
                .observe(
                    &ns(),
                    None,
                    &status_with(&[]),
                    PendingCommit::At(SystemTime::UNIX_EPOCH)
                )
                .await,
            committed,
            "a commit waiting to be pushed is not clean"
        );
        assert_eq!(
            clock
                .observe(&ns(), None, &status_with(&[]), PendingCommit::None)
                .await,
            None,
            "pushed and clean"
        );
        assert_eq!(clock.get(&ns()).await, None);
    }

    #[tokio::test]
    async fn a_commit_made_elsewhere_waits_from_when_the_app_sees_it() {
        // `quilt commit` in a terminal: no time stored for the package.
        let clock = LastChange::new(PackageStateStore::in_memory());
        let before = now();
        let at = clock
            .observe(
                &ns(),
                None,
                &status_with(&[]),
                PendingCommit::At(SystemTime::UNIX_EPOCH),
            )
            .await;
        assert!(at.is_some_and(|at| at >= before));
    }

    #[tokio::test]
    async fn a_commit_newer_than_the_saved_time_waits_from_now() {
        // The app saw an edit an hour ago; then `quilt commit` ran in a
        // terminal, or replaced the commit the app knew of.
        let clock = LastChange::changed_for_test(&ns(), hour()).await;
        let committed = PendingCommit::At(now() - Duration::from_mins(1));
        let before = now();
        let at = clock
            .observe(&ns(), None, &status_with(&[]), committed)
            .await;
        assert!(at.is_some_and(|at| at >= before));
    }

    #[tokio::test]
    async fn a_publish_whose_push_failed_keeps_the_time() {
        // The retry backs off; it doesn't wait a whole new wait.
        let clock = LastChange::changed_for_test(&ns(), hour()).await;
        let saved = clock.get(&ns()).await;
        let committed = PendingCommit::At(now());
        clock.on_own_commit(&ns(), committed);
        assert_eq!(
            clock
                .observe(&ns(), None, &status_with(&[]), committed)
                .await,
            saved
        );
    }

    #[tokio::test]
    async fn a_failed_push_after_a_walk_keeps_the_time() {
        // The walk before the publish saw the files; the walk after sees them
        // in the app's own commit, and must not take that for a new edit.
        let clock = LastChange::changed_for_test(&ns(), hour()).await;
        let saved = clock
            .observe(&ns(), None, &status_with(&["a"]), PendingCommit::None)
            .await;
        let committed = PendingCommit::At(now());
        clock.on_own_commit(&ns(), committed);
        assert_eq!(
            clock
                .observe(&ns(), None, &status_with(&[]), committed)
                .await,
            saved
        );
        assert_eq!(
            clock
                .observe(&ns(), None, &status_with(&[]), committed)
                .await,
            saved,
            "the same commit, walked again"
        );
    }

    #[tokio::test]
    async fn a_later_commit_after_a_publish_is_judged_afresh() {
        let clock = LastChange::changed_for_test(&ns(), hour()).await;
        let own = now();
        clock.on_own_commit(&ns(), PendingCommit::At(own));
        clock
            .observe(&ns(), None, &status_with(&[]), PendingCommit::At(own))
            .await;
        // `quilt commit` replaces it.
        let before = now();
        let at = clock
            .observe(
                &ns(),
                None,
                &status_with(&[]),
                PendingCommit::At(own + Duration::from_secs(1)),
            )
            .await;
        assert!(at.is_some_and(|at| at >= before));
    }

    #[tokio::test]
    async fn edits_after_a_commit_restart_the_wait() {
        // The watcher is off: only the walk sees the edit made after the commit.
        let clock = LastChange::changed_for_test(&ns(), hour()).await;
        clock
            .observe(&ns(), None, &status_with(&["a"]), PendingCommit::None)
            .await;
        let commit = PendingCommit::At(now());
        clock.on_commit(&ns(), commit).await;
        tokio::time::sleep(Duration::from_millis(5)).await;
        let before = now();
        let at = clock
            .observe(&ns(), None, &status_with(&["b"]), commit)
            .await;
        assert!(at.is_some_and(|at| at >= before));
    }

    #[tokio::test]
    async fn a_revert_after_the_apps_commit_restarts_the_wait() {
        let clock = LastChange::changed_for_test(&ns(), hour()).await;
        clock
            .observe(&ns(), None, &status_with(&["a"]), PendingCommit::None)
            .await;
        let commit = PendingCommit::At(now());
        clock.on_own_commit(&ns(), commit);
        clock.observe(&ns(), None, &status_with(&[]), commit).await;
        clock
            .observe(&ns(), None, &status_with(&["b"]), commit)
            .await;
        tokio::time::sleep(Duration::from_millis(5)).await;
        let before = now();
        // `b` reverted to what the commit holds.
        let at = clock.observe(&ns(), None, &status_with(&[]), commit).await;
        assert!(at.is_some_and(|at| at >= before));
    }

    #[tokio::test]
    async fn the_restart_check_runs_on_the_first_walk_only() {
        // Clean at launch, then edited with the watcher off: the wait runs from
        // when the walk first saw the edit, and later walks keep that time.
        let clock = LastChange::new(PackageStateStore::in_memory());
        clock
            .observe(&ns(), None, &status_with(&[]), PendingCommit::None)
            .await;
        let seen = clock
            .observe(&ns(), None, &status_with(&["a"]), PendingCommit::None)
            .await;
        tokio::time::sleep(Duration::from_millis(5)).await;
        assert_eq!(
            clock
                .observe(&ns(), None, &status_with(&["a"]), PendingCommit::None)
                .await,
            seen
        );
    }

    #[tokio::test]
    async fn a_commit_dated_ahead_of_the_clock_restarts_the_wait_once() {
        let clock = LastChange::changed_for_test(&ns(), hour()).await;
        let ahead = PendingCommit::At(now() + hour());
        let first = clock.observe(&ns(), None, &status_with(&[]), ahead).await;
        tokio::time::sleep(Duration::from_millis(5)).await;
        assert_eq!(
            clock.observe(&ns(), None, &status_with(&[]), ahead).await,
            first
        );
    }

    #[tokio::test]
    async fn a_commit_the_app_made_keeps_its_time() {
        let clock = LastChange::changed_for_test(&ns(), hour()).await;
        let committed_at = now();
        clock
            .on_commit(&ns(), PendingCommit::At(committed_at))
            .await;
        let saved = clock.get(&ns()).await;
        assert_eq!(
            clock
                .observe(
                    &ns(),
                    None,
                    &status_with(&[]),
                    PendingCommit::At(committed_at)
                )
                .await,
            saved
        );
    }

    #[tokio::test]
    async fn an_unknown_commit_state_keeps_the_time() {
        let clock = LastChange::changed_for_test(&ns(), hour()).await;
        let saved = clock.get(&ns()).await;
        assert_eq!(
            clock
                .observe(&ns(), None, &status_with(&[]), PendingCommit::Unknown)
                .await,
            saved
        );
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
        assert_eq!(
            clock
                .observe(&ns(), None, &status_with(&["a"]), PendingCommit::None)
                .await,
            old
        );
        assert_eq!(
            clock
                .observe(&ns(), None, &status_with(&["a"]), PendingCommit::None)
                .await,
            old,
            "the same changes keep the time"
        );
        let before = now();
        let at = clock
            .observe(&ns(), None, &status_with(&["a", "b"]), PendingCommit::None)
            .await;
        assert!(at.is_some_and(|at| at >= before));
    }

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
                .observe(
                    &ns(),
                    Some(dir.path()),
                    &status_with(&["a"]),
                    PendingCommit::None
                )
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
        let at = clock
            .observe(&ns(), None, &status_with(&["a"]), PendingCommit::None)
            .await;
        assert!(at.is_some_and(|at| at >= before));
    }
}
