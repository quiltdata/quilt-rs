//! `quilt upgrade`: installs the latest quilt-cli with cargo.

use std::path::Path;
use std::path::PathBuf;
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
    /// Where cargo put the binary, if known.
    bin_dir: Option<PathBuf>,
    /// The folder of the `quilt` that ran, when it is not `bin_dir`.
    elsewhere: Option<PathBuf>,
}

impl std::fmt::Display for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Installed the latest quilt-cli with \"{}\"",
            self.installer
        )?;
        if let (Some(bin_dir), Some(running)) = (&self.bin_dir, &self.elsewhere) {
            write!(
                f,
                "\nIt is in {}, but this quilt runs from {}; PATH may still find the old one first",
                bin_dir.display(),
                running.display()
            )?;
        }
        Ok(())
    }
}

impl Render for Output {
    fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "installer": self.installer.name(),
            "bin_dir": self.bin_dir.as_ref().map(|dir| dir.to_string_lossy()),
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
    let bin_dir = cargo_bin_dir(
        std::env::var_os("CARGO_INSTALL_ROOT").map(PathBuf::from),
        std::env::var_os("CARGO_HOME").map(PathBuf::from),
        dirs::home_dir(),
    );
    let running = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf));
    let elsewhere = match (&bin_dir, running) {
        (Some(bin_dir), Some(running)) if !same_dir(bin_dir, &running) => Some(running),
        _ => None,
    };
    Ok(Output {
        installer,
        bin_dir,
        elsewhere,
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

/// Where cargo installs binaries, by its own precedence.
fn cargo_bin_dir(
    install_root: Option<PathBuf>,
    cargo_home: Option<PathBuf>,
    home: Option<PathBuf>,
) -> Option<PathBuf> {
    install_root
        .or(cargo_home)
        .or_else(|| home.map(|home| home.join(".cargo")))
        .map(|root| root.join("bin"))
}

fn same_dir(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
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
    fn the_bin_dir_follows_cargo() {
        let p = |s: &str| Some(PathBuf::from(s));
        assert_eq!(
            cargo_bin_dir(p("/root"), p("/cargo"), p("/home/u")),
            p("/root/bin")
        );
        assert_eq!(
            cargo_bin_dir(None, p("/cargo"), p("/home/u")),
            p("/cargo/bin")
        );
        assert_eq!(
            cargo_bin_dir(None, None, p("/home/u")),
            p("/home/u/.cargo/bin")
        );
        assert_eq!(cargo_bin_dir(None, None, None), None);
    }

    #[test]
    fn a_quilt_outside_the_bin_dir_is_named() {
        let output = Output {
            installer: Installer::Binstall,
            bin_dir: Some(PathBuf::from("/home/u/.cargo/bin")),
            elsewhere: Some(PathBuf::from("/usr/local/bin")),
        };
        assert_eq!(
            output.to_string(),
            "Installed the latest quilt-cli with \"cargo binstall --no-confirm quilt-cli\"\n\
             It is in /home/u/.cargo/bin, but this quilt runs from /usr/local/bin; PATH may still find the old one first"
        );
        assert_eq!(
            output.to_json(),
            serde_json::json!({"installer": "cargo-binstall", "bin_dir": "/home/u/.cargo/bin"})
        );
    }
}
