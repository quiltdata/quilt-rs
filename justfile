# Simple justfile for quilt-rs workspace

# Start QuiltSync development server
start:
    cd quilt-sync && cargo tauri dev

# Run test coverage for all packages
coverage:
    cargo tarpaulin --out html

# Lint all packages with all features
lint:
    cargo clippy --workspace --all-targets --all-features
    cargo clippy --target wasm32-unknown-unknown -p quilt-sync-ui --all-targets --all-features

# Every crate that tests on the host. `quilt-uri` is not a default member and
# `quilt-sync-ui` is wasm-only (see `test-frontend`), so neither `cargo test`
# nor a bare `cargo nextest run` covers the workspace on its own.
scope := "--workspace --all-targets --exclude quilt-sync-ui"

# Run every test (the live_* fixture tests need AWS credentials)
test:
    cargo nextest run {{ scope }}

# Run only the tests needing no AWS credentials (what a fork's CI runs)
test-no-aws:
    cargo nextest run --profile no-aws {{ scope }}

# Run QuiltSync frontend tests in headless Firefox
test-frontend:
    CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=wasm-bindgen-test-runner cargo test -p quilt-sync-ui --target wasm32-unknown-unknown
