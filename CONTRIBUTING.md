# Contributing to Quilt Workspace

This repository contains multiple projects in a unified workspace:

- **[quilt-rs](quilt-rs/)** - Rust library for accessing Quilt data packages
  (built on [aws-sdk-rust](https://github.com/awslabs/aws-sdk-rust) and
  [Tokio](https://tokio.rs/))
- **[quilt-cli](quilt-cli/)** - Command-line interface for Quilt data packages
  (built with [clap](https://github.com/clap-rs/clap))
- **[quilt-sync](quilt-sync/)** - Cross-platform desktop GUI application built
  with [Tauri](https://tauri.app/) and vanilla JavaScript (no frontend framework)
  (QuiltSync)

## Project-Specific Contributing Guides

For detailed contributing information, see the project-specific guides:

- **[quilt-rs Contributing Guide](quilt-rs/CONTRIBUTING.md)** - Rust library and
  CLI development
- **[QuiltSync Contributing Guide](quilt-sync/CONTRIBUTING.md)** - Desktop
  application development

## Development Workflows

This project uses `just` as a task runner for common development tasks.

```bash
cargo install just
cargo install cargo-nextest --locked   # the test runner CI uses

just -l
```

All cargo commands work on the entire workspace by default. Use the `-p` flag to
target specific packages:

```bash
# Testing
cargo test                          # All workspace packages
cargo test -p quilt-rs              # Specific package only

# Building, formatting, linting follow the same pattern
cargo build [-p package-name]
cargo fmt [--check] [-p package-name]
cargo clippy [-- --deny warnings] [-p package-name]
```

### Tests that need AWS

Tests that read or write the shared S3 fixtures are named `live_*`. Nothing
marks them as skipped, so `cargo test` and `cargo nextest run` both run them by
default and both need AWS credentials in the environment.

Without credentials, deselect them by name:

```bash
just test-no-aws            # the recipe; wraps the line below
cargo nextest run --profile no-aws
cargo test -- --skip live_  # same effect without nextest
```

Note that `cargo test --profile no-aws` does **not** work — to `cargo`,
`--profile` names a build profile, and it will fail with `profile 'no-aws' is
not defined`. The `no-aws` profile belongs to nextest.

CI splits the same line: one step runs the `no-aws` selection everywhere, and a
second step runs `live_*` with credentials, skipped when the pull request comes
from a fork. GitHub withholds secrets from fork pull requests — a platform
rule, not a project choice — so the split is what lets an outside contributor
get a CI signal at all.

**The naming convention is load-bearing.** A test that touches the fixtures but
is not named `live_*` lands in the credential-free step, and fails there on
every run — including your own pushes, not just fork pull requests. If a test
you just wrote fails with a credentials error, check its name first.

## Release Process Overview

Each project has different release approaches:

- **quilt-rs**: Library published to crates.io via GitHub Actions
- **quilt-cli**: Published to crates.io and as prebuilt binaries on GitHub
  Releases for macOS (x86_64, aarch64) and Linux (x86_64-gnu);
  install via `cargo binstall quilt-cli` or `cargo install quilt-cli`
- **QuiltSync**: Desktop app releases with cross-platform builds via GitHub Actions

### Version Management

- **Library (`quilt-rs`)**: Versioned and published to crates.io
- **CLI (`quilt-cli`)**: Versioned and published to crates.io; each release
  also attaches prebuilt binary archives (`quilt-cli-<target>.tar.gz`) to
  the corresponding GitHub Release for `cargo binstall` discovery
- **QuiltSync (`quilt-sync`)**: Uses workspace version for Tauri app releases

### Pre-release Versioning

For unreleased changes, use pre-release tags
in both `Cargo.toml` and `CHANGELOG.md` (e.g., `0.24.0-alpha.1`).

See project-specific contributing guides for detailed release procedures.

## File Integrity Verification

See [docs/verification.md](docs/verification.md) for SHA256-chunked,
CRC64/NVMe, and manifest verification recipes.

## Reporting Security Issues

See [SECURITY.md](SECURITY.md) for how to report vulnerabilities privately
and what is in scope.
