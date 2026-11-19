//! `quilt catalog` and the default catalog `login` and `role` fall back to,
//! through the binary, with the shared folder and quilt3's data directory
//! pointed at a temporary one.
#![cfg(unix)]

use std::path::Path;
use std::process::Command;

use tempfile::TempDir;

fn quilt(home: &Path, args: &[&str]) -> (bool, serde_json::Value) {
    let output = Command::new(env!("CARGO_BIN_EXE_quilt"))
        .args(["--json", "--domain"])
        .arg(home.join("domain"))
        .args(args)
        .env_clear()
        .env("HOME", home)
        .env("XDG_DATA_HOME", home.join("xdg"))
        .env("QUILT_AUTH_DIR", home.join("auth"))
        .output()
        .unwrap();
    let stream = if output.status.success() {
        output.stdout
    } else {
        output.stderr
    };
    let last = String::from_utf8(stream)
        .unwrap()
        .lines()
        .last()
        .unwrap_or_default()
        .to_string();
    (
        output.status.success(),
        serde_json::from_str(&last).unwrap(),
    )
}

#[test]
fn the_default_is_set_shown_and_cleared() {
    let home = TempDir::new().unwrap();

    let (ok, out) = quilt(home.path(), &["catalog", "set", "Open.Quiltdata.com"]);
    assert!(ok);
    assert_eq!(out["default_catalog"], "open.quiltdata.com");

    let (_, out) = quilt(home.path(), &["catalog"]);
    assert_eq!(out["default_catalog"], "open.quiltdata.com");
    let stored = std::fs::read_to_string(home.path().join("auth/settings.json")).unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&stored).unwrap(),
        serde_json::json!({ "default_catalog": "open.quiltdata.com" })
    );

    let (_, out) = quilt(home.path(), &["catalog", "clear"]);
    assert_eq!(out["default_catalog"], serde_json::Value::Null);
    let (_, out) = quilt(home.path(), &["catalog"]);
    assert_eq!(out["default_catalog"], serde_json::Value::Null);
}

#[test]
fn quilt3s_catalog_is_adopted_and_its_file_is_left_alone() {
    let home = TempDir::new().unwrap();
    let config = home.path().join("xdg/Quilt/config.yml");
    std::fs::create_dir_all(config.parent().unwrap()).unwrap();
    let yaml = "navigator_url: https://quilt3.example.com/\nregistryUrl: https://r.example.com\n";
    std::fs::write(&config, yaml).unwrap();

    let (_, out) = quilt(home.path(), &["catalog"]);

    assert_eq!(out["default_catalog"], "quilt3.example.com");
    assert_eq!(std::fs::read_to_string(&config).unwrap(), yaml);
    let stored = std::fs::read_to_string(home.path().join("auth/settings.json")).unwrap();
    assert!(stored.contains("quilt3.example.com"), "{stored}");

    // Clearing sticks: it is not adopted again.
    quilt(home.path(), &["catalog", "clear"]);
    let (_, out) = quilt(home.path(), &["catalog"]);
    assert_eq!(out["default_catalog"], serde_json::Value::Null);
}

#[test]
fn login_and_role_without_a_host_need_a_default() {
    let home = TempDir::new().unwrap();

    for args in [&["login"][..], &["role"][..]] {
        let (ok, out) = quilt(home.path(), args);
        assert!(!ok, "{args:?}");
        assert_eq!(out["error"]["kind"], "no_catalog", "{args:?}");
    }
}

#[test]
fn login_without_a_host_uses_the_default() {
    let home = TempDir::new().unwrap();
    quilt(home.path(), &["catalog", "set", "quilt.test"]);

    let (ok, out) = quilt(home.path(), &["login"]);

    // Without a code, login says where to get one, for the default's host.
    assert!(!ok);
    assert_eq!(out["error"]["kind"], "login_required");
    let message = out["error"]["message"].as_str().unwrap();
    assert!(message.contains("--host quilt.test"), "{message}");
}
