# Simple justfile for quilt-rs workspace

# Start QuiltSync development server
start:
    cd quilt-sync && cargo tauri dev

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

# Every crate that tests on the host — `quilt-uri` is not a default member, so a
# bare `cargo test` does not cover the workspace on its own.
#
# `quilt-sync-ui` is IN. It is not wasm-only, whatever this line used to say: the
# crate has two test flavours on two targets and neither harness sees the other,
# so its 124 `#[test]` functions run here and its `#[wasm_bindgen_test]` ones run
# in `test-frontend`. Excluding it meant a green `just test` said nothing about
# any of them — qhq-8mgw.27, the same hole CI had until the `Test (host target)`
# step closed it.
scope := "--workspace --all-targets"

# Run every test (the live_* fixture tests need AWS credentials)
test:
    cargo nextest run {{ scope }}

# Run only the tests needing no AWS credentials (what a fork's CI runs)
test-no-aws:
    cargo nextest run --profile no-aws {{ scope }}

# Run QuiltSync frontend tests in headless Firefox
test-frontend:
    CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=wasm-bindgen-test-runner cargo test -p quilt-sync-ui --target wasm32-unknown-unknown
