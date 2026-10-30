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

# The cargo flags that pick what `check` and `clippy` lint: the whole
# workspace, unless CI narrows it for a job.
export SCOPE := env("SCOPE", "--workspace --all-targets")

# Report every formatting and lint failure; writes nothing
lint: lint-md check wasm-check
    cargo clippy --target wasm32-unknown-unknown -p quilt-sync-ui --all-targets --all-features -- --deny warnings

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

# Check formatting, then lint `SCOPE` with clippy
check: && clippy
    cargo fmt --all --check

# Lint `SCOPE` with clippy for this OS
clippy:
    cargo clippy $SCOPE --all-features -- --deny warnings

# Build quilt-uri for wasm
wasm-check:
    cargo check -p quilt-uri --target wasm32-unknown-unknown
    cargo check -p quilt-uri --target wasm32-unknown-unknown --all-features

# `--all-features` is the cargo-deny action's default.
#
# Check dependency licenses and advisories
deny:
    cargo deny --all-features check

# Build QuiltSync's frontend for release
release-build:
    cd quilt-sync/ui && trunk build --release

# Each CI job runs these recipes, some with a narrower `SCOPE`; the workflow adds
# only triggers, setup and secrets. Cheap checks run first, so a failure ends
# the run early.
#
# Run everything CI checks, live AWS tests included; use before a push
ci: lint deny test::no-aws test::live test::doc test::frontend release-build
