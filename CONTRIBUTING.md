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

## Getting a pull request merged

Three things are checked on every pull request. Meeting them is necessary
rather than sufficient — a change still has to be correct, readable, and free
of anything that weakens security, and review will say so where it is not. But
a pull request missing any of these will be sent back, so they are the cheap
ones to check first.

**CI is green.** A pull request from a fork runs the same checks as a branch in
this repository, minus the tests that need AWS fixture credentials — GitHub does
not pass secrets to a fork's workflow run, so those are skipped rather than
failed. Everything else runs, and a red check is a red check wherever it came
from.

**Every review comment is resolved, and @greptileai is at 5/5.** Reviews come
from Greptile and Copilot as well as from maintainers. The bots are usually
right and sometimes wrong — a reply explaining why a comment does not apply
resolves it just as well as a code change does. Silence does not. If Greptile
holds below 5/5 over something we have deliberately decided against, say so in a
reply and a maintainer will merge past it.

**The docs still tell the truth.** This is the one most pull requests miss. The
`README.md` files describe current behaviour and cite open issues by number for
known limitations, so a change that closes an issue tends to falsify a paragraph
somewhere else in the tree. Grep for the issue number *and* for the behaviour
you changed; they do not find the same lines. A command that gains a flag, a
default, or a subcommand usually touches the root `README.md` quickstart, the
crate's own `README.md`, and any "known issues" list that named the gap.

Changelog entries are welcome but not required — a maintainer will write one
if you have not.

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
