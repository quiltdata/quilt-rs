# Simple justfile for quilt-rs workspace

# Start QuiltSync development server
start: ui-stubs
    cd quilt-sync && cargo tauri dev

# The component gallery: `dev`, `build` and `preview`
mod gallery "quilt-sync/ui/gallery.just"

# In the module because a module recipe cannot depend on a root one.
[private]
ui-stubs: gallery::stubs

# Run test coverage for all packages
coverage:
    cargo tarpaulin --out html

# Report every formatting and lint failure; writes nothing
lint:
    #!/usr/bin/env bash
    set -euo pipefail
    cargo fmt --all --check
    # rumdl needs an explicit file list: pointed at a directory it obeys every
    # .gitignore above the checkout, and one ignoring `*` leaves it zero files.
    git ls-files -z '*.md' | xargs -0 rumdl check
    cargo clippy --workspace --all-targets --all-features
    cargo clippy --target wasm32-unknown-unknown -p quilt-sync-ui --all-targets --all-features

# Fix what `lint` reports, over the same file set
fmt:
    #!/usr/bin/env bash
    set -euo pipefail
    cargo fmt --all
    # `rumdl fmt` looks equivalent but exits 0 with unfixable issues outstanding
    git ls-files -z '*.md' | xargs -0 rumdl check --fix

# Every crate that tests on the host. `quilt-uri` is not a default member, so a
# bare `cargo test` does not cover the whole workspace.
#
# `quilt-sync-ui` is included. It has tests for two targets, and neither harness
# runs the other's: its `#[test]` functions run here, and its
# `#[wasm_bindgen_test]` ones run in `test-frontend`. CI runs the same host tests
# in its `Test (host target)` step.
scope := "--workspace --all-targets"

# Run every test (the live_* fixture tests need AWS credentials)
test:
    cargo nextest run {{ scope }}

# Run only the tests needing no AWS credentials (what a fork's CI runs)
test-no-aws:
    cargo nextest run --profile no-aws {{ scope }}

# Run QuiltSync frontend tests in headless Firefox. The runner's default 20s
# budget covers the whole run, and the suite alone takes about 18s.
test-frontend:
    WASM_BINDGEN_TEST_TIMEOUT=60 CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=wasm-bindgen-test-runner cargo test -p quilt-sync-ui --target wasm32-unknown-unknown
