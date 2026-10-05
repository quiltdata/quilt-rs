# Simple justfile for quilt-rs workspace

# Start QuiltSync development server
start: ui-stubs
    cd quilt-sync && cargo tauri dev

# Trunk refuses to start until the generated files in its watch-ignore list exist,
# so a fresh checkout fails before the pre-build hooks that write them ever run.
# Empty stubs get it past that check; the hooks then overwrite them.
[private]
ui-stubs:
    cd quilt-sync/ui && mkdir -p assets/js && touch assets/js/json-editor.js assets/css/kit/_modules.scss assets/css/kit/_normalize.scss

# Serve the component gallery (the kit's design record, never the app); `--open`
# opens a browser tab.
# Needs the frontend toolchain, see CONTRIBUTING.md.
#
# `--dist` is load-bearing. Trunk writes whatever target it is given to
# `<dist>/index.html`, and `just start` points Tauri at `ui/dist`, so sharing one
# directory means whichever of the two rebuilt last owns the page both of them serve.
# With its own directory the gallery and the app run side by side.
#
# Extra arguments go to `trunk serve`: `just gallery --open`, `just gallery --address 0.0.0.0`.
# The port defaults through Trunk's env var rather than `--port`, so `just gallery --port 9000`
# overrides it instead of passing the flag twice.
gallery *args: ui-stubs
    cd quilt-sync/ui && TRUNK_SERVE_PORT="${TRUNK_SERVE_PORT:-8787}" trunk serve gallery.html --dist dist-gallery {{args}}

# Use `just gallery` while iterating: it rebuilds fast and keeps full names in
# panics, but its wasm is large. Use this one for a final look before pushing:
# its wasm is as small as the shipped app's, but each build is slower.
#
# Serve the gallery as a release build (`just gallery` with Trunk's `--release`)
gallery-release *args:
    TRUNK_BUILD_RELEASE=true {{just_executable()}} gallery {{args}}

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
