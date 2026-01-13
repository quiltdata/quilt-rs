# Contributing to Quilt Workspace

This repository contains multiple projects in a unified workspace:

- **[quilt-rs](quilt-rs/)** - Rust library for accessing Quilt data packages
- **[quilt-cli](quilt-cli/)** - Command-line interface for Quilt data packages
- **[quilt-sync](quilt-sync/)** - Cross-platform desktop GUI application (QuiltSync)

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

For debugging and verification purposes across all projects:

### SHA256-Chunked Verification

#### 0Mb Files

```bash
sha256sum ./FILE | xxd -r -p | base64
```

#### <= 8Mb Files

```bash
sha256sum ./FILE | xxd -r -p | sha256sum | xxd -r -p | base64
```

#### > 8Mb Files

```bash
split -b 8388608 ./FILE --filter='sha256sum' | xxd -r -p | \
  sha256sum | xxd -r -p | base64
```

### Verify Packages

```bash
split -l 1 ~/MANIFEST.jsonl --filter="jq -cSM 'del(.physical_keys)'" | \
  tr -d '\n' | sha256sum
```

### CRC64/NVMe Verification

**TODO**: CRC64/NVMe checksum verification procedures are not yet documented.
This is an area for future contribution and documentation improvements.
