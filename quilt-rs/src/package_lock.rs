//! One lineage writer per package at a time.
//!
//! Every public writer of a package's lineage entry holds that package's
//! lock, `.quilt/locks/<owner>/<name>.lock`, from its first read of the entry
//! to its write. Two writers of one package then run one after the other, so
//! the entry a writer writes is always the one it read. Different packages
//! have different lock files and never wait on each other; their writes meet
//! only at the short lock around each splice of `data.json` (see
//! [`DomainLineageIo::update`](crate::lineage::DomainLineageIo::update)).
//!
//! Lock order is fixed: the package's lock, then the short one, never the
//! reverse. The lock is not reentrant, so only public entry points take it,
//! and the calls inside them assume it is held.
//!
//! Lock files are never deleted, not even on uninstall: a writer waiting on
//! a deleted file would lock an unlinked inode while a new writer locked a
//! new file at the same path, and both would run.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::LazyLock;
use std::sync::RwLock;

use crate::Res;
use crate::io::storage::LockGuard;
use crate::io::storage::Storage;
use crate::paths::DomainPaths;
use quilt_uri::Namespace;

type Notice = Arc<dyn Fn() + Send + Sync>;

static WAIT_NOTICE: LazyLock<RwLock<Option<Notice>>> = LazyLock::new(RwLock::default);

/// Sets what runs, once per wait, when a writer finds its package's lock held
/// by another writer and is about to wait for it. Replaces any earlier one.
///
/// The CLI prints that it is waiting; the desktop app sets none, since its
/// spinner already covers the wait.
///
/// # Panics
///
/// Panics if a notice panicked while the hook was being replaced.
pub fn on_package_lock_wait(notice: impl Fn() + Send + Sync + 'static) {
    *WAIT_NOTICE.write().expect("wait notice") = Some(Arc::new(notice));
}

/// Takes `namespace`'s lock, waiting for it if another writer holds it.
pub(crate) async fn lock(
    storage: &impl Storage,
    paths: &DomainPaths,
    namespace: &Namespace,
) -> Res<LockGuard> {
    let path = paths.package_lock(namespace);
    if let Some(guard) = storage.try_lock_exclusive(&path).await? {
        return Ok(guard);
    }
    let notice = WAIT_NOTICE.read().expect("wait notice").clone();
    if let Some(notice) = notice {
        notice();
    }
    storage.lock_exclusive(path).await
}

/// Takes `namespace`'s lock if it is free now; `None` when another writer
/// holds it.
pub(crate) async fn try_lock(
    storage: &impl Storage,
    paths: &DomainPaths,
    namespace: &Namespace,
) -> Res<Option<LockGuard>> {
    storage
        .try_lock_exclusive(paths.package_lock(namespace))
        .await
}

/// A package's lock, held until this drops. Only [`Self::take`] and
/// [`Self::try_take`] make one, so a function given one knows the lock is
/// held, and need not take it again, which would wait on itself.
#[derive(Debug)]
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "the ref store that takes it is not used yet")
)]
pub(crate) struct PackageLock {
    namespace: Namespace,
    file: PathBuf,
    _guard: LockGuard,
}

#[cfg_attr(
    not(test),
    expect(dead_code, reason = "the ref store that takes it is not used yet")
)]
impl PackageLock {
    /// [`lock`], kept as proof.
    pub(crate) async fn take(
        storage: &impl Storage,
        paths: &DomainPaths,
        namespace: &Namespace,
    ) -> Res<Self> {
        Ok(Self {
            namespace: namespace.clone(),
            file: paths.package_lock(namespace),
            _guard: lock(storage, paths, namespace).await?,
        })
    }

    /// [`try_lock`], kept as proof.
    pub(crate) async fn try_take(
        storage: &impl Storage,
        paths: &DomainPaths,
        namespace: &Namespace,
    ) -> Res<Option<Self>> {
        Ok(try_lock(storage, paths, namespace)
            .await?
            .map(|guard| Self {
                namespace: namespace.clone(),
                file: paths.package_lock(namespace),
                _guard: guard,
            }))
    }

    /// The package locked, if the lock is `paths`' own: a lock taken in
    /// another domain proves nothing here.
    pub(crate) fn namespace_in(&self, paths: &DomainPaths) -> Option<&Namespace> {
        (self.file == paths.package_lock(&self.namespace)).then_some(&self.namespace)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::atomic::AtomicUsize;
    use std::sync::atomic::Ordering;
    use std::time::Duration;

    use test_log::test;

    use crate::io::storage::LocalStorage;

    /// A free lock says nothing; a held one fires the notice once, then
    /// waits, and is taken once the holder lets go.
    #[test(tokio::test)]
    async fn a_writer_waiting_on_a_held_package_fires_the_notice_once() -> Res {
        static WAITS: AtomicUsize = AtomicUsize::new(0);
        on_package_lock_wait(|| {
            WAITS.fetch_add(1, Ordering::SeqCst);
        });
        let dir = tempfile::TempDir::new()?;
        let paths = DomainPaths::new(dir.path().to_path_buf());
        let storage = LocalStorage::new();
        let namespace: Namespace = ("acme", "demo").into();

        let held = lock(&storage, &paths, &namespace).await?;
        assert_eq!(WAITS.load(Ordering::SeqCst), 0, "a free lock says nothing");
        assert!(try_lock(&storage, &paths, &namespace).await?.is_none());

        let mut waiting = std::pin::pin!(lock(&storage, &paths, &namespace));
        let early = tokio::time::timeout(Duration::from_millis(200), &mut waiting).await;
        assert!(early.is_err(), "the second writer waits");
        assert_eq!(WAITS.load(Ordering::SeqCst), 1);

        drop(held);
        tokio::time::timeout(Duration::from_secs(5), waiting)
            .await
            .expect("taken once the holder lets go")?;
        assert_eq!(WAITS.load(Ordering::SeqCst), 1, "once per wait");
        Ok(())
    }
}
