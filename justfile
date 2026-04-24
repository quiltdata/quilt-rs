# Simple justfile for quilt-rs workspace

# Start QuiltSync development server
start:
    cd quilt-sync && cargo tauri dev

# Run all tests (includes CLI bin target)
test:
    cargo test --all-features

# Run test coverage for all packages
coverage:
    cargo tarpaulin --all-features --out html

# Lint all packages with all features
lint:
    cargo clippy --all-features -- --deny warnings
    cargo clippy --target wasm32-unknown-unknown -p quilt-sync-ui -- --deny warnings

# Run QuiltSync frontend tests in headless Firefox
test-frontend:
    CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=wasm-bindgen-test-runner cargo test -p quilt-sync-ui --target wasm32-unknown-unknown
