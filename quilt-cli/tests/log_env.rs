//! A `QUILT_LOG` that does not parse stops `quilt` before it runs anything,
//! run on the real binary so the test process's environment is never changed.

mod common;

use std::process::Output;

use common::QUILT_VARS;
use common::quilt_command;
use common::removes;

fn quilt_with_log(value: &str, args: &[&str]) -> Output {
    let dir = tempfile::tempdir().expect("a temporary folder");
    quilt_command(&dir.path().join("domain"))
        .args(args)
        .env("QUILT_LOG", value)
        .output()
        .expect("quilt runs")
}

#[test]
fn a_typo_in_the_level_fails_before_the_command_runs() {
    let output = quilt_with_log("debgu", &["list"]);

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty(), "{:?}", output.stdout);
    assert_eq!(
        String::from_utf8_lossy(&output.stderr).trim(),
        "QUILT_LOG=\"debgu\" is not a log level or a list of directives"
    );
}

#[test]
fn under_json_the_failure_is_the_error_object() {
    let output = quilt_with_log("quilt_rs=lots", &["list", "--json", "-v"]);

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty(), "{:?}", output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let last = stderr.lines().last().expect("an error line");
    let error: serde_json::Value = serde_json::from_str(last).expect("a JSON object");
    assert_eq!(error["error"]["kind"], "invalid_log_filter");
    assert_eq!(
        error["error"]["message"],
        "QUILT_LOG=\"quilt_rs=lots\" is not a log level or a list of directives"
    );
}

#[test]
fn a_valid_level_runs_the_command() {
    let output = quilt_with_log("debug", &["list"]);

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// The guard for every test that runs the binary: none of the caller's
/// `QUILT_*` variables reaches it, so a developer's `QUILT_FORMAT=json` or
/// `QUILT_DOMAIN` cannot change what a test sees.
#[test]
fn the_child_sees_none_of_the_callers_quilt_variables() {
    let dir = tempfile::tempdir().expect("a temporary folder");
    let command = quilt_command(&dir.path().join("domain"));

    for name in QUILT_VARS {
        assert!(removes(&command, name), "{name}");
    }
    for (name, _) in std::env::vars() {
        if name.starts_with("QUILT_") {
            assert!(removes(&command, &name), "{name}");
        }
    }
}

/// A command that picks a default home must pick it inside the temporary
/// folder, never under the developer's own home. Unix only: Windows finds
/// the home through a system call, not `HOME`.
#[cfg(unix)]
#[test]
fn a_default_home_lands_in_the_temporary_folder() {
    let dir = tempfile::tempdir().expect("a temporary folder");
    let domain = dir.path().join("domain");
    let list = quilt_command(&domain)
        .arg("list")
        .output()
        .expect("quilt runs");
    assert!(
        list.status.success(),
        "{}",
        String::from_utf8_lossy(&list.stderr)
    );

    let home = quilt_command(&domain)
        .args(["home", "--json"])
        .output()
        .expect("quilt runs");
    let stdout = String::from_utf8_lossy(&home.stdout);
    assert!(
        stdout.contains(&*dir.path().join("user-home").to_string_lossy()),
        "{stdout}"
    );
}
