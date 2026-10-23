//! One engine per data folder.
//!
//! An engine (desktop or headless) holds an exclusive OS lock on
//! `<data dir>/engine.lock` for the life of its process. Only the OS lock
//! counts: the file itself stays behind after every run and is never deleted,
//! and after a crash the OS has already released the lock, so the next start
//! takes it.
//!
//! While holding the lock, an engine that serves a browser writes
//! `<data dir>/engine.json` with its address. A second start reads that file,
//! and trusts it only while the lock is still held by someone else.

use std::fs;
use std::io;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::Deserialize;
use serde::Serialize;

use crate::telemetry::prelude::*;

const LOCK_FILE: &str = "engine.lock";
const INFO_FILE: &str = "engine.json";

/// What a running engine publishes in `engine.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineInfo {
    pub pid: u32,
    pub url: String,
    pub token: String,
    pub version: String,
}

impl EngineInfo {
    /// The address to open, token included.
    #[must_use]
    pub fn url_with_token(&self) -> String {
        format!("{}/?t={}", self.url.trim_end_matches('/'), self.token)
    }
}

/// The held lock. Dropping it closes the file, which releases the OS lock.
#[derive(Debug)]
pub struct EngineLock {
    _file: fs::File,
    data_dir: PathBuf,
}

/// The outcome of [`acquire`].
#[derive(Debug)]
pub enum Acquired {
    /// This process is now the engine for the folder.
    Held(EngineLock),
    /// Another process holds the lock. Its `engine.json`, when it has
    /// published one yet.
    Running(Option<EngineInfo>),
}

fn open_lock_file(data_dir: &Path) -> io::Result<fs::File> {
    fs::create_dir_all(data_dir)?;
    let mut options = fs::OpenOptions::new();
    options.create(true).truncate(false).write(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(data_dir.join(LOCK_FILE))
}

/// Try to become the engine for `data_dir` without waiting.
///
/// # Errors
///
/// When the folder or the lock file cannot be opened.
pub fn acquire(data_dir: &Path) -> io::Result<Acquired> {
    let file = open_lock_file(data_dir)?;
    match file.try_lock() {
        Ok(()) => {
            // Anything there was written by an engine that is gone.
            let _ = fs::remove_file(data_dir.join(INFO_FILE));
            Ok(Acquired::Held(EngineLock {
                _file: file,
                data_dir: data_dir.to_path_buf(),
            }))
        }
        Err(fs::TryLockError::WouldBlock) => Ok(Acquired::Running(read_info(data_dir))),
        Err(fs::TryLockError::Error(err)) => Err(err),
    }
}

/// Like [`acquire`], but when another engine holds the lock, waits up to a
/// few seconds for it to publish its address.
///
/// # Errors
///
/// As [`acquire`].
pub fn acquire_or_wait_for_info(data_dir: &Path) -> io::Result<Acquired> {
    let mut acquired = acquire(data_dir)?;
    for _ in 0..30 {
        if !matches!(acquired, Acquired::Running(None)) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
        acquired = acquire(data_dir)?;
    }
    Ok(acquired)
}

fn read_info(data_dir: &Path) -> Option<EngineInfo> {
    let text = fs::read_to_string(data_dir.join(INFO_FILE)).ok()?;
    serde_json::from_str(&text).ok()
}

impl EngineLock {
    /// Write `engine.json` (owner-only on Unix) for later starts to find.
    ///
    /// # Errors
    ///
    /// When the file cannot be written.
    pub fn write_info(&self, url: &str, token: &str) -> io::Result<()> {
        let info = EngineInfo {
            pid: std::process::id(),
            url: url.to_owned(),
            token: token.to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
        };
        let tmp = self.data_dir.join(format!("{INFO_FILE}.tmp"));
        let mut options = fs::OpenOptions::new();
        options.create(true).truncate(true).write(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let mut file = options.open(&tmp)?;
        file.write_all(serde_json::to_string_pretty(&info)?.as_bytes())?;
        file.sync_all()?;
        drop(file);
        fs::rename(tmp, self.data_dir.join(INFO_FILE))
    }
}

/// The lock this process holds, kept until exit.
static HELD: Mutex<Option<EngineLock>> = Mutex::new(None);

/// Keep `lock` for the life of the process.
pub fn hold(lock: EngineLock) {
    *HELD.lock().expect("engine lock") = Some(lock);
}

/// Publish this engine's address in `engine.json`, once its server is up.
/// A no-op when this process holds no engine lock.
pub fn publish(url: &str, token: &str) {
    if let Some(lock) = HELD.lock().expect("engine lock").as_ref()
        && let Err(err) = lock.write_info(url, token)
    {
        warn!("failed to write engine.json: {err}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_handle_cannot_lock_until_the_first_is_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let first = open_lock_file(dir.path()).unwrap();
        first.try_lock().unwrap();
        let second = open_lock_file(dir.path()).unwrap();
        assert!(matches!(
            second.try_lock(),
            Err(fs::TryLockError::WouldBlock)
        ));
        drop(first);
        second.try_lock().unwrap();
    }

    #[test]
    fn acquire_reports_the_running_engine() {
        let dir = tempfile::tempdir().unwrap();
        let Acquired::Held(lock) = acquire(dir.path()).unwrap() else {
            panic!("a fresh folder was locked");
        };
        assert!(matches!(
            acquire(dir.path()).unwrap(),
            Acquired::Running(None)
        ));
        lock.write_info("http://localhost:1", "tok").unwrap();
        let Acquired::Running(Some(info)) = acquire(dir.path()).unwrap() else {
            panic!("engine.json was not read");
        };
        assert_eq!(info.url_with_token(), "http://localhost:1/?t=tok");
        assert_eq!(info.pid, std::process::id());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(dir.path().join(INFO_FILE))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        drop(lock);
        assert!(matches!(acquire(dir.path()).unwrap(), Acquired::Held(_)));
    }

    #[test]
    fn a_leftover_lock_file_with_no_holder_does_not_block() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(LOCK_FILE), b"").unwrap();
        fs::write(
            dir.path().join(INFO_FILE),
            r#"{"pid":1,"url":"http://x","token":"t","version":"0"}"#,
        )
        .unwrap();
        let Acquired::Held(_lock) = acquire(dir.path()).unwrap() else {
            panic!("a leftover lock file blocked the start");
        };
        assert!(dir.path().join(LOCK_FILE).exists());
        // The stale address is gone, so nobody trusts it.
        assert!(!dir.path().join(INFO_FILE).exists());
    }
}
