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
//!
//! The engine that holds the lock also keeps the browser's token in
//! `<data dir>/web-token`, so a tab opened on an earlier run reconnects after a
//! restart, and the desktop app's last port in `<data dir>/web-port`.

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
const TOKEN_FILE: &str = "web-token";
const PORT_FILE: &str = "web-port";

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
        write_private(
            &self.data_dir.join(INFO_FILE),
            serde_json::to_string_pretty(&info)?.as_bytes(),
        )
    }

    /// The token in `web-token`, made and saved first when there is none.
    ///
    /// # Errors
    ///
    /// When the file cannot be read or written.
    pub fn web_token(&self) -> io::Result<String> {
        match fs::read_to_string(self.data_dir.join(TOKEN_FILE)) {
            Ok(text) if is_token(text.trim()) => Ok(text.trim().to_owned()),
            Ok(_) => self.new_web_token(),
            Err(err) if err.kind() == io::ErrorKind::NotFound => self.new_web_token(),
            Err(err) => Err(err),
        }
    }

    /// Replace `web-token` with a fresh token and return it.
    ///
    /// # Errors
    ///
    /// When the file cannot be written.
    pub fn new_web_token(&self) -> io::Result<String> {
        let token = crate::server::new_token();
        write_private(&self.data_dir.join(TOKEN_FILE), token.as_bytes())?;
        Ok(token)
    }

    /// The port saved in `web-port`, if any.
    #[must_use]
    pub fn saved_port(&self) -> Option<u16> {
        fs::read_to_string(self.data_dir.join(PORT_FILE))
            .ok()?
            .trim()
            .parse()
            .ok()
            .filter(|port| *port != 0)
    }

    /// Save `port` in `web-port`.
    ///
    /// # Errors
    ///
    /// When the file cannot be written.
    pub fn save_port(&self, port: u16) -> io::Result<()> {
        write_private(&self.data_dir.join(PORT_FILE), port.to_string().as_bytes())
    }
}

/// A token as [`crate::server::new_token`] makes it: hex digits only.
fn is_token(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Write `path` whole (owner-only on Unix): a temp file, then a rename.
fn write_private(path: &Path, contents: &[u8]) -> io::Result<()> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    let mut options = fs::OpenOptions::new();
    options.create(true).truncate(true).write(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options.open(&tmp)?;
    file.write_all(contents)?;
    file.sync_all()?;
    drop(file);
    fs::rename(tmp, path)
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

/// The browser's token: the saved one when this process holds the engine
/// lock, otherwise (or when the file fails) a fresh one for this run only.
#[must_use]
pub fn web_token() -> String {
    if let Some(lock) = HELD.lock().expect("engine lock").as_ref() {
        match lock.web_token() {
            Ok(token) => return token,
            Err(err) => warn!("failed to read or write {TOKEN_FILE}: {err}"),
        }
    }
    crate::server::new_token()
}

/// Save a fresh token for the next start. The running server keeps its own.
///
/// # Errors
///
/// When this process holds no engine lock or the file cannot be written.
pub fn reset_web_token() -> io::Result<()> {
    let held = HELD.lock().expect("engine lock");
    let lock = held
        .as_ref()
        .ok_or_else(|| io::Error::other("this process is not the engine for its data folder"))?;
    lock.new_web_token().map(drop)
}

/// The desktop app's last port, when this process holds the engine lock.
#[must_use]
pub fn saved_port() -> Option<u16> {
    HELD.lock().expect("engine lock").as_ref()?.saved_port()
}

/// Remember `port` for the desktop app's next start.
pub fn save_port(port: u16) {
    if let Some(lock) = HELD.lock().expect("engine lock").as_ref()
        && let Err(err) = lock.save_port(port)
    {
        warn!("failed to write {PORT_FILE}: {err}");
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

    #[test]
    fn the_token_and_the_port_survive_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let Acquired::Held(lock) = acquire(dir.path()).unwrap() else {
            panic!("a fresh folder was locked");
        };
        let token = lock.web_token().unwrap();
        assert!(is_token(&token));
        assert_eq!(lock.web_token().unwrap(), token);
        assert_eq!(lock.saved_port(), None);
        lock.save_port(51234).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(dir.path().join(TOKEN_FILE))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        drop(lock);

        let Acquired::Held(lock) = acquire(dir.path()).unwrap() else {
            panic!("the folder was still locked");
        };
        assert_eq!(lock.web_token().unwrap(), token);
        assert_eq!(lock.saved_port(), Some(51234));
        let fresh = lock.new_web_token().unwrap();
        assert_ne!(fresh, token);
        assert_eq!(lock.web_token().unwrap(), fresh);
    }

    #[test]
    fn a_damaged_token_file_is_replaced() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(TOKEN_FILE), "not a token\n").unwrap();
        let Acquired::Held(lock) = acquire(dir.path()).unwrap() else {
            panic!("a fresh folder was locked");
        };
        let token = lock.web_token().unwrap();
        assert!(is_token(&token));
        assert_eq!(
            fs::read_to_string(dir.path().join(TOKEN_FILE)).unwrap(),
            token
        );
    }
}
