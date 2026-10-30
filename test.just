# The cargo flags that pick what `no-aws` and `live` test: every crate that
# tests on the host, with every target, unless CI narrows it for a job.
#
# `quilt-sync-ui` has tests for two targets, and neither harness runs the
# other's: its `#[test]` functions run here, and its `#[wasm_bindgen_test]`
# ones run in `just test frontend`.
export SCOPE := env("SCOPE", "--workspace --all-targets")

# The nextest filterset `live` runs; CI narrows it to split reads from writes.
export LIVE := env("LIVE", "test(/::live_/)")

# Run the tests needing no AWS credentials (what a fork's CI runs); arguments go to nextest
[positional-arguments]
no-aws *args:
    cargo nextest run --profile no-aws $SCOPE "$@"

# Run only the live_* fixture tests, which need AWS credentials; arguments go to nextest
[positional-arguments]
live *args:
    cargo nextest run $SCOPE -E "$LIVE" "$@"

# nextest does not run doctests. The QuiltSync crates are left out: CI's runner
# for them cannot build them.
#
# Run the doctests
doc:
    cargo test --doc --workspace --exclude quilt-sync --exclude quilt-sync-ui

# The runner's default timeout, 20 s for the whole run, is shorter than the
# suite. The timeout limits only the wait for the tests to finish; past it the
# runner fails with "Failed to detect test as having been run" and names no
# test. It is a hang guard, set well above the suite's time.
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
