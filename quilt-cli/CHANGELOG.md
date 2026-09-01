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

## [v0.30.0-alpha6] - 2026-08-31

### Added

- `quilt list --json` and `quilt status --json` print a machine-readable form, so `quilt list --json | jq` works (<https://github.com/quiltdata/quilt-rs/pull/851>)

### Fixed

- A command that fails now exits non-zero. It reported the error and exited 0, so a shell script could not tell success from failure (<https://github.com/quiltdata/quilt-rs/pull/851>)

## [v0.30.0-alpha5] - 2026-08-31

### Added

- `quilt log` lists a package's revisions, newest first, with the short hash, the commit message, and the date this copy obtained the revision. That date is what is recorded locally and nothing more: a manifest carries no timestamp of its own, so for a revision fetched from a remote it is the fetch time rather than the commit time — hence the `obtained` heading. A package's full published history is not available yet (<https://github.com/quiltdata/quilt-rs/pull/849>)

## [v0.30.0-alpha4] - 2026-08-31

### Changed

- `commit`, `pull`, `push`, `status` and `uninstall` infer the package from the directory you are standing in, so `--namespace` is only needed from outside a package's working copy. Inference reads the two path components below the configured home; anywhere else it is an error naming the flag (<https://github.com/quiltdata/quilt-rs/pull/850>)

## [v0.30.0-alpha3] - 2026-08-31

### Changed

- `quilt` no longer needs `--home` on first use — it defaults to `~/QuiltSync`, the same working-copy directory QuiltSync uses. Pass `--home` to choose a different one; it is still remembered after that (<https://github.com/quiltdata/quilt-rs/pull/848>)

### Fixed

- A command that cannot resolve the package home now exits non-zero, instead of reporting the error and exiting 0 (<https://github.com/quiltdata/quilt-rs/pull/848>)

## [v0.30.0-alpha2] - 2026-08-31

### Added

- `-v` / `--verbose` shows INFO-level logs on stderr; `RUST_LOG` still takes precedence for target-specific filtering (<https://github.com/quiltdata/quilt-rs/pull/847>)

### Changed

- Piping a command's output now works — stdout carries only the command's own output, and logs go to stderr at WARN and above (was INFO on stdout) (<https://github.com/quiltdata/quilt-rs/pull/847>)

## [v0.30.0-alpha1] - 2026-08-07

### Changed

- `quilt list` prints a table of bucket, namespace and upstream status — each bucket named once, spanning its packages' rows — instead of one `InstalledPackage<namespace>` line per package. Status comes from the local lineage record, so listing stays offline and reports the last-known remote tip; `quilt status <namespace>` refreshes it (<https://github.com/quiltdata/quilt-rs/pull/846>)

## [v0.29.2] - 2026-08-07

### quilt-rs

- A package can now be set to download its whole contents, including files added later. `quilt pull` deliberately does **not** honour that setting — it keeps fetching only the files you already have, whatever the desktop app recorded for the package (<https://github.com/quiltdata/quilt-rs/pull/834>)

## [v0.29.1] - 2026-08-06

### quilt-rs

- Log levels rearranged and long file-list dumps replaced with summaries: a `RUST_LOG=debug` log is roughly 100× smaller. `RUST_LOG=quilt_rs=trace` brings the per-file detail back (<https://github.com/quiltdata/quilt-rs/pull/828>)

## [v0.29.0] - 2026-07-30

### Added

- `quilt role --host HOST` lists the roles you hold and marks the active one; `--set ROLE` switches it, which is server-side and so applies to every Quilt client signed in as you (<https://github.com/quiltdata/quilt-rs/pull/807>)

### Changed

- `quilt status` now exits non-zero with the reason when the active role cannot read the package's bucket, instead of printing stale state from the last successful refresh (<https://github.com/quiltdata/quilt-rs/pull/807>)

### quilt-rs

- Updated [from v0.33.0 to v0.34.0](https://github.com/quiltdata/quilt-rs/compare/quilt-rs/v0.33.0...quilt-rs/v0.34.0) (see [quilt-rs/CHANGELOG.md](../quilt-rs/CHANGELOG.md))

### quilt-uri

- Updated [from v0.3.0 to v0.4.0](https://github.com/quiltdata/quilt-rs/compare/quilt-uri/v0.3.0...quilt-uri/v0.4.0) (see [quilt-uri/CHANGELOG.md](../quilt-uri/CHANGELOG.md))

## [v0.28.0] - 2026-07-14

### Changed

- Workflow support: `quilt commit` gains `--workflow` / `--no-workflow` (omitting `--workflow` applies the bucket's default workflow; an explicitly-empty `--workflow` is rejected), `quilt push` gains the same flags to choose the workflow for a package's first push, committing / publishing / first-pushing a package that fails its bucket's workflow is refused with the rule it violated, and `quilt push` warns on stderr when it attaches a remote but cannot resolve the bucket's default workflow (<https://github.com/quiltdata/quilt-rs/pull/747>, <https://github.com/quiltdata/quilt-rs/pull/748>, <https://github.com/quiltdata/quilt-rs/pull/753>, <https://github.com/quiltdata/quilt-rs/pull/755>)

### Fixed

- `quilt commit` without `--user-meta` preserves the package's existing metadata instead of silently dropping it (<https://github.com/quiltdata/quilt-rs/pull/734>)

### quilt-rs

- Updated [from v0.32.0 to v0.33.0](https://github.com/quiltdata/quilt-rs/compare/quilt-rs/v0.32.0...quilt-rs/v0.33.0) (see [quilt-rs/CHANGELOG.md](../quilt-rs/CHANGELOG.md))

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
