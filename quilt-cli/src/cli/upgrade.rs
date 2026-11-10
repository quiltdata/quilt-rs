//! `quilt upgrade`: installs the latest quilt-cli with cargo.

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
    /// cargo's arguments. binstall's prompt is skipped: typing `quilt upgrade`
    /// was the confirmation.
    fn args(self) -> &'static [&'static str] {
        match self {
            Installer::Binstall => &["binstall", "--no-confirm", "quilt-cli"],
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
    let installer = choose(probe)?;
    if cfg!(windows) {
        return Err(Error::UpgradeUnsupported(installer));
    }
    if installer == Installer::Install {
        eprintln!(
            "cargo-binstall is not installed, so cargo builds quilt-cli from source; this takes a few minutes"
        );
    }
    let status = Command::new("cargo")
        .args(installer.args())
        .stdout(std::io::stderr())
        .status()?;
    if !status.success() {
        return Err(Error::UpgradeFailed(installer));
    }
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

/// binstall when `cargo binstall -V` worked; cargo install when cargo is
/// there without it.
fn choose(probe: std::io::Result<bool>) -> Result<Installer, Error> {
    match probe {
        Ok(true) => Ok(Installer::Binstall),
        Ok(false) => Ok(Installer::Install),
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
    fn binstall_is_used_when_cargo_has_it() {
        assert_eq!(choose(Ok(true)).unwrap(), Installer::Binstall);
        assert_eq!(choose(Ok(false)).unwrap(), Installer::Install);
    }

    #[test]
    fn no_cargo_is_its_own_error() {
        let missing = std::io::Error::from(std::io::ErrorKind::NotFound);
        assert!(matches!(choose(Err(missing)), Err(Error::CargoMissing)));
        let denied = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        assert!(matches!(choose(Err(denied)), Err(Error::Io(_))));
    }

    #[test]
    fn the_commands_install_quilt_cli() {
        assert_eq!(
            Installer::Binstall.to_string(),
            "cargo binstall --no-confirm quilt-cli"
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
            "Installed the latest quilt-cli with \"cargo binstall --no-confirm quilt-cli\"\n\
             The quilt on your PATH reports 0.41.0"
        );
        assert_eq!(
            output.to_json(),
            serde_json::json!({"installer": "cargo-binstall", "path_version": "0.41.0"})
        );
    }
}
