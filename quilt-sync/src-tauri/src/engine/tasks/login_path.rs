//! The PATH of the user's login shell, for the agents `QuiltSync` starts.
//!
//! Started by launchd, `QuiltSync` gets `/usr/bin:/bin:/usr/sbin:/sbin`. An agent
//! given that PATH can't find the user's tools (`gh`, Homebrew's `git`, `deno`)
//! and may search the home folder for them, which raises macOS's privacy
//! prompts in `QuiltSync`'s name.

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub use shell::login_path;

/// Windows apps get the user's PATH as it is.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
#[allow(clippy::unused_async)]
pub async fn login_path() -> Option<&'static str> {
    None
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod shell {
    use std::ffi::OsStr;
    use std::ffi::OsString;
    use std::process::Stdio;
    use std::sync::OnceLock;
    use std::time::Duration;
    use std::time::Instant;

    use tokio::io::AsyncReadExt;
    use tokio::sync::Mutex;

    use crate::engine::telemetry::prelude::*;

    /// Brackets the PATH in the shell's output, which may also carry whatever
    /// the user's startup files print.
    const MARKER: &str = "__QUILTSYNC_PATH__";

    /// A slow startup file costs a run this long, at most.
    const TIMEOUT: Duration = Duration::from_secs(5);

    /// The login shell's PATH, kept once found; `None` keeps `QuiltSync`'s own.
    /// Runs waiting on a lookup that fails share its `None`, so none waits more
    /// than one timeout; a run that starts later asks again. A tool installed
    /// later shows up after a restart.
    pub async fn login_path() -> Option<&'static str> {
        static PATH: OnceLock<String> = OnceLock::new();
        static FAILED_AT: Mutex<Option<Instant>> = Mutex::const_new(None);
        shared(&PATH, &FAILED_AT, async || {
            from_shell(&shell(), TIMEOUT).await
        })
        .await
    }

    /// `path` once `lookup` has found it. One lookup at a time; the callers
    /// that waited on a failed one get its `None` without looking again.
    async fn shared<'a>(
        path: &'a OnceLock<String>,
        failed_at: &Mutex<Option<Instant>>,
        lookup: impl AsyncFnOnce() -> Option<String>,
    ) -> Option<&'a str> {
        if let Some(path) = path.get() {
            return Some(path);
        }
        let asked = Instant::now();
        let mut failed_at = failed_at.lock().await;
        if let Some(path) = path.get() {
            return Some(path);
        }
        if failed_at.is_some_and(|failed| failed > asked) {
            return None;
        }
        let Some(found) = lookup().await else {
            warn!("tasks: no PATH from the login shell, the agent gets QuiltSync's own");
            *failed_at = Some(Instant::now());
            return None;
        };
        Some(path.get_or_init(|| found))
    }

    fn shell() -> OsString {
        let fallback = if cfg!(target_os = "macos") {
            "/bin/zsh"
        } else {
            "/bin/sh"
        };
        std::env::var_os("SHELL").unwrap_or_else(|| fallback.into())
    }

    /// Interactive as well as login, so zsh reads `.zshrc` too; bash reads
    /// only `.bash_profile` or `.profile`. Reading stops at the second marker,
    /// so a job a startup file leaves holding stdout doesn't hold the read.
    async fn from_shell(shell: &OsStr, timeout: Duration) -> Option<String> {
        let mut command = tokio::process::Command::new(shell);
        command
            .arg("-ilc")
            .arg(format!(r#"printf '{MARKER}%s{MARKER}' "$PATH""#))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        // SAFETY: `setsid` is async-signal-safe and touches no memory of ours.
        // Its own session leaves an interactive shell no terminal to take from
        // `QuiltSync`, and makes one group of all it starts.
        unsafe {
            command.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
        let mut child = command.spawn().ok()?;
        let mut stdout = child.stdout.take()?;
        let read = async {
            let mut output = Vec::new();
            let mut chunk = [0; 4096];
            loop {
                let read = stdout.read(&mut chunk).await.ok()?;
                output.extend_from_slice(&chunk[..read]);
                let path = between_markers(&String::from_utf8_lossy(&output));
                if path.is_some() || read == 0 {
                    return path;
                }
            }
        };
        let path = tokio::time::timeout(timeout, read).await.ok().flatten();
        // Without a PATH, kill what the startup files left running too, even
        // once the shell has exited, or each retry adds to it. The shell is
        // not reaped before this, so its PID still names its group.
        if path.is_none()
            && let Some(group) = child.id().and_then(|id| libc::pid_t::try_from(id).ok())
        {
            // SAFETY: `kill` takes plain integers and touches no memory of ours.
            unsafe { libc::kill(-group, libc::SIGKILL) };
        }
        path
    }

    fn between_markers(output: &str) -> Option<String> {
        let (_, rest) = output.split_once(MARKER)?;
        let (path, _) = rest.split_once(MARKER)?;
        Some(path.to_string()).filter(|path| !path.is_empty())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        use std::sync::atomic::AtomicUsize;
        use std::sync::atomic::Ordering;

        #[test]
        fn the_path_is_read_from_between_the_markers() {
            assert_eq!(
                between_markers(&format!(
                    "Welcome back!\n{MARKER}/opt/homebrew/bin:/usr/bin{MARKER}"
                ))
                .as_deref(),
                Some("/opt/homebrew/bin:/usr/bin")
            );
            assert_eq!(between_markers("no markers at all"), None);
            assert_eq!(between_markers(&format!("{MARKER}{MARKER}")), None);
            assert_eq!(between_markers(&format!("{MARKER}cut short")), None);
        }

        /// A lookup that counts itself and takes a moment, giving `found`.
        async fn lookup(lookups: &AtomicUsize, found: Option<&str>) -> Option<String> {
            lookups.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(100)).await;
            found.map(str::to_string)
        }

        #[tokio::test]
        async fn callers_waiting_on_a_lookup_share_it() {
            let path = OnceLock::new();
            let failed_at = Mutex::new(None);
            let lookups = AtomicUsize::new(0);

            let waited = tokio::join!(
                shared(&path, &failed_at, async || lookup(&lookups, None).await),
                shared(&path, &failed_at, async || lookup(&lookups, None).await),
                shared(&path, &failed_at, async || lookup(&lookups, None).await),
            );
            assert_eq!(waited, (None, None, None));
            assert_eq!(lookups.load(Ordering::SeqCst), 1, "one failed lookup");

            let later = shared(&path, &failed_at, async || {
                lookup(&lookups, Some("/login/bin")).await
            })
            .await;
            assert_eq!(later, Some("/login/bin"), "a later caller looks again");
            let after = shared(&path, &failed_at, async || lookup(&lookups, None).await).await;
            assert_eq!(after, Some("/login/bin"));
            assert_eq!(lookups.load(Ordering::SeqCst), 2, "a found PATH is kept");
        }

        #[tokio::test]
        async fn a_posix_shell_gives_its_path() {
            let path = from_shell("/bin/sh".as_ref(), TIMEOUT).await;
            assert!(
                path.as_ref().is_some_and(|path| path.contains("/bin")),
                "{path:?}"
            );
        }

        #[tokio::test]
        async fn a_missing_or_silent_shell_gives_none() {
            let missing = from_shell("/nonexistent/quiltsync-test-shell".as_ref(), TIMEOUT).await;
            assert_eq!(missing, None);
            assert_eq!(from_shell("true".as_ref(), TIMEOUT).await, None);
        }

        /// A shell whose startup leaves a job holding stdout: the PATH is read
        /// without waiting for the job.
        #[tokio::test]
        async fn a_job_left_holding_stdout_does_not_hold_the_read() -> std::io::Result<()> {
            let dir = tempfile::TempDir::new()?;
            let shell = dir.path().join("shell");
            tokio::fs::write(
                &shell,
                format!("#!/bin/sh\nsleep 10 &\nprintf '{MARKER}/held/bin{MARKER}'\n"),
            )
            .await?;
            tokio::fs::set_permissions(&shell, std::os::unix::fs::PermissionsExt::from_mode(0o755))
                .await?;
            let started = std::time::Instant::now();
            let path = from_shell(shell.as_os_str(), TIMEOUT).await;
            assert_eq!(path.as_deref(), Some("/held/bin"));
            assert!(started.elapsed() < TIMEOUT, "{:?}", started.elapsed());
            Ok(())
        }

        /// Runs a shell whose startup starts a job holding stdout, then `end`s
        /// without printing the PATH; whether the job is gone afterwards.
        async fn job_outlives_a_shell_that(end: &str) -> std::io::Result<bool> {
            let dir = tempfile::TempDir::new()?;
            let shell = dir.path().join("shell");
            let job = dir.path().join("job");
            tokio::fs::write(
                &shell,
                format!(
                    "#!/bin/sh\nsleep 30 &\necho $! > '{}'\n{end}\n",
                    job.display()
                ),
            )
            .await?;
            tokio::fs::set_permissions(&shell, std::os::unix::fs::PermissionsExt::from_mode(0o755))
                .await?;
            let path = from_shell(shell.as_os_str(), Duration::from_millis(300)).await;
            assert_eq!(path, None);
            let job: libc::pid_t = tokio::fs::read_to_string(&job)
                .await?
                .trim()
                .parse()
                .expect("a PID");
            for _ in 0..40 {
                // SAFETY: signal 0 only checks that `job` exists.
                if unsafe { libc::kill(job, 0) } != 0 {
                    return Ok(false);
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            Ok(true)
        }

        #[tokio::test]
        async fn a_shell_that_hangs_is_given_up_on_with_its_job() -> std::io::Result<()> {
            assert!(!job_outlives_a_shell_that("wait").await?);
            Ok(())
        }

        #[tokio::test]
        async fn a_job_left_by_a_shell_that_exited_without_a_path_is_killed() -> std::io::Result<()>
        {
            assert!(!job_outlives_a_shell_that("exit 0").await?);
            Ok(())
        }
    }
}
