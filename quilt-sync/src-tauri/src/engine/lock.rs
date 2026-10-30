//! One engine per data folder, and what a browser needs to reach it.
//!
//! An engine holds an exclusive OS lock on `<data folder>/engine.lock` for the
//! life of its process. Only the OS lock counts: the file stays behind after
//! every run, and after a crash the OS has already released the lock, so the
//! next start takes it.
//!
//! Beside the lock, all owner-only:
//!
//! - `engine.json`: the running engine's address, for a second start to open.
//!   Trusted only while the lock is held by someone else.
//! - `web-token`: the browser's token, made once, so an open tab reconnects
//!   after a restart.
//! - `web-port`: the last port, tried first on the next start.

use std::fs;
use std::io;
use std::io::Write;
use std::net::IpAddr;
use std::net::SocketAddr;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;

use serde::Deserialize;
use serde::Serialize;
use tokio::net::TcpListener;

const LOCK_FILE: &str = "engine.lock";
const INFO_FILE: &str = "engine.json";
const TOKEN_FILE: &str = "web-token";
const PORT_FILE: &str = "web-port";

/// What a running engine publishes in `engine.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineInfo {
    pub pid: u32,
    /// The address without the token, e.g. `http://127.0.0.1:7878`.
    pub url: String,
    pub version: String,
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
    /// Another process holds the lock. Its `engine.json`, once it has written
    /// one.
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
/// When the folder or the lock file can't be opened.
pub fn acquire(data_dir: &Path) -> io::Result<Acquired> {
    let file = open_lock_file(data_dir)?;
    match file.try_lock() {
        Ok(()) => {
            // Anything there was written by an engine that is gone.
            match fs::remove_file(data_dir.join(INFO_FILE)) {
                Err(err) if err.kind() != io::ErrorKind::NotFound => return Err(err),
                _ => {}
            }
            Ok(Acquired::Held(EngineLock {
                _file: file,
                data_dir: data_dir.to_path_buf(),
            }))
        }
        Err(fs::TryLockError::WouldBlock) => Ok(Acquired::Running(read_info(data_dir))),
        Err(fs::TryLockError::Error(err)) => Err(err),
    }
}

/// Like [`acquire`], but when another engine holds the lock, waits a few
/// seconds for it to write its address: it writes it only once it serves.
///
/// # Errors
///
/// As [`acquire`].
pub async fn acquire_or_wait_for_info(data_dir: &Path) -> io::Result<Acquired> {
    let mut acquired = acquire(data_dir)?;
    for _ in 0..50 {
        if !matches!(acquired, Acquired::Running(None)) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        acquired = acquire(data_dir)?;
    }
    Ok(acquired)
}

fn read_info(data_dir: &Path) -> Option<EngineInfo> {
    let text = fs::read_to_string(data_dir.join(INFO_FILE)).ok()?;
    serde_json::from_str(&text).ok()
}

/// The saved browser token, without making one. For a second start that
/// opens the running engine's page.
#[must_use]
pub fn read_token(data_dir: &Path) -> Option<String> {
    let text = fs::read_to_string(data_dir.join(TOKEN_FILE)).ok()?;
    let token = text.trim();
    is_token(token).then(|| token.to_owned())
}

/// The address to open: `url` with the token, which the page keeps and drops
/// from the address.
#[must_use]
pub fn link(url: &str, token: &str) -> String {
    format!("{}/?t={token}", url.trim_end_matches('/'))
}

impl EngineLock {
    /// Write `engine.json` for a later start to find.
    ///
    /// # Errors
    ///
    /// When the file can't be written.
    pub fn write_info(&self, url: &str, version: &str) -> io::Result<()> {
        let info = EngineInfo {
            pid: std::process::id(),
            url: url.to_owned(),
            version: version.to_owned(),
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
    /// When the file can't be read or written.
    pub fn web_token(&self) -> io::Result<String> {
        match fs::read_to_string(self.data_dir.join(TOKEN_FILE)) {
            Ok(text) if is_token(text.trim()) => Ok(text.trim().to_owned()),
            Ok(_) => self.new_web_token(),
            Err(err) if err.kind() == io::ErrorKind::NotFound => self.new_web_token(),
            Err(err) => Err(err),
        }
    }

    fn new_web_token(&self) -> io::Result<String> {
        let token = uuid::Uuid::new_v4().simple().to_string();
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
    /// When the file can't be written.
    pub fn save_port(&self, port: u16) -> io::Result<()> {
        write_private(&self.data_dir.join(PORT_FILE), port.to_string().as_bytes())
    }
}

/// The desktop app's port: the last one when it is free, otherwise any free
/// one.
///
/// # Errors
///
/// When not even a free port can be bound.
pub async fn bind_desktop(ip: IpAddr, last: Option<u16>) -> io::Result<TcpListener> {
    if let Some(port) = last
        && let Ok(listener) = TcpListener::bind(SocketAddr::new(ip, port)).await
    {
        return Ok(listener);
    }
    TcpListener::bind(SocketAddr::new(ip, 0)).await
}

/// A token as [`EngineLock::web_token`] makes it: hex digits only.
fn is_token(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Write `path` whole and owner-only: a temp file, then a rename.
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

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;

    use super::*;

    const LOOPBACK: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);

    #[cfg(unix)]
    fn mode(path: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn a_second_engine_cannot_lock_the_folder_until_the_first_is_gone() {
        let dir = tempfile::tempdir().unwrap();
        let Acquired::Held(lock) = acquire(dir.path()).unwrap() else {
            panic!("a fresh folder was locked");
        };
        assert!(matches!(
            acquire(dir.path()).unwrap(),
            Acquired::Running(None)
        ));

        lock.write_info("http://127.0.0.1:1", "1.2.3").unwrap();
        let Acquired::Running(Some(info)) = acquire(dir.path()).unwrap() else {
            panic!("engine.json was not read");
        };
        assert_eq!(info.url, "http://127.0.0.1:1");
        assert_eq!(info.version, "1.2.3");
        assert_eq!(info.pid, std::process::id());
        #[cfg(unix)]
        assert_eq!(mode(&dir.path().join(INFO_FILE)), 0o600);

        drop(lock);
        assert!(matches!(acquire(dir.path()).unwrap(), Acquired::Held(_)));
    }

    #[test]
    fn a_lock_file_left_by_a_crash_does_not_block() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(LOCK_FILE), b"").unwrap();
        fs::write(
            dir.path().join(INFO_FILE),
            r#"{"pid":1,"url":"http://x","version":"0"}"#,
        )
        .unwrap();
        let Acquired::Held(_lock) = acquire(dir.path()).unwrap() else {
            panic!("a leftover lock file blocked the start");
        };
        // The dead engine's address is gone, so nobody opens it.
        assert!(!dir.path().join(INFO_FILE).exists());
    }

    #[test]
    fn the_token_and_the_port_survive_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let Acquired::Held(lock) = acquire(dir.path()).unwrap() else {
            panic!("a fresh folder was locked");
        };
        assert_eq!(read_token(dir.path()), None);
        let token = lock.web_token().unwrap();
        assert!(is_token(&token));
        assert_eq!(lock.web_token().unwrap(), token);
        #[cfg(unix)]
        assert_eq!(mode(&dir.path().join(TOKEN_FILE)), 0o600);
        assert_eq!(lock.saved_port(), None);
        lock.save_port(51234).unwrap();
        drop(lock);

        let Acquired::Held(lock) = acquire(dir.path()).unwrap() else {
            panic!("the folder was still locked");
        };
        assert_eq!(lock.web_token().unwrap(), token);
        assert_eq!(read_token(dir.path()), Some(token));
        assert_eq!(lock.saved_port(), Some(51234));
    }

    #[test]
    fn a_damaged_token_file_is_replaced() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(TOKEN_FILE), "not a token\n").unwrap();
        assert_eq!(read_token(dir.path()), None);
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

    #[tokio::test]
    async fn the_desktop_keeps_its_port_while_free_and_takes_any_when_not() {
        let first = bind_desktop(LOOPBACK, None).await.unwrap();
        let port = first.local_addr().unwrap().port();
        drop(first);

        let again = bind_desktop(LOOPBACK, Some(port)).await.unwrap();
        assert_eq!(again.local_addr().unwrap().port(), port);

        // `again` still holds the port.
        let other = bind_desktop(LOOPBACK, Some(port)).await.unwrap();
        assert_ne!(other.local_addr().unwrap().port(), port);
    }

    #[test]
    fn the_link_carries_the_token() {
        assert_eq!(
            link("http://127.0.0.1:7878", "abc"),
            "http://127.0.0.1:7878/?t=abc"
        );
        assert_eq!(
            link("http://127.0.0.1:7878/", "abc"),
            "http://127.0.0.1:7878/?t=abc"
        );
    }
}
