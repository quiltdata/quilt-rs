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

# Run only the live_* fixture tests, which need AWS credentials
live:
    cargo nextest run {{ scope }} -E 'test(/::live_/)'

# The timeout limits only the wait for the tests to finish. It is a hang guard,
# well above the suite's time; CI warns when the suite outgrows its budget.
#
# Run QuiltSync frontend tests in headless Firefox
frontend:
    WASM_BINDGEN_TEST_TIMEOUT=180 CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=wasm-bindgen-test-runner cargo test -p quilt-sync-ui --target wasm32-unknown-unknown

# An MVP, kept out of `ci`. Builds the debug app (which sends no telemetry) and
# the UI, then drives headless QuiltSync in Playwright's Chromium.
#
# Run the QuiltSync browser end-to-end tests
e2e *args:
    cargo build {{ scope }}
    cd quilt-sync/ui && trunk build
    cd quilt-sync/e2e && npm ci --no-audit --no-fund && npx playwright install chromium && npx playwright test {{ args }}
