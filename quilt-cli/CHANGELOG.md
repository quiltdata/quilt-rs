<!--
     Follow keepachangelog.com format.
     Use GitHub autolinks for PR references.
     Use nested lists when there are multiple PR links.
     Put quilt-rs and quilt-uri updates under their respective `###` section.
     Use alpha pre-release versions (e.g. v0.24.1-alpha1) instead of [Unreleased]
     to keep changelog in sync with Cargo.toml version.
-->
<!-- markdownlint-disable MD013 -->
# Changelog

## [v0.27.0] - 2026-05-25

### Changed

- `quilt` now stores its default data directory under `com.quiltdata.quilt-sync` so state is shared with QuiltSync; users with an existing `com.quiltdata.quilt-rs` directory should move it manually (<https://github.com/quiltdata/quilt-rs/pull/696>)

### Fixed

- `quilt login --help` now describes the subcommand correctly (was "List installed packages") (<https://github.com/quiltdata/quilt-rs/pull/695>)

## [v0.26.0] - 2026-05-19

### Changed

- `quilt status` now prints "Your commits are detached from the remote" (was "Local-only package") for a package whose configured remote already has revisions published by another client (<https://github.com/quiltdata/quilt-rs/pull/682>)
- `quilt status` no longer refreshes the on-disk lineage as a side effect; the `latest_hash` refresh moved into operations that actually need it (<https://github.com/quiltdata/quilt-rs/pull/682>)

### quilt-rs

- Updated [from v0.31.1 to v0.32.0](https://github.com/quiltdata/quilt-rs/compare/quilt-rs/v0.31.1...quilt-rs/v0.32.0) (see [quilt-rs/CHANGELOG.md](../quilt-rs/CHANGELOG.md))

## [v0.25.3] - 2026-05-06

### quilt-rs

- Bumped to v0.31.1 (<https://github.com/quiltdata/quilt-rs/pull/664>, see [quilt-rs/CHANGELOG.md](../quilt-rs/CHANGELOG.md))

### quilt-uri

- Bumped to v0.3.0 (<https://github.com/quiltdata/quilt-rs/pull/664>, see [quilt-uri/CHANGELOG.md](../quilt-uri/CHANGELOG.md))

## [v0.25.2] - 2026-05-04

### Added

- Publish prebuilt macOS (x86_64, aarch64) and Linux (x86_64-gnu) binaries on each release; `cargo binstall quilt-cli` now downloads them instead of compiling from source (<https://github.com/quiltdata/quilt-rs/pull/659>)

### quilt-rs

- Bumped to v0.31.0 (<https://github.com/quiltdata/quilt-rs/pull/660>, see [quilt-rs/CHANGELOG.md](../quilt-rs/CHANGELOG.md))

### quilt-uri

- Bumped to v0.2.0 (<https://github.com/quiltdata/quilt-rs/pull/660>, see [quilt-uri/CHANGELOG.md](../quilt-uri/CHANGELOG.md))

## [v0.25.1] - 2026-04-29

### Added

- First crates.io release — install with `cargo install quilt-cli`, then run `quilt`

### Changed

- `quilt push` now warns when the latest tag could not be updated (remote has newer changes) instead of silently succeeding
- Migrated to the Rust 2024 edition; building from source now requires Rust 1.85+ (<https://github.com/quiltdata/quilt-rs/pull/646>)

### quilt-rs

- Updated [from v0.28.0 to v0.30.1](https://github.com/quiltdata/quilt-rs/compare/quilt-rs/v0.28.0...quilt-rs/v0.30.1) (see [quilt-rs/CHANGELOG.md](../quilt-rs/CHANGELOG.md))

### quilt-uri

- Added v0.1.0 (see [quilt-uri/CHANGELOG.md](../quilt-uri/CHANGELOG.md))

## [v0.25.0] - 2026-04-07

### Added

- Add `quilt create` command for creating new local-only packages with optional `--source` and `--message` flags (<https://github.com/quiltdata/quilt-rs/pull/596>)
- Add `--bucket` and `--origin` flags to `quilt push` for first push of local-only packages (<https://github.com/quiltdata/quilt-rs/pull/596>)

## [v0.24.0] - 2025-02-04

### Changed

- Updated to use quilt-rs v0.27.0 with JSONL manifest format
  migration (<https://github.com/quiltdata/quilt-rs/pull/476>)

## [v0.23.0] - 2025-11-28

### Added

- Improved test coverage for CLI model with `HostConfig`
  parameter (<https://github.com/quiltdata/quilt-rs/pull/393>)

### Changed

- Updated to use quilt-rs v0.23.0 with CRC64/NVMe object hash
  support (<https://github.com/quiltdata/quilt-rs/pull/393>)

## [v0.8.11] - 2025-02-XX

### Added

- **New `login` command** for Quilt Stack authentication
- Support for authentication to Quilt Stack with backward compatibility for
  `~/.aws` credentials

### Changed

- `domain` path now optional for users (uses default user data directory if not
  provided)
- Domain path required internally for every command but seamless for end users

## [v0.8.8] - 2025-01-XX

### Added

- **New `workflow` parameter** for commit command
- Comprehensive integration tests for CLI commands using real Quilt packages

### Changed

- Increased CLI test coverage to 79%
- CLI tests now treated as integration tests with real package data

## [v0.8.6] - 2024-12-XX

### Added

- `package` command now accepts `--message` and `--user_meta` arguments
  (similar to `commit` command)

## [v0.8.5] - 2024-12-XX

### Changed

- `package` command now automatically calculates checksum if missing

## [v0.5.7] - 2024-03-21

### Added

- **Initial CLI implementation** with core commands:
  - `browse` - Browse remote manifest
  - `install` - Install packages locally
  - `list` - List installed packages
  - `package` - Package management
  - `uninstall` - Uninstall packages

### Changed

- Added complete command-line interface as frontend for quilt-rs library

## Earlier Versions

Prior to v0.5.7, CLI functionality was not yet implemented. The library
provided the core functionality but no command-line interface was available.

See [`quilt-rs/CHANGELOG.md`](../quilt-rs/CHANGELOG.md) for complete library
changes that power these CLI commands.
