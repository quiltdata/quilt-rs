<!--
     Follow keepachangelog.com format.
     Use GitHub autolinks for PR references.
     Use nested lists when there are multiple PR links.
     Use alpha pre-release versions (e.g. v0.24.1-alpha1) instead of [Unreleased]
     to keep changelog in sync with Cargo.toml version.
-->
<!-- markdownlint-disable MD013 -->
# Changelog

## [v0.24.1-alpha1] - 2026-03-26

### Added

- Add job stories for `quilt sync` — a single command to register, commit, and
  push a folder as a package ([docs/sync-folder.md](docs/sync-folder.md),
  <https://github.com/quiltdata/quilt-rs/pull/590>)
- Add `quilt create quilt+s3://bucket#package=owner/name [--source ...]` for
  creating a new local package from a remote package handle without installing
  an existing revision first
  (<https://github.com/quiltdata/quilt-rs/pull/590>)
- Add `version` to `quilt-rs` dependency to enable publishing `quilt-cli` to
  crates.io (<https://github.com/quiltdata/quilt-rs/pull/590>)

### Changed

- Update to use quilt-rs first-revision package support for create, commit, and
  push workflows (<https://github.com/quiltdata/quilt-rs/pull/590>)

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
