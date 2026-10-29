# Simple justfile for quilt-rs workspace

# List the recipes, so a bare `just` starts nothing heavy
default:
    @{{ just_executable() }} --list

# The desktop app: `dev`
mod app "quilt-sync/app.just"

# The component gallery: `dev`, `build` and `preview`
mod gallery "quilt-sync/ui/gallery.just"

# The test suites: plain `just test`, `live`, `frontend`, and CI's test steps
mod test "test.just"

# Run test coverage for all packages
coverage:
    cargo tarpaulin --out html

# Report every formatting and lint failure; writes nothing
lint: lint-md
    cargo fmt --all --check
    cargo clippy --workspace --all-targets --all-features
    cargo clippy --target wasm32-unknown-unknown -p quilt-sync-ui --all-targets --all-features

# Fix what `lint` reports, over the same file set
fmt:
    #!/usr/bin/env bash
    set -euo pipefail
    cargo fmt --all
    # `rumdl fmt` looks equivalent but exits 0 with unfixable issues outstanding
    git ls-files -z '*.md' | xargs -0 rumdl check --fix

# Each CI job runs the recipes below; the workflow adds only triggers, setup
# and secrets. Cheap checks run first, so a failure ends the run early.
#
# Run every CI job that runs on Linux, live AWS tests included; use before a push
[group('ci')]
ci: lint-md deny lint-rs lint-sync test::rs test::rs-live test::writes test::doc test::sync test::sync-live ci-ui

# `base` is the branch the pull request merges into.
#
# Run what CI runs on a pull request on Linux, given the files changed since `base`
[group('ci')]
ci-pr base="origin/main":
    #!/usr/bin/env bash
    set -euo pipefail
    just={{ quote(just_executable()) }}
    eval "$("$just" changes "$(git merge-base {{ quote(base) }} HEAD)")"
    if $md; then "$just" lint-md; fi
    if $rs; then
        "$just" deny
        "$just" lint-rs
        "$just" test rs
        "$just" test rs-live
        "$just" test writes
        "$just" test doc
    fi
    if $sync; then
        "$just" lint-sync
        "$just" test sync
        "$just" test sync-live
    fi
    if $ui; then "$just" ci-ui; fi

# The jobs a change needs, one `name=true|false` line each, which is the format
# of a GitHub step output. A pull request in CI compares its merge commit with
# the base. `md` is the markdown lint; `rs` the quilt-rs crates; `sync`
# QuiltSync's backend, which builds on every crate but the UI; `ui` the
# frontend, which builds on its path dependencies (`cargo metadata`: only
# quilt-uri) and the workspace files. Only markdown changes skip `sync`, and
# only those and `quilt-sync/` skip `rs`: Cargo forbids dependency cycles, so
# no other crate depends on QuiltSync.
#
# Print which CI jobs the changes `git diff <args>` lists need
[group('ci')]
[positional-arguments]
changes *args:
    #!/usr/bin/env bash
    set -euo pipefail
    md=false rs=false sync=false ui=false
    while IFS= read -r f; do
        case "$f" in
            *.md | .rumdl.toml | justfile | .github/workflows/test.yaml) md=true ;;
        esac
        case "$f" in
            *.md) ;;
            quilt-sync/*) sync=true ;;
            *) rs=true sync=true ;;
        esac
        case "$f" in
            *.md) ;;
            quilt-sync/ui/* | quilt-uri/* | Cargo.toml | Cargo.lock | rust-toolchain.toml | clippy.toml | .cargo/* | \
                justfile | test.just | .github/workflows/test.yaml | .github/actions/*) ui=true ;;
        esac
    done < <(git diff --name-only "$@")
    printf '%s\n' "md=$md" "rs=$rs" "sync=$sync" "ui=$ui"

# rumdl needs an explicit file list: pointed at a directory it obeys every
# .gitignore above the checkout, and one ignoring `*` leaves it zero files.
#
# Lint the tracked markdown
[group('ci')]
lint-md:
    git ls-files -z '*.md' | xargs -0 rumdl check

# `--all-features` is the cargo-deny action's default.
#
# Check dependency licenses and advisories
[group('ci')]
deny:
    cargo deny --all-features check

# The quilt-rs crates: everything but QuiltSync, which needs Tauri's system
# libraries or wasm
rs := "--workspace --exclude quilt-sync --exclude quilt-sync-ui"

# Check formatting, lint the quilt-rs crates and build quilt-uri for wasm
[group('ci')]
lint-rs: && clippy-rs
    cargo fmt --all --check
    cargo check -p quilt-uri --target wasm32-unknown-unknown
    cargo check -p quilt-uri --target wasm32-unknown-unknown --all-features

# Lint the quilt-rs crates for this OS
[group('ci')]
clippy-rs:
    cargo clippy {{ rs }} --all-targets --all-features -- --deny warnings

# Check QuiltSync backend formatting and lint it
[group('ci')]
lint-sync: && clippy-sync
    cargo fmt --check -p quilt-sync

# Lint QuiltSync's backend for this OS
[group('ci')]
clippy-sync:
    cargo clippy -p quilt-sync --all-targets --all-features -- --deny warnings

# This crate has tests for two targets, and neither harness runs the other's:
# `wasm-bindgen-test-runner` runs only `#[wasm_bindgen_test]`, and libtest runs
# only `#[test]`. `test frontend` runs the wasm tests; the `cargo test` line
# runs the host tests.
#
# DOM tests cannot run on the host, because `mount_to` and similar helpers are
# wasm-bindgen bindings into JS. Pure-logic tests could move to wasm instead.
# Once v1 is deleted only about twenty host tests remain, and converting them
# and dropping the host run becomes the cheaper option.
#
# `--lib --bins`, not `--bin quilt-sync-ui`: most of the tests are in the
# library target, and `--bin` alone runs only the few in the binary.
#
# Lint, test and release-build QuiltSync's frontend
[group('ci')]
ci-ui:
    #!/usr/bin/env bash
    set -euo pipefail
    cargo fmt --check -p quilt-sync-ui
    cargo clippy --target wasm32-unknown-unknown -p quilt-sync-ui --all-targets --all-features -- --deny warnings
    {{ quote(just_executable()) }} test frontend
    cargo test -p quilt-sync-ui --lib --bins
    cd quilt-sync/ui && trunk build --release
