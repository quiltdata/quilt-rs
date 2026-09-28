//! An exclusive lock on a path, held against every other holder in this
//! process and in any other.
//!
//! Two layers, taken in this order:
//!
//! - a [`tokio::sync::Mutex`] from a process-wide registry keyed by the
//!   canonical path, so two handles that spell the path differently still find
//!   one lock, and a task waiting on it does not block a runtime thread;
//! - an OS lock on the file itself ([`std::fs::File::lock`]: `flock` on macOS
//!   and Linux, `LockFileEx` on Windows), for another process on the same
//!   directory. It blocks its thread, so it runs in `spawn_blocking`, and only
//!   ever after the mutex: two open files in one process conflict on it too,
//!   and without the mutex first a second task would block its own thread.
//!
//! The OS drops the file lock when its holder dies. The file is never renamed
//! or deleted, since a lock on a replaced file guards nothing.

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::LazyLock;
use std::sync::Mutex;

use tokio::sync::OwnedMutexGuard;

use crate::Res;

type Registry = Mutex<HashMap<PathBuf, Arc<tokio::sync::Mutex<()>>>>;

static REGISTRY: LazyLock<Registry> = LazyLock::new(Registry::default);

/// Held while the lock is. Dropping it closes the file, which releases the OS
/// lock, and then releases the mutex.
#[derive(Debug)]
pub struct LockGuard {
    // Declared first so it drops first: the OS lock goes before the mutex lets
    // the next task in this process at it.
    _file: fs::File,
    _held: OwnedMutexGuard<()>,
}

fn joined<T>(result: Result<std::io::Result<T>, tokio::task::JoinError>) -> Res<T> {
    Ok(result.map_err(std::io::Error::other)??)
}

/// Opens `path`, creating it and its parent directory if needed, and finds
/// its mutex in the registry.
async fn open(path: &Path) -> Res<(fs::File, Arc<tokio::sync::Mutex<()>>)> {
    let path = path.to_path_buf();
    let (file, key) = joined(
        tokio::task::spawn_blocking(move || {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            let file = fs::OpenOptions::new()
                .create(true)
                .truncate(false)
                .write(true)
                .open(&path)?;
            let key = fs::canonicalize(&path)?;
            Ok((file, key))
        })
        .await,
    )?;
    let mutex = Arc::clone(
        REGISTRY
            .lock()
            .expect("lock registry")
            .entry(key)
            .or_default(),
    );
    Ok((file, mutex))
}

/// Locks `path` exclusively, creating it and its parent directory if needed.
///
/// A cancelled caller cannot abort the blocking wait for the OS lock; if it
/// is cancelled mid-wait, the thread takes the lock later and drops it at once
/// with its unread result.
pub(crate) async fn lock_exclusive(path: &Path) -> Res<LockGuard> {
    let (file, mutex) = open(path).await?;
    let held = mutex.lock_owned().await;
    let file = joined(
        tokio::task::spawn_blocking(move || {
            file.lock()?;
            Ok(file)
        })
        .await,
    )?;
    Ok(LockGuard {
        _file: file,
        _held: held,
    })
}

/// [`lock_exclusive`] without the wait: `None` when another holder, in this
/// process or another, has the lock now.
pub(crate) async fn try_lock_exclusive(path: &Path) -> Res<Option<LockGuard>> {
    let (file, mutex) = open(path).await?;
    let Ok(held) = mutex.try_lock_owned() else {
        return Ok(None);
    };
    // `try_lock` does not block, but it is still a file system call.
    let file = joined(
        tokio::task::spawn_blocking(move || match file.try_lock() {
            Ok(()) => Ok(Some(file)),
            Err(fs::TryLockError::WouldBlock) => Ok(None),
            Err(fs::TryLockError::Error(err)) => Err(err),
        })
        .await,
    )?;
    // Busy in another process: `held` drops here and frees the mutex again.
    Ok(file.map(|file| LockGuard {
        _file: file,
        _held: held,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::time::Duration;

    use test_log::test;

    /// Two spellings of one path find one lock: the second waits for the first.
    #[test(tokio::test)]
    async fn two_spellings_of_one_path_share_the_lock() -> Res {
        let dir = tempfile::TempDir::new()?;
        let plain = dir.path().join("data.json.lock");
        let dotted = dir.path().join(".").join("data.json.lock");

        let first = lock_exclusive(&plain).await?;
        let second =
            tokio::time::timeout(Duration::from_millis(200), lock_exclusive(&dotted)).await;
        assert!(second.is_err(), "the second handle must wait");

        drop(first);
        tokio::time::timeout(Duration::from_secs(5), lock_exclusive(&dotted))
            .await
            .expect("the lock is free once the first guard drops")?;
        Ok(())
    }

    /// A try on a held lock says it is busy at once, and takes it once free.
    #[test(tokio::test)]
    async fn a_try_on_a_held_lock_is_busy() -> Res {
        let dir = tempfile::TempDir::new()?;
        let path = dir.path().join("demo.lock");

        let first = lock_exclusive(&path).await?;
        assert!(try_lock_exclusive(&path).await?.is_none(), "held in this process");

        drop(first);
        assert!(try_lock_exclusive(&path).await?.is_some(), "free once dropped");
        Ok(())
    }

    /// Different paths never contend.
    #[test(tokio::test)]
    async fn different_paths_do_not_contend() -> Res {
        let dir = tempfile::TempDir::new()?;
        let _a = lock_exclusive(&dir.path().join("a.lock")).await?;
        tokio::time::timeout(
            Duration::from_secs(5),
            lock_exclusive(&dir.path().join("b.lock")),
        )
        .await
        .expect("another path is free")?;
        Ok(())
    }

    const CHILD_LOCK: &str = "QUILT_RS_TEST_CHILD_LOCK";

    /// Not a test on its own: the child half of
    /// [`another_process_waits_for_the_lock`]. It takes the lock, says so, and
    /// holds it until the parent removes the `hold` file.
    #[test]
    #[ignore = "run only as the child of another_process_waits_for_the_lock"]
    fn child_holds_the_lock() -> Res {
        let Ok(dir) = std::env::var(CHILD_LOCK) else {
            return Ok(());
        };
        let dir = PathBuf::from(dir);
        let runtime = tokio::runtime::Runtime::new()?;
        runtime.block_on(async {
            let _held = lock_exclusive(&dir.join("data.json.lock")).await?;
            fs::write(dir.join("locked"), b"")?;
            while dir.join("hold").exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            Ok(())
        })
    }

    /// The cross-process layer: while another process holds the lock, this one
    /// waits for it, and gets it once that process lets go.
    #[test(tokio::test)]
    async fn another_process_waits_for_the_lock() -> Res {
        let dir = tempfile::TempDir::new()?;
        fs::write(dir.path().join("hold"), b"")?;
        let mut child = std::process::Command::new(std::env::current_exe()?)
            .args([
                "--exact",
                "io::storage::lock::tests::child_holds_the_lock",
                "--ignored",
                "--nocapture",
            ])
            .env(CHILD_LOCK, dir.path())
            .spawn()?;
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        while !dir.path().join("locked").exists() {
            assert!(
                std::time::Instant::now() < deadline,
                "the child never took the lock"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        let lock_path = dir.path().join("data.json.lock");
        let waiting = tokio::time::timeout(Duration::from_millis(300), lock_exclusive(&lock_path));
        assert!(waiting.await.is_err(), "the other process holds the lock");

        assert!(
            try_lock_exclusive(&lock_path).await?.is_none(),
            "a try sees the other process too"
        );

        fs::remove_file(dir.path().join("hold"))?;
        tokio::time::timeout(Duration::from_secs(30), lock_exclusive(&lock_path))
            .await
            .expect("the lock is free once the other process lets go")?;
        assert!(child.wait()?.success());
        Ok(())
    }
}
