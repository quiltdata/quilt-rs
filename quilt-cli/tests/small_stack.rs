//! `quilt` on a main thread with little stack: a command's stack is the one
//! `main` gives its thread, not the one the OS gives the process.
//!
//! Unix only, where `ulimit -s` sets the main thread's stack of the process a
//! shell starts. Windows fixes it in the executable, at 1 MB, and has no
//! per-process limit to lower. That is the platform these tests stand in for:
//! there, the other tests that run `quilt` fail when a command outgrows it.
#![cfg(unix)]

use std::path::Path;
use std::process::Command;

/// A quarter of the main-thread stack Windows gives, and well under what a
/// debug `quilt` needs before any command starts.
const MAIN_THREAD_STACK_KB: u32 = 256;

/// Runs `quilt` on `domain` with [`MAIN_THREAD_STACK_KB`] of main-thread
/// stack and fails unless it exits 0.
fn quilt(domain: &Path, args: &[&str]) {
    let output = Command::new("sh")
        .arg("-c")
        .arg(format!(r#"ulimit -s {MAIN_THREAD_STACK_KB} && exec "$@""#))
        .arg("sh")
        .arg(env!("CARGO_BIN_EXE_quilt"))
        // The ref path's commands, which the switch off keeps.
        .env("QUILT_DATASPACES", "0")
        .arg("--domain")
        .arg(domain)
        .args(args)
        .output()
        .expect("sh runs");
    assert!(
        output.status.success(),
        "quilt {args:?} with {MAIN_THREAD_STACK_KB} KB of main-thread stack: {}\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
}

/// `commit` is the deepest of the commands that need no network: on the main
/// thread, a debug build overflowed below 976 KB, and every command below
/// 624 KB.
#[test]
fn commands_run_on_a_small_main_thread_stack() {
    let dir = tempfile::TempDir::new().expect("a temporary folder");
    let domain = dir.path().join("domain");
    let home = dir.path().join("home");

    quilt(&domain, &["version"]);
    quilt(&domain, &["home", home.to_str().expect("a UTF-8 path")]);
    quilt(&domain, &["create", "--namespace", "test/pkg"]);
    let package = home.join("test/pkg");
    std::fs::create_dir_all(&package).expect("the package's folder");
    std::fs::write(package.join("data.txt"), "data").expect("a file in the package");
    quilt(&domain, &["status", "--namespace", "test/pkg"]);
    quilt(&domain, &["commit", "-m", "one", "--namespace", "test/pkg"]);
    quilt(&domain, &["log", "--namespace", "test/pkg"]);
}
