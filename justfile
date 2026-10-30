# Simple justfile for quilt-rs workspace

# List the recipes, so a bare `just` starts nothing heavy
default:
    @{{ just_executable() }} --list

# The desktop app: `dev`
mod app "quilt-sync/app.just"

# The component gallery: `dev`, `build` and `preview`
mod gallery "quilt-sync/ui/gallery.just"

# The test suites: plain `just test`, `live`, `frontend` and `doc`
mod test "test.just"

# Run test coverage for all packages
coverage:
    cargo tarpaulin --out html

# The crates a lint `scope` selects. `all` is every crate built for the host.
# CI lints the rest apart: `rs`, the quilt-rs crates, on a runner without
# Tauri's system libraries; `sync`, QuiltSync's backend; `ui`, the frontend,
# which is built for wasm. Cargo forbids dependency cycles, so no `rs` crate
# depends on QuiltSync.
lint_all := "--workspace"
lint_rs := "--workspace --exclude quilt-sync --exclude quilt-sync-ui"
lint_sync := "-p quilt-sync"
lint_ui := "--target wasm32-unknown-unknown -p quilt-sync-ui"

# Report every formatting and lint failure; writes nothing
lint: lint-md (check "all") (clippy "ui")

# `rumdl fmt` looks equivalent but exits 0 with unfixable issues outstanding.
#
# Fix what `lint` reports, over the same file set
fmt:
    cargo fmt --all
    git ls-files -z '*.md' | xargs -0 rumdl check --fix

# rumdl needs an explicit file list: pointed at a directory it obeys every
# .gitignore above the checkout, and one ignoring `*` leaves it zero files.
#
# Lint the tracked markdown
lint-md:
    git ls-files -z '*.md' | xargs -0 rumdl check

# Check formatting, then lint `scope` (all, rs, sync or ui); `all` and `rs` also build quilt-uri for wasm
check scope="all": && (clippy scope)
    cargo fmt --all --check
    {{ if scope =~ '^(all|rs)$' { "cargo check -p quilt-uri --target wasm32-unknown-unknown" } else { "" } }}
    {{ if scope =~ '^(all|rs)$' { "cargo check -p quilt-uri --target wasm32-unknown-unknown --all-features" } else { "" } }}

# Lint `scope` (all, rs, sync or ui) with clippy for this OS
clippy scope="all":
    cargo clippy {{ if scope == "all" { lint_all } else if scope == "rs" { lint_rs } else if scope == "sync" { lint_sync } else if scope == "ui" { lint_ui } else { error("scope is all, rs, sync or ui") } }} --all-targets --all-features -- --deny warnings

# `--all-features` is the cargo-deny action's default.
#
# Check dependency licenses and advisories
deny:
    cargo deny --all-features check

# Build QuiltSync's frontend for release
release-build:
    cd quilt-sync/ui && trunk build --release

# Each CI job runs these recipes, some with a narrower scope; the workflow adds
# only triggers, setup and secrets. Cheap checks run first, so a failure ends
# the run early.
#
# Run everything CI checks, live AWS tests included; use before a push
ci: lint deny test::no-aws test::live test::doc test::frontend release-build
