//! Whether a newer `quilt-cli` is published, read from crates.io's index at
//! most every six hours, as the desktop app reads its feed.
//!
//! The answer is one line for stderr. Every failure is silent: a check must
//! never slow or fail a command.

use std::future::Future;
use std::io::IsTerminal;
use std::io::Write;
use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;
use std::time::Instant;
use std::time::SystemTime;

use semver::Version;
use serde::Deserialize;
use serde::Serialize;

/// The crate's sparse-index file: one JSON line per published version.
const INDEX: &str = "https://index.crates.io/qu/il/quilt-cli";

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// How long an answer stands, a failed check's included.
const FRESH: u64 = 6 * 60 * 60;

/// The longest a check may add to a command, when one is due.
const TIMEOUT: Duration = Duration::from_secs(2);

/// What the last check found, kept between runs.
#[derive(Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Cache {
    /// Seconds since the Unix epoch.
    checked_at: u64,
    /// The newest version seen; kept when a later check fails.
    latest: Option<String>,
}

/// One line of the index; other fields are ignored.
#[derive(Deserialize)]
struct IndexLine {
    vers: String,
    yanked: bool,
}

/// A check running beside the command.
pub struct Pending {
    started: Instant,
    line: mpsc::Receiver<String>,
}

/// Starts the check on a thread of its own, unless this run skips it. The
/// thread is never joined: a DNS lookup the timeout cannot cancel must not
/// hold the process open.
pub fn start() -> Option<Pending> {
    if !std::io::stderr().is_terminal()
        || std::env::var_os("CI").is_some()
        || std::env::var_os("QUILT_NO_UPDATE_CHECK").is_some_and(|v| v == "1")
    {
        return None;
    }
    let (send, line) = mpsc::channel();
    std::thread::Builder::new()
        .spawn(move || {
            let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            else {
                return;
            };
            if let Some(found) = runtime.block_on(notice()) {
                let _ = send.send(found);
            }
        })
        .ok()?;
    Some(Pending {
        started: Instant::now(),
        line,
    })
}

impl Pending {
    /// The line to print, waiting at most until [`TIMEOUT`] after the start.
    pub fn line(self) -> Option<String> {
        self.line
            .recv_timeout(TIMEOUT.saturating_sub(self.started.elapsed()))
            .ok()
    }
}

/// Prints the notice in yellow, after a blank line. `anstream` drops the
/// colour for `NO_COLOR` and terminals without it.
pub fn print(line: &str) {
    let yellow = anstyle::Style::new().fg_color(Some(anstyle::AnsiColor::Yellow.into()));
    anstream::eprintln!("\n{yellow}{line}{yellow:#}");
}

/// The line to print, if a newer release is out.
async fn notice() -> Option<String> {
    let path = dirs::cache_dir()?.join("quilt").join("update-check.json");
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .ok()?
        .as_secs();
    let current = Version::parse(VERSION).ok()?;
    check(&path, now, &current, fetch_latest()).await
}

/// Reads the cache at `path`, awaits `fetch` only when the cache is
/// [`FRESH`] old or missing, and saves what it learned. The attempt is saved before
/// the fetch, since the process may exit before a slow one returns.
async fn check(
    path: &Path,
    now: u64,
    current: &Version,
    fetch: impl Future<Output = Option<Version>>,
) -> Option<String> {
    let cache: Option<Cache> = std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok());
    let latest = match cache {
        Some(cache) if !due(&cache, now) => cache.latest,
        cache => {
            let mut cache = Cache {
                checked_at: now,
                latest: cache.and_then(|cache| cache.latest),
            };
            // Unsaved, every run would ask again.
            if save(path, &cache).is_ok()
                && let Some(found) = fetch.await
            {
                cache.latest = Some(found.to_string());
                let _ = save(path, &cache);
            }
            cache.latest
        }
    };
    let latest = Version::parse(&latest?).ok()?;
    (latest > *current).then(|| {
        format!("quilt {latest} is available (you have {current}): cargo binstall quilt-cli")
    })
}

/// [`FRESH`] has passed, or the clock went back past the last check.
fn due(cache: &Cache, now: u64) -> bool {
    now < cache.checked_at || now - cache.checked_at >= FRESH
}

/// Replaces the file whole, so a reader never sees half of it.
fn save(path: &Path, cache: &Cache) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let mut file = tempfile::NamedTempFile::new_in(dir)?;
    file.write_all(&serde_json::to_vec(cache)?)?;
    file.persist(path)?;
    Ok(())
}

async fn fetch_latest() -> Option<Version> {
    let body = reqwest::Client::builder()
        .user_agent(format!("quilt-cli/{VERSION}"))
        .timeout(TIMEOUT)
        .build()
        .ok()?
        .get(INDEX)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .ok()?
        .text()
        .await
        .ok()?;
    latest(&body)
}

/// The highest release in the index that is not yanked. Pre-releases are
/// skipped: `cargo binstall` would not install one.
fn latest(index: &str) -> Option<Version> {
    index
        .lines()
        .filter_map(|line| serde_json::from_str::<IndexLine>(line).ok())
        .filter(|line| !line.yanked)
        .filter_map(|line| Version::parse(&line.vers).ok())
        .filter(|v| v.pre.is_empty())
        .max()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap()
    }

    #[test]
    fn the_latest_is_the_highest_release_not_yanked() {
        let index = r#"{"name":"quilt-cli","vers":"0.40.2","deps":[],"yanked":false}
{"name":"quilt-cli","vers":"0.9.0","deps":[],"yanked":false}
{"name":"quilt-cli","vers":"0.41.0","deps":[],"yanked":true}
{"name":"quilt-cli","vers":"0.42.0-rc.1","deps":[],"yanked":false}
not json
{"name":"quilt-cli","vers":"not a version","yanked":false}
"#;
        assert_eq!(latest(index), Some(v("0.40.2")));
    }

    #[test]
    fn an_index_with_nothing_usable_has_no_latest() {
        assert_eq!(latest(""), None);
        assert_eq!(
            latest(r#"{"name":"quilt-cli","vers":"0.1.0","yanked":true}"#),
            None
        );
    }

    #[test]
    fn a_check_is_due_after_six_hours() {
        let cache = Cache {
            checked_at: 1_000_000,
            latest: None,
        };
        assert!(!due(&cache, 1_000_000));
        assert!(!due(&cache, 1_000_000 + FRESH - 1));
        assert!(due(&cache, 1_000_000 + FRESH));
        assert!(due(&cache, 999_999), "the clock went back");
    }

    fn cache_file() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("quilt").join("update-check.json");
        (dir, path)
    }

    fn read(path: &Path) -> Cache {
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
    }

    #[tokio::test]
    async fn a_first_run_asks_and_saves_the_answer() {
        let (_dir, path) = cache_file();
        let line = check(&path, 100, &v("0.40.2"), async { Some(v("0.41.0")) }).await;
        assert_eq!(
            line.as_deref(),
            Some("quilt 0.41.0 is available (you have 0.40.2): cargo binstall quilt-cli")
        );
        assert_eq!(
            read(&path),
            Cache {
                checked_at: 100,
                latest: Some("0.41.0".into()),
            }
        );
    }

    #[tokio::test]
    async fn within_six_hours_the_saved_answer_is_used() {
        let (_dir, path) = cache_file();
        save(
            &path,
            &Cache {
                checked_at: 100,
                latest: Some("0.41.0".into()),
            },
        )
        .unwrap();
        let line = check(&path, 100 + FRESH - 1, &v("0.40.2"), async {
            panic!("asked the network while the answer was fresh")
        })
        .await;
        assert!(line.is_some());
    }

    #[tokio::test]
    async fn a_failed_check_is_recorded_and_keeps_what_was_known() {
        let (_dir, path) = cache_file();
        save(
            &path,
            &Cache {
                checked_at: 100,
                latest: Some("0.41.0".into()),
            },
        )
        .unwrap();
        let line = check(&path, 100 + FRESH, &v("0.40.2"), async { None }).await;
        assert!(line.is_some());
        assert_eq!(
            read(&path),
            Cache {
                checked_at: 100 + FRESH,
                latest: Some("0.41.0".into()),
            }
        );
    }

    /// A process that exits during a slow fetch still skips the next checks.
    #[tokio::test]
    async fn the_attempt_is_saved_before_the_fetch() {
        let (_dir, path) = cache_file();
        save(
            &path,
            &Cache {
                checked_at: 100,
                latest: Some("0.41.0".into()),
            },
        )
        .unwrap();
        let fetch = async {
            assert_eq!(
                read(&path),
                Cache {
                    checked_at: 100 + FRESH,
                    latest: Some("0.41.0".into()),
                }
            );
            None
        };
        check(&path, 100 + FRESH, &v("0.40.2"), fetch).await;
    }

    #[tokio::test]
    async fn a_cache_that_cannot_be_saved_skips_the_fetch() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a file");
        std::fs::write(&file, "").unwrap();
        let path = file.join("update-check.json");
        let line = check(&path, 100, &v("0.40.2"), async {
            panic!("asked the network with no cache to record it")
        })
        .await;
        assert_eq!(line, None);
    }

    #[tokio::test]
    async fn the_same_or_an_older_release_prints_nothing() {
        let (_dir, path) = cache_file();
        assert_eq!(
            check(&path, 100, &v("0.40.2"), async { Some(v("0.40.2")) }).await,
            None
        );
        let (_dir, path) = cache_file();
        assert_eq!(
            check(&path, 100, &v("0.41.0-dev"), async { Some(v("0.40.2")) }).await,
            None
        );
    }

    #[tokio::test]
    async fn an_unreadable_cache_counts_as_missing() {
        let (_dir, path) = cache_file();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "{").unwrap();
        let line = check(&path, 100, &v("0.40.2"), async { Some(v("0.41.0")) }).await;
        assert!(line.is_some());
        assert_eq!(read(&path).checked_at, 100);
    }
}
