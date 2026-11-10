//! `quilt upgrade` against fake `cargo` and `quilt` commands on PATH, so the
//! probe-then-install order and the streams are checked without installing.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;
use std::process::Output;

/// Logs each call, has binstall only when `HAS_BINSTALL` is set, and fails an
/// install when `FAIL` is set. Install chatter goes to stdout, as binstall's
/// does.
const CARGO: &str = r#"#!/bin/sh
echo "$*" >> "$LOG"
case "$1" in
  binstall)
    [ -n "$HAS_BINSTALL" ] || { echo "error: no such command: binstall" >&2; exit 101; }
    [ "$2" = "-V" ] && { echo 1.26.2; exit 0; } ;;
esac
echo "cargo $1 is installing"
[ -z "$FAIL" ]
"#;

const QUILT: &str = "#!/bin/sh\necho quilt-cli 9.9.9\n";

fn script(dir: &Path, name: &str, body: &str) {
    let path = dir.join(name);
    std::fs::write(&path, body).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// Runs `quilt --json upgrade` and returns its output and cargo's calls.
fn upgrade(env: &[(&str, &str)]) -> (Output, Vec<String>) {
    let dir = tempfile::TempDir::new().unwrap();
    let bin = dir.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    script(&bin, "cargo", CARGO);
    script(&bin, "quilt", QUILT);
    let log = dir.path().join("cargo.log");
    let output = Command::new(env!("CARGO_BIN_EXE_quilt"))
        .args(["--json", "--domain"])
        .arg(dir.path().join("domain"))
        .arg("upgrade")
        .env_clear()
        .env("PATH", &bin)
        .env("HOME", dir.path())
        .env("LOG", &log)
        .envs(env.iter().copied())
        .output()
        .unwrap();
    let calls = std::fs::read_to_string(&log)
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect();
    (output, calls)
}

fn stdout_json(output: &Output) -> serde_json::Value {
    serde_json::from_slice(&output.stdout).expect("stdout is only the JSON result")
}

#[test]
fn binstall_is_used_when_cargo_has_it() {
    let (output, calls) = upgrade(&[("HAS_BINSTALL", "1")]);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(calls, ["binstall -V", "binstall --no-confirm quilt-cli"]);
    assert_eq!(
        stdout_json(&output),
        serde_json::json!({"installer": "cargo-binstall", "path_version": "9.9.9"})
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("cargo binstall is installing"));
}

#[test]
fn cargo_install_builds_it_without_binstall() {
    let (output, calls) = upgrade(&[]);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(calls, ["binstall -V", "install --locked quilt-cli"]);
    assert_eq!(stdout_json(&output)["installer"], "cargo-install");
}

#[test]
fn a_failed_binstall_starts_no_build() {
    let (output, calls) = upgrade(&[("HAS_BINSTALL", "1"), ("FAIL", "1")]);
    assert!(!output.status.success());
    assert_eq!(calls, ["binstall -V", "binstall --no-confirm quilt-cli"]);
    assert_eq!(String::from_utf8_lossy(&output.stdout), "");
    let stderr = String::from_utf8_lossy(&output.stderr);
    let error: serde_json::Value = serde_json::from_str(stderr.lines().last().unwrap()).unwrap();
    assert_eq!(error["error"]["kind"], "upgrade_failed");
}
