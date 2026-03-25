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

## Release Process Overview

Each project has different release approaches:

- **quilt-rs**: Library published to crates.io via GitHub Actions
- **quilt-cli**: No separate releases - users compile from source
- **QuiltSync**: Desktop app releases with cross-platform builds via GitHub Actions

### Version Management

- **Library (`quilt-rs`)**: Versioned and published to crates.io
- **CLI (`quilt-cli`)**: Not published, inherits version from workspace but not
  released
- **QuiltSync (`quilt-sync`)**: Uses workspace version for Tauri app releases

### Pre-release Versioning

For unreleased changes, use pre-release tags
in both `Cargo.toml` and `CHANGELOG.md` (e.g., `0.24.0-alpha.1`).

See project-specific contributing guides for detailed release procedures.

## File Integrity Verification

See [docs/verification.md](docs/verification.md) for SHA256-chunked,
CRC64/NVMe, and manifest verification recipes.
