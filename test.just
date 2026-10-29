# Every crate that tests on the host, with every target.
#
# `quilt-sync-ui` is included. It has tests for two targets, and neither harness
# runs the other's: its `#[test]` functions run here, and its
# `#[wasm_bindgen_test]` ones run in `just test frontend`. CI runs the same host
# tests in its `Test (host target)` step.
scope := "--workspace --all-targets"

# Run the tests needing no AWS credentials (what a fork's CI runs); arguments go to nextest
[positional-arguments]
no-aws *args:
    cargo nextest run --profile no-aws {{ scope }} "$@"

# On macOS and Windows, a pull request's CI runs only this selection
#
# Run the tests of code that differs by OS; arguments go to nextest
[positional-arguments]
os *args:
    cargo nextest run --profile os-sensitive {{ scope }} "$@"

# Check that every file branching on the OS has a test in `just test os`
os-check:
    .config/check-os-sensitive.sh {{ scope }}

# Run only the live_* fixture tests, which need AWS credentials
live:
    cargo nextest run {{ scope }} -E 'test(/::live_/)'

# The timeout limits only the wait for the tests to finish. It is a hang guard,
# well above the suite's time; CI warns when the suite outgrows its budget.
#
# Run QuiltSync frontend tests in headless Firefox
frontend:
    WASM_BINDGEN_TEST_TIMEOUT=180 CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=wasm-bindgen-test-runner cargo test -p quilt-sync-ui --target wasm32-unknown-unknown
