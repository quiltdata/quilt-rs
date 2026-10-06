//! A `QUILT_LOG` that does not parse stops `quilt` before it runs anything,
//! run on the real binary so the test process's environment is never changed.

use std::process::Command;
use std::process::Output;

fn quilt_with_log(value: &str, args: &[&str]) -> Output {
    let domain = tempfile::tempdir().expect("a temp domain");
    Command::new(env!("CARGO_BIN_EXE_quilt"))
        .arg("--domain")
        .arg(domain.path())
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
