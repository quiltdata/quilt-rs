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

# The engine embeds the UI from `ui/dist`, so it is built first. A restart
# frees its port for a moment, and another test's engine could take it, so the
# restart tests run after the others, one at a time.
#
# Run the browser tests against headless QuiltSync in Chrome; arguments go to nextest
[positional-arguments]
e2e *args:
    cd quilt-sync/ui && trunk build
    cargo nextest run -p quilt-sync --features e2e -E 'test(/engine::e2e::/) - test(/engine::e2e::restart::/)' "$@"
    cargo nextest run -p quilt-sync --features e2e -E 'test(/engine::e2e::restart::/)' -j 1 "$@"
