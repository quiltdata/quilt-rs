//! Where quiltx keeps its credential files, and the key an entry has in the OS
//! keyring.
//!
//! The directory is `platformdirs.user_data_path("quiltx")` as Python computes
//! it, re-derived rule by rule from platformdirs 4.14 (what a fresh install gets on
//! Python 3.11+; 3.10 gets 4.12, which differs only for a relative `HOME`):
//!
//! * Linux and other Unix: `$XDG_DATA_HOME` if it is an absolute path, else
//!   `~/.local/share`; `quiltx` below it.
//! * macOS: the same `$XDG_DATA_HOME` rule (platformdirs honors it since 4.6),
//!   else `~/Library/Application Support`; `quiltx` below it.
//! * Windows: the `Local AppData` folder, then `quiltx\quiltx` (`platformdirs` puts
//!   the app name in for the missing author).
//!
//! `~` is `$HOME` when that is an absolute path, otherwise the passwd entry's
//! home (a relative or empty `HOME` counts as unset); if neither gives an
//! absolute path the lookup fails with platformdirs' own `RuntimeError`
//! message. Python then passes the string through `pathlib.Path`, which
//! collapses repeated slashes, drops `.` parts and a trailing slash. The logic
//! works on bytes and takes its inputs as values, so every platform's branch is
//! unit-tested on every OS.
//!
//! Older platformdirs differed: before 4.6 macOS ignored `$XDG_DATA_HOME`;
//! before 4.12 a relative one was used as is and an empty `HOME` was used
//! verbatim; before 4.14 so was a relative `HOME`. quiltx does not pin
//! platformdirs, so the Python it replaced gave different answers on those
//! inputs depending on what was installed. This follows the current rules.
//!
//! Deliberate difference: platformdirs on Windows downgrades a profile path
//! that has non-Latin-1 characters to its 8.3 short form; this returns the
//! long form. It is the same directory.

use std::ffi::OsString;
use std::path::PathBuf;

pub const APP_NAME: &str = "quiltx";
pub const KEYRING_SERVICE: &str = "quiltx";
pub const CREDENTIALS_FILE: &str = "credentials.json";
pub const INDEX_FILE: &str = "credentials_index.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    /// Linux and every other non-macOS Unix.
    Unix,
    MacOs,
    Windows,
}

impl Platform {
    #[must_use]
    pub fn current() -> Self {
        if cfg!(windows) {
            Platform::Windows
        } else if cfg!(target_os = "macos") {
            Platform::MacOs
        } else {
            Platform::Unix
        }
    }
}

/// Everything the lookup reads from the machine.
#[derive(Debug, Clone, Default)]
pub struct Inputs {
    pub xdg_data_home: Option<OsString>,
    pub home: Option<OsString>,
    /// Home directory from the passwd database; read only when `home` is unset.
    pub passwd_home: Option<OsString>,
    /// Windows `Local AppData` folder.
    pub local_app_data: Option<PathBuf>,
}

impl Inputs {
    #[must_use]
    pub fn from_env() -> Self {
        let home = std::env::var_os("HOME");
        let home_usable = home.as_ref().is_some_and(|h| to_bytes(h).starts_with(b"/"));
        let passwd_home = if !home_usable && !cfg!(windows) {
            passwd_home()
        } else {
            None
        };
        Inputs {
            xdg_data_home: std::env::var_os("XDG_DATA_HOME"),
            home,
            passwd_home,
            local_app_data: local_app_data(),
        }
    }
}

#[cfg(windows)]
fn local_app_data() -> Option<PathBuf> {
    dirs::data_local_dir()
}

#[cfg(not(windows))]
fn local_app_data() -> Option<PathBuf> {
    None
}

/// The home directory in the passwd database, like `pwd.getpwuid(os.getuid()).pw_dir`.
#[cfg(unix)]
fn passwd_home() -> Option<OsString> {
    use std::os::unix::ffi::OsStringExt;

    let mut buf = vec![0u8; 1024];
    loop {
        // SAFETY: `passwd` is plain data, `buf` outlives the call, and on success
        // `result` points at `pwd`, whose strings live in `buf`.
        let (rc, result, pwd) = unsafe {
            let mut pwd: libc::passwd = std::mem::zeroed();
            let mut result: *mut libc::passwd = std::ptr::null_mut();
            let rc = libc::getpwuid_r(
                libc::getuid(),
                &raw mut pwd,
                buf.as_mut_ptr().cast(),
                buf.len(),
                &raw mut result,
            );
            (rc, result, pwd)
        };
        if rc == libc::ERANGE && buf.len() < (1 << 20) {
            buf.resize(buf.len() * 2, 0);
            continue;
        }
        if rc != 0 || result.is_null() || pwd.pw_dir.is_null() {
            return None;
        }
        // SAFETY: `pw_dir` is a NUL-terminated string inside `buf`.
        let dir = unsafe { std::ffi::CStr::from_ptr(pwd.pw_dir) };
        return Some(OsString::from_vec(dir.to_bytes().to_vec()));
    }
}

#[cfg(not(unix))]
fn passwd_home() -> Option<OsString> {
    None
}

#[cfg(unix)]
fn to_bytes(s: &OsString) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    s.as_os_str().as_bytes().to_vec()
}

#[cfg(not(unix))]
fn to_bytes(s: &OsString) -> Vec<u8> {
    s.to_string_lossy().into_owned().into_bytes()
}

#[cfg(unix)]
fn from_bytes(b: Vec<u8>) -> PathBuf {
    use std::os::unix::ffi::OsStringExt;
    PathBuf::from(OsString::from_vec(b))
}

#[cfg(not(unix))]
#[allow(
    clippy::needless_pass_by_value,
    reason = "only the Unix variant consumes the bytes; this branch is Windows and wasm32, and both take the same type"
)]
fn from_bytes(b: Vec<u8>) -> PathBuf {
    PathBuf::from(String::from_utf8_lossy(&b).into_owned())
}

/// Why no directory could be found; the message is platformdirs' `RuntimeError`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathError(pub String);

/// platformdirs' `_expand_user("~" + rest)`, with `rest` starting with `/`.
fn expanduser(inputs: &Inputs, rest: &str) -> Result<Vec<u8>, PathError> {
    let home_is_usable = inputs
        .home
        .as_ref()
        .is_some_and(|h| to_bytes(h).starts_with(b"/"));
    let userhome = if home_is_usable {
        inputs.home.as_ref()
    } else {
        // HOME unset, empty or relative: the passwd entry, if there is one.
        inputs.passwd_home.as_ref()
    };
    let expanded = match userhome {
        Some(home) => {
            let home = to_bytes(home);
            let end = home.iter().rposition(|&b| b != b'/').map_or(0, |i| i + 1);
            let mut out = home[..end].to_vec();
            out.extend_from_slice(rest.as_bytes());
            out
        }
        None => [b"~", rest.as_bytes()].concat(),
    };
    if expanded.starts_with(b"/") {
        Ok(expanded)
    } else {
        Err(PathError(format!(
            "could not determine the home directory for '~{rest}', set an absolute HOME or XDG variable"
        )))
    }
}

/// `posixpath.join(base, name)` for a relative `name`.
fn join_posix(base: &[u8], name: &str) -> Vec<u8> {
    let mut out = base.to_vec();
    if !out.is_empty() && !out.ends_with(b"/") {
        out.push(b'/');
    }
    out.extend_from_slice(name.as_bytes());
    out
}

/// `str(pathlib.PurePosixPath(p))`.
fn pure_posix(p: &[u8]) -> Vec<u8> {
    let slashes = p.iter().take_while(|&&b| b == b'/').count();
    let root: &[u8] = match slashes {
        0 => b"",
        2 => b"//",
        _ => b"/",
    };
    let parts: Vec<&[u8]> = p
        .split(|&b| b == b'/')
        .filter(|part| !part.is_empty() && *part != b".")
        .collect();
    let mut out = root.to_vec();
    out.extend_from_slice(&parts.join(&b'/'));
    if out.is_empty() {
        out.push(b'.');
    }
    out
}

/// `platformdirs.user_data_path("quiltx")`.
///
/// # Errors
///
/// A [`PathError`] when no absolute home directory can be found.
pub fn user_data_dir(platform: Platform, inputs: &Inputs) -> Result<PathBuf, PathError> {
    let default_base = match platform {
        Platform::Windows => {
            let base = inputs.local_app_data.clone().unwrap_or_default();
            return Ok(base.join(APP_NAME).join(APP_NAME));
        }
        Platform::MacOs => "/Library/Application Support",
        Platform::Unix => "/.local/share",
    };
    let base = match &inputs.xdg_data_home {
        // platformdirs' `_xdg_dir`: only an absolute path counts.
        Some(xdg) if to_bytes(xdg).starts_with(b"/") => to_bytes(xdg),
        _ => expanduser(inputs, default_base)?,
    };
    Ok(from_bytes(pure_posix(&join_posix(&base, APP_NAME))))
}

/// The file-based credential fallback, `credentials.json`.
///
/// # Errors
///
/// A [`PathError`] when no absolute home directory can be found.
pub fn credentials_file(platform: Platform, inputs: &Inputs) -> Result<PathBuf, PathError> {
    Ok(user_data_dir(platform, inputs)?.join(CREDENTIALS_FILE))
}

/// The keyring index, `credentials_index.json`.
///
/// # Errors
///
/// A [`PathError`] when no absolute home directory can be found.
pub fn index_file(platform: Platform, inputs: &Inputs) -> Result<PathBuf, PathError> {
    Ok(user_data_dir(platform, inputs)?.join(INDEX_FILE))
}

/// The OS-keyring entry for a catalog: `(service, username)`. The username is
/// the DNS exactly as given; normalizing is the caller's job
/// ([`crate::dns::normalize_dns`]), as it always was.
#[must_use]
pub fn keyring_entry(dns: &str) -> (&'static str, String) {
    (KEYRING_SERVICE, dns.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs(xdg: Option<&str>, home: Option<&str>, passwd: Option<&str>) -> Inputs {
        Inputs {
            xdg_data_home: xdg.map(OsString::from),
            home: home.map(OsString::from),
            passwd_home: passwd.map(OsString::from),
            local_app_data: None,
        }
    }

    fn dir(platform: Platform, i: &Inputs) -> String {
        user_data_dir(platform, i)
            .unwrap()
            .to_string_lossy()
            .into_owned()
    }

    fn unix(xdg: Option<&str>, home: Option<&str>, passwd: Option<&str>) -> String {
        dir(Platform::Unix, &inputs(xdg, home, passwd))
    }

    #[test]
    fn an_absolute_xdg_data_home_wins() {
        assert_eq!(unix(Some("/data"), Some("/home/u"), None), "/data/quiltx");
        assert_eq!(unix(Some("/data/"), Some("/home/u"), None), "/data/quiltx");
        assert_eq!(unix(Some("/a//b/./c/"), None, None), "/a/b/c/quiltx");
        assert_eq!(unix(Some("//two"), None, None), "//two/quiltx");
        assert_eq!(unix(Some("///three"), None, None), "/three/quiltx");
        assert_eq!(unix(Some("/a/../b"), None, None), "/a/../b/quiltx");
        assert_eq!(unix(Some("/"), None, None), "/quiltx");
    }

    #[test]
    fn a_relative_blank_or_unset_xdg_data_home_is_ignored() {
        let want = "/home/u/.local/share/quiltx";
        for xdg in [
            None,
            Some(""),
            Some("   "),
            Some("rel"),
            Some("."),
            Some("\t"),
        ] {
            assert_eq!(unix(xdg, Some("/home/u"), None), want, "{xdg:?}");
        }
    }

    #[test]
    fn home_is_taken_without_trailing_slashes() {
        let want = "/home/u/.local/share/quiltx";
        assert_eq!(unix(None, Some("/home/u/"), None), want);
        assert_eq!(unix(None, Some("/home/u///"), None), want);
        assert_eq!(unix(None, Some("/"), None), "/.local/share/quiltx");
    }

    #[test]
    fn an_empty_or_relative_home_counts_as_unset() {
        let want = "/pw/.local/share/quiltx";
        assert_eq!(unix(None, Some(""), Some("/pw")), want);
        assert_eq!(unix(None, Some("rel"), Some("/pw/")), want);
        assert_eq!(unix(None, None, Some("/pw")), want);
        assert_eq!(unix(None, None, Some("")), "/.local/share/quiltx");
    }

    #[test]
    fn no_absolute_home_is_platformdirs_runtime_error() {
        let want = PathError(
            "could not determine the home directory for '~/.local/share', set an absolute HOME or XDG variable"
                .into(),
        );
        for i in [
            inputs(None, None, None),
            inputs(None, Some(""), None),
            inputs(None, Some("rel"), None),
            inputs(Some("rel"), None, Some("rel")),
        ] {
            assert_eq!(user_data_dir(Platform::Unix, &i).unwrap_err(), want);
        }
        let mac = user_data_dir(Platform::MacOs, &inputs(None, None, None)).unwrap_err();
        assert!(mac.0.contains("'~/Library/Application Support'"), "{mac:?}");
    }

    #[test]
    fn macos_defaults_to_application_support_and_honors_an_absolute_xdg() {
        let i = inputs(None, Some("/Users/u/"), None);
        assert_eq!(
            dir(Platform::MacOs, &i),
            "/Users/u/Library/Application Support/quiltx"
        );
        let xdg = inputs(Some("/data"), Some("/Users/u"), None);
        assert_eq!(dir(Platform::MacOs, &xdg), "/data/quiltx");
        let rel = inputs(Some("rel"), Some("/Users/u"), None);
        assert_eq!(
            dir(Platform::MacOs, &rel),
            "/Users/u/Library/Application Support/quiltx"
        );
    }

    #[test]
    fn windows_nests_the_app_name_twice() {
        let i = Inputs {
            local_app_data: Some(PathBuf::from("base")),
            ..Inputs::default()
        };
        assert_eq!(
            user_data_dir(Platform::Windows, &i).unwrap(),
            PathBuf::from("base").join("quiltx").join("quiltx")
        );
    }

    #[test]
    fn file_names_sit_in_that_directory() {
        let i = inputs(Some("/d"), None, None);
        assert_eq!(
            credentials_file(Platform::Unix, &i).unwrap(),
            PathBuf::from("/d/quiltx/credentials.json")
        );
        assert_eq!(
            index_file(Platform::Unix, &i).unwrap(),
            PathBuf::from("/d/quiltx/credentials_index.json")
        );
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_bytes_survive() {
        use std::os::unix::ffi::OsStringExt;
        let i = Inputs {
            xdg_data_home: Some(OsString::from_vec(b"/d\xff".to_vec())),
            ..Inputs::default()
        };
        let got = user_data_dir(Platform::Unix, &i).unwrap();
        assert_eq!(to_bytes(&got.into_os_string()), b"/d\xff/quiltx");
    }

    #[test]
    fn keyring_entry_is_service_and_the_dns_verbatim() {
        assert_eq!(
            keyring_entry("Nightly.QuiltTest.com"),
            ("quiltx", "Nightly.QuiltTest.com".to_string())
        );
        assert_eq!(keyring_entry(""), ("quiltx", String::new()));
    }

    #[cfg(unix)]
    #[test]
    fn the_passwd_lookup_finds_this_user() {
        let home = passwd_home().expect("the test user has a passwd entry");
        assert!(to_bytes(&home).starts_with(b"/"), "{home:?}");
    }

    #[test]
    fn from_env_finds_a_directory_on_this_machine() {
        let i = Inputs::from_env();
        let dir = user_data_dir(Platform::current(), &i).unwrap();
        assert!(dir.ends_with("quiltx"), "{dir:?}");
    }
}
