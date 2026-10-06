//! Running the real `quilt` binary from a test without touching anything of
//! the developer's: their `QUILT_*` variables, their domain, or their home.

use std::path::Path;
use std::process::Command;

/// The variables `quilt` reads. Removed by name even when unset here, so the
/// list is checked; any other `QUILT_*` the caller has set is removed too.
pub const QUILT_VARS: [&str; 3] = ["QUILT_DOMAIN", "QUILT_FORMAT", "QUILT_LOG"];

/// A `quilt` command on `domain`, which must be inside the temporary folder.
///
/// The child sees none of the caller's `QUILT_*` variables, and its `HOME`
/// is `user-home` next to `domain`, so a default home it creates lands in
/// the same temporary folder rather than in the developer's home (on Unix,
/// where the home comes from `HOME`).
pub fn quilt_command(domain: &Path) -> Command {
    assert!(
        domain.starts_with(std::env::temp_dir()),
        "{} is not a temporary folder",
        domain.display()
    );
    let user_home = domain
        .parent()
        .expect("the domain sits in a temporary folder")
        .join("user-home");

    let mut command = Command::new(env!("CARGO_BIN_EXE_quilt"));
    for name in QUILT_VARS {
        command.env_remove(name);
    }
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("QUILT_") {
            command.env_remove(name);
        }
    }
    command.env("HOME", user_home).arg("--domain").arg(domain);
    command
}

/// Whether `command` removes `name` from the child's environment.
#[allow(dead_code, reason = "used by one test crate, not the other")]
pub fn removes(command: &Command, name: &str) -> bool {
    command
        .get_envs()
        .any(|(key, value)| key == name && value.is_none())
}
