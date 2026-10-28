//! The PATH of the user's login shell, for the agents `QuiltSync` starts.
//!
//! Started by launchd, `QuiltSync` gets `/usr/bin:/bin:/usr/sbin:/sbin`. An agent
//! given that PATH can't find the user's tools (`gh`, Homebrew's `git`, `deno`)
//! and may search the home folder for them, which raises macOS's privacy
//! prompts in `QuiltSync`'s name.

#[cfg(unix)]
pub use unix::login_path;

/// Windows apps get the user's PATH as it is.
#[cfg(not(unix))]
#[allow(clippy::unused_async)]
pub async fn login_path() -> Option<&'static str> {
    None
}

#[cfg(unix)]
mod unix {
    use std::ffi::OsStr;
    use std::ffi::OsString;
    use std::process::Stdio;
    use std::time::Duration;

    use tokio::sync::OnceCell;

    use crate::telemetry::prelude::*;

    /// Brackets the PATH in the shell's output, which may also carry whatever
    /// the user's startup files print.
    const MARKER: &str = "__QUILTSYNC_PATH__";

    /// A slow startup file costs the first run this long, at most.
    const TIMEOUT: Duration = Duration::from_secs(5);

    /// The login shell's PATH, asked for once per process; `None` keeps
    /// `QuiltSync`'s own. A tool installed later shows up after a restart.
    pub async fn login_path() -> Option<&'static str> {
        static PATH: OnceCell<Option<String>> = OnceCell::const_new();
        PATH.get_or_init(async || {
            let path = from_shell(&shell(), TIMEOUT).await;
            if path.is_none() {
                warn!("tasks: no PATH from the login shell, agents get QuiltSync's own");
            }
            path
        })
        .await
        .as_deref()
    }

    fn shell() -> OsString {
        let fallback = if cfg!(target_os = "macos") {
            "/bin/zsh"
        } else {
            "/bin/sh"
        };
        std::env::var_os("SHELL").unwrap_or_else(|| fallback.into())
    }

    /// Interactive as well as login, so a PATH set in `.zshrc` or `.bashrc`
    /// counts too.
    async fn from_shell(shell: &OsStr, timeout: Duration) -> Option<String> {
        let output = tokio::process::Command::new(shell)
            .arg("-ilc")
            .arg(format!(r#"printf '{MARKER}%s{MARKER}' "$PATH""#))
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .output();
        let output = tokio::time::timeout(timeout, output).await.ok()?.ok()?;
        between_markers(&String::from_utf8_lossy(&output.stdout))
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
    }
}
