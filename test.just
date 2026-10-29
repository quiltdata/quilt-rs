# Every crate that tests on the host, with every target.
#
# `quilt-sync-ui` is included. It has tests for two targets, and neither harness
# runs the other's: its `#[test]` functions run here, and its
# `#[wasm_bindgen_test]` ones run in `just test frontend`.
scope := "--workspace --all-targets"

# The crates CI tests apart from QuiltSync: `quilt-sync` needs Tauri's system
# libraries, and `quilt-sync-ui` is tested in the frontend job. Cargo forbids
# dependency cycles, so nothing here depends on either.
rs := "--workspace --all-targets --exclude quilt-sync --exclude quilt-sync-ui"

# The timeout limits only the wait for the tests to finish; browser start and
# page load have their own limits. Past it the runner fails with "Failed to
# detect test as having been run" and names no test. It is a hang guard, set
# well above the suite's time.
wasm_timeout := "180"

# The suite's time is watched against this smaller budget instead: over it, the
# recipe warns but passes, so growth shows long before the timeout.
wasm_budget := "75"

# Run the tests needing no AWS credentials (what a fork's CI runs); arguments go to nextest
[positional-arguments]
no-aws *args:
    cargo nextest run --profile no-aws {{ scope }} "$@"

# Run only the live_* fixture tests, which need AWS credentials
live:
    cargo nextest run {{ scope }} -E 'test(/::live_/)'

# Run QuiltSync frontend tests in headless Firefox
frontend:
    #!/usr/bin/env bash
    set -euo pipefail
    log=$(mktemp)
    WASM_BINDGEN_TEST_TIMEOUT={{ wasm_timeout }} CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=wasm-bindgen-test-runner \
        cargo test -p quilt-sync-ui --target wasm32-unknown-unknown 2>&1 | tee "$log"
    secs=$(grep -oE 'finished in [0-9.]+s' "$log" | grep -oE '[0-9.]+' | sort -n | tail -1 || true)
    if [ -n "$secs" ] && awk -v s="$secs" -v b={{ wasm_budget }} 'BEGIN { exit !(s > b) }'; then
        echo "::warning title=Wasm test suite time::The wasm tests took ${secs}s, over the {{ wasm_budget }}s budget (timeout {{ wasm_timeout }}s). Speed up or split the suite."
    fi

# The recipes below are CI's test steps; `just ci` runs them all.

# Run the quilt-rs crates' tests needing no AWS credentials; arguments go to nextest
[group('ci')]
[positional-arguments]
rs *args:
    cargo nextest run --profile no-aws {{ rs }} "$@"

# Run the quilt-rs crates' live_* tests that only read S3; arguments go to nextest
[group('ci')]
[positional-arguments]
rs-live *args:
    cargo nextest run {{ rs }} -E 'test(/::live_/) and not test(/::live_write_/)' "$@"

# Run the live_write_* tests, which write to S3; arguments go to nextest
[group('ci')]
[positional-arguments]
writes *args:
    cargo nextest run {{ rs }} -E 'test(/::live_write_/)' "$@"

# nextest does not run doctests. The QuiltSync crates are left out: CI's runner
# for them cannot build them.
#
# Run the quilt-rs crates' doctests
[group('ci')]
doc:
    cargo test --doc --workspace --exclude quilt-sync --exclude quilt-sync-ui

# Run QuiltSync's backend tests needing no AWS credentials; arguments go to nextest
[group('ci')]
[positional-arguments]
sync *args:
    cargo nextest run --profile no-aws -p quilt-sync "$@"

# These only read the fixtures. A `live_write_*` test here would need a place
# in CI's `live-writes` job.
#
# Run QuiltSync's backend live_* tests
[group('ci')]
sync-live:
    cargo nextest run -p quilt-sync -E 'test(/::live_/)'
