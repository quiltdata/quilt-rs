//! `quilt upgrade`: installs the latest quilt-cli with cargo.

use std::io::BufRead;
use std::io::IsTerminal;
use std::process::Command;
use std::process::Stdio;

use crate::cli::Error;
use crate::cli::output::Render;
use crate::cli::output::Std;

/// How cargo installs the release.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Installer {
    Binstall,
    Install,
}

impl Installer {
    /// cargo's arguments. binstall's prompt is skipped, since typing
    /// `quilt upgrade` was the confirmation, and so is its build from source,
    /// which is asked for separately.
    fn args(self) -> &'static [&'static str] {
        match self {
            Installer::Binstall => &[
                "binstall",
                "--no-confirm",
                "--disable-strategies",
                "compile",
                "quilt-cli",
            ],
            Installer::Install => &["install", "--locked", "quilt-cli"],
        }
    }

    fn name(self) -> &'static str {
        match self {
            Installer::Binstall => "cargo-binstall",
            Installer::Install => "cargo-install",
        }
    }
}

impl std::fmt::Display for Installer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "cargo {}", self.args().join(" "))
    }
}

pub struct Output {
    installer: Installer,
    /// What `quilt --version`, found through PATH, reports after the
    /// install: the copy the next command runs, wherever cargo put the new one.
    path_version: Option<String>,
}

impl std::fmt::Display for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Installed the latest quilt-cli with \"{}\"",
            self.installer
        )?;
        if let Some(version) = &self.path_version {
            write!(f, "\nThe quilt on your PATH reports {version}")?;
        }
        Ok(())
    }
}

impl Render for Output {
    fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "installer": self.installer.name(),
            "path_version": self.path_version,
        })
    }
}

pub fn command() -> Std {
    Std::from_result(upgrade())
}

fn upgrade() -> Result<Output, Error> {
    let probe = Command::new("cargo")
        .args(["binstall", "-V"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success());
    let has_binstall = has_binstall(probe)?;
    if cfg!(windows) {
        let installer = if has_binstall {
            Installer::Binstall
        } else {
            Installer::Install
        };
        return Err(Error::UpgradeUnsupported(installer));
    }
    let installer = if has_binstall && run(Installer::Binstall)? {
        Installer::Binstall
    } else {
        let why = if has_binstall {
            "No prebuilt quilt-cli could be installed."
        } else {
            "cargo-binstall is not installed, so no prebuilt quilt-cli can be installed."
        };
        if !confirm_build(why)? {
            return Err(Error::NotBuilt(Installer::Install));
        }
        if !run(Installer::Install)? {
            return Err(Error::UpgradeFailed(Installer::Install));
        }
        Installer::Install
    };
    let path_version = Command::new("quilt")
        .arg("--version")
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| version(&String::from_utf8_lossy(&output.stdout)));
    Ok(Output {
        installer,
        path_version,
    })
}

/// Runs cargo with its output on stderr, so stdout keeps only the result.
fn run(installer: Installer) -> Result<bool, Error> {
    let status = Command::new("cargo")
        .args(installer.args())
        .stdout(std::io::stderr())
        .status()?;
    Ok(status.success())
}

/// Asks before building from source; without a terminal there is no one to
/// ask, and the answer is no.
fn confirm_build(why: &str) -> Result<bool, Error> {
    if !std::io::stdin().is_terminal() {
        return Ok(false);
    }
    eprint!(
        "{why} Build it from source with \"{}\"? This takes a few minutes. [y/N] ",
        Installer::Install
    );
    let mut answer = String::new();
    std::io::stdin().lock().read_line(&mut answer)?;
    Ok(is_yes(&answer))
}

fn is_yes(answer: &str) -> bool {
    matches!(answer.trim().to_lowercase().as_str(), "y" | "yes")
}

/// Whether `cargo binstall -V` worked; no cargo at all is its own error.
fn has_binstall(probe: std::io::Result<bool>) -> Result<bool, Error> {
    match probe {
        Ok(has) => Ok(has),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Err(Error::CargoMissing),
        Err(err) => Err(err.into()),
    }
}

/// The version in `quilt --version`'s first line, `quilt-cli 0.41.0`.
fn version(stdout: &str) -> Option<String> {
    stdout
        .lines()
        .next()?
        .split_whitespace()
        .last()
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_cargo_is_its_own_error() {
        assert!(has_binstall(Ok(true)).unwrap());
        assert!(!has_binstall(Ok(false)).unwrap());
        let missing = std::io::Error::from(std::io::ErrorKind::NotFound);
        assert!(matches!(
            has_binstall(Err(missing)),
            Err(Error::CargoMissing)
        ));
        let denied = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        assert!(matches!(has_binstall(Err(denied)), Err(Error::Io(_))));
    }

    #[test]
    fn only_yes_builds() {
        for yes in ["y\n", "Y", " yes \n", "YES"] {
            assert!(is_yes(yes), "{yes:?}");
        }
        for no in ["", "\n", "n", "no", "yep", "sure"] {
            assert!(!is_yes(no), "{no:?}");
        }
    }

    #[test]
    fn the_commands_install_quilt_cli() {
        assert_eq!(
            Installer::Binstall.to_string(),
            "cargo binstall --no-confirm --disable-strategies compile quilt-cli"
        );
        assert_eq!(
            Installer::Install.to_string(),
            "cargo install --locked quilt-cli"
        );
    }

    #[test]
    fn the_version_is_read_from_quilt_version() {
        assert_eq!(version("quilt-cli 0.41.0\n").as_deref(), Some("0.41.0"));
        assert_eq!(version(""), None);
    }

    #[test]
    fn the_result_names_what_path_runs() {
        let output = Output {
            installer: Installer::Binstall,
            path_version: Some("0.41.0".into()),
        };
        assert_eq!(
            output.to_string(),
            "Installed the latest quilt-cli with \"cargo binstall --no-confirm --disable-strategies compile quilt-cli\"\n\
             The quilt on your PATH reports 0.41.0"
        );
        assert_eq!(
            output.to_json(),
            serde_json::json!({"installer": "cargo-binstall", "path_version": "0.41.0"})
        );
    }
}
