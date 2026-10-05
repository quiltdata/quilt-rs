# Simple justfile for quilt-rs workspace

# List the recipes, so a bare `just` starts nothing heavy
default:
    @{{ just_executable() }} --list

# The desktop app: `dev`
mod app "quilt-sync/app.just"

# The component gallery: `dev`, `build` and `preview`
mod gallery "quilt-sync/ui/gallery.just"

# The test suites: plain `just test`, `live` and `frontend`
mod test "test.just"

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

# The list is kept in step with .github/workflows by hand; CI runs it as
# separate jobs, not through this recipe. Cheap checks run first, so a failure
# ends the run early.
#
# Run everything CI checks, live AWS tests included; use before a push
ci: lint ci-static test::no-aws test::live ci-doctests test::frontend ci-release-build

# `--all-features` is the cargo-deny action's default
[private]
ci-static:
    cargo deny --all-features check
    cargo check -p quilt-uri --target wasm32-unknown-unknown
    cargo check -p quilt-uri --target wasm32-unknown-unknown --all-features

# nextest does not run doctests. CI leaves out the two QuiltSync crates; their
# runner cannot build them.
[private]
ci-doctests:
    cargo test --doc --workspace --exclude quilt-sync --exclude quilt-sync-ui

[private]
ci-release-build:
    cd quilt-sync/ui && trunk build --release
