# Every crate that tests on the host. `quilt-uri` is not a default member, so a
# bare `cargo test` does not cover the whole workspace.
#
# `quilt-sync-ui` is included. It has tests for two targets, and neither harness
# runs the other's: its `#[test]` functions run here, and its
# `#[wasm_bindgen_test]` ones run in `just test frontend`. CI runs the same host
# tests in its `Test (host target)` step.
scope := "--workspace --all-targets"

# Run the tests needing no AWS credentials (what a fork's CI runs)
no-aws:
    cargo nextest run --profile no-aws {{ scope }}

# Run only the live_* fixture tests, which need AWS credentials
live:
    cargo nextest run {{ scope }} -E 'test(/::live_/)'

# The runner's default timeout covers the whole run, and the suite alone nearly
# fills it.
#
# Run QuiltSync frontend tests in headless Firefox
frontend:
    WASM_BINDGEN_TEST_TIMEOUT=60 CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=wasm-bindgen-test-runner cargo test -p quilt-sync-ui --target wasm32-unknown-unknown
