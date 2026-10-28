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
    use std::time::Duration;

    use tokio::io::AsyncReadExt;
    use tokio::sync::OnceCell;

    use crate::telemetry::prelude::*;

    /// Brackets the PATH in the shell's output, which may also carry whatever
    /// the user's startup files print.
    const MARKER: &str = "__QUILTSYNC_PATH__";

    /// A slow startup file costs a run this long, at most.
    const TIMEOUT: Duration = Duration::from_secs(5);

    /// The login shell's PATH, kept once found; `None` keeps `QuiltSync`'s own,
    /// and the next run asks again. A tool installed later shows up after a
    /// restart.
    pub async fn login_path() -> Option<&'static str> {
        static PATH: OnceCell<String> = OnceCell::const_new();
        let found = PATH
            .get_or_try_init(async || from_shell(&shell(), TIMEOUT).await.ok_or(()))
            .await;
        if found.is_err() {
            warn!("tasks: no PATH from the login shell, the agent gets QuiltSync's own");
        }
        found.ok().map(String::as_str)
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
        if path.is_none()
            && matches!(child.try_wait(), Ok(None))
            && let Some(group) = child.id().and_then(|id| libc::pid_t::try_from(id).ok())
        {
            // SAFETY: `kill` takes plain integers and touches no memory of ours.
            // The shell is unreaped, so its PID still names its group.
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

        /// A shell that never prints the PATH is given up on, and what it
        /// started is killed with it.
        #[tokio::test]
        async fn a_shell_that_hangs_is_given_up_on() -> std::io::Result<()> {
            let dir = tempfile::TempDir::new()?;
            let shell = dir.path().join("shell");
            tokio::fs::write(&shell, "#!/bin/sh\nsleep 30\n").await?;
            tokio::fs::set_permissions(&shell, std::os::unix::fs::PermissionsExt::from_mode(0o755))
                .await?;
            let path = from_shell(shell.as_os_str(), Duration::from_millis(300)).await;
            assert_eq!(path, None);
            Ok(())
        }
    }
}
