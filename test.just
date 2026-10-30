# The crates a test `scope` selects. `all` is every crate that tests on the
# host, with every target. CI tests the rest apart: `rs`, the quilt-rs crates,
# on a runner without Tauri's system libraries; `sync`, QuiltSync's backend;
# `ui`, the frontend's host tests.
#
# `quilt-sync-ui` has tests for two targets, and neither harness runs the
# other's: its `#[test]` functions run on the host, in `all` and `ui`, and its
# `#[wasm_bindgen_test]` ones run in `just test frontend`. DOM tests cannot run
# on the host, because `mount_to` and similar helpers are wasm-bindgen bindings
# into JS. Pure-logic tests could move to wasm instead. Once v1 is deleted only
# about twenty host tests remain, and converting them and dropping the host run
# becomes the cheaper option.
#
# `--lib --bins`, not `--bin quilt-sync-ui`: most of the tests are in the
# library target, and `--bin` alone runs only the few in the binary.
all := "--workspace --all-targets"
rs := "--workspace --all-targets --exclude quilt-sync --exclude quilt-sync-ui"
sync := "-p quilt-sync --all-targets"
ui := "-p quilt-sync-ui --lib --bins"

# The timeout limits only the wait for the tests to finish; browser start and
# page load have their own limits. Past it the runner fails with "Failed to
# detect test as having been run" and names no test. It is a hang guard, set
# well above the suite's time.
wasm_timeout := "180"

# The suite's time is watched against this smaller budget instead: over it, the
# recipe warns but passes, so growth shows long before the timeout.
wasm_budget := "75"

# `shift` drops the scope, so only the arguments after it reach nextest.
#
# Run the tests of `scope` (all, rs, sync or ui) needing no AWS credentials (what a fork's CI runs); arguments go to nextest
[positional-arguments]
no-aws scope="all" *args:
    shift && cargo nextest run --profile no-aws {{ if scope == "all" { all } else if scope == "rs" { rs } else if scope == "sync" { sync } else if scope == "ui" { ui } else { error("scope is all, rs, sync or ui") } }} "$@"

# Run the live_* fixture tests of `scope`, which need AWS credentials, narrowed by the nextest `filter`; arguments go to nextest
[positional-arguments]
live scope="all" filter="all()" *args:
    shift 2 && cargo nextest run {{ if scope == "all" { all } else if scope == "rs" { rs } else if scope == "sync" { sync } else if scope == "ui" { ui } else { error("scope is all, rs, sync or ui") } }} -E {{ quote("test(/::live_/) and (" + filter + ")") }} "$@"

# nextest does not run doctests. The QuiltSync crates are left out: CI's runner
# for them cannot build them.
#
# Run the doctests
doc:
    cargo test --doc --workspace --exclude quilt-sync --exclude quilt-sync-ui

# Run QuiltSync frontend tests in headless Firefox; warns when they outgrow their time budget
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
