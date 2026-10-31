//! The app's version, for the page to compare with the engine it talks to: an
//! open tab keeps the UI it loaded, so after an upgrade it reloads.

use std::path::Path;

fn main() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("../src-tauri/Cargo.toml");
    println!("cargo:rerun-if-changed={}", manifest.display());
    let text = std::fs::read_to_string(&manifest)
        .unwrap_or_else(|err| panic!("could not read {}: {err}", manifest.display()));
    // The first `version` is the package's own.
    let version = text
        .lines()
        .find_map(|line| line.strip_prefix("version = \"")?.strip_suffix('"'))
        .unwrap_or_else(|| panic!("no version in {}", manifest.display()));
    println!("cargo:rustc-env=QUILTSYNC_VERSION={version}");
}
