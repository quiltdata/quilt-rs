<!--
     Follow keepachangelog.com format.
     Use GitHub autolinks for PR references.
     Use nested lists when there are multiple PR links.
     Put quilt-rs and quilt-uri updates under their respective `###` section.
     Head unreleased changes with the Cargo.toml version: `-dev`, no date
     (e.g. [v0.32.1-dev]), not [Unreleased]. If the cycle needs a bigger bump,
     rename both. The first PR to change the crate after a release opens the
     next patch `-dev` version and heading; the release PR drops `-dev` and
     adds the date.
     Describe the change since the last release, not each PR: when a PR makes
     an unreleased entry stale, rewrite it in place and append the PR link.
     Never edit released sections.
-->
<!-- markdownlint-disable MD013 -->
# Changelog

## [v0.32.0] - 2026-09-18

### Added

- `--json` works on every command, not just `list` and `status`, so a script or an agent can read any result without parsing tables — `quilt push --json | jq .hash`. Failures are machine-readable too: `{"error": {"kind": "...", "message": "..."}}` on stderr, with a stable `kind` to branch on instead of matching English (<https://github.com/quiltdata/quilt-rs/pull/941>)

### Fixed

- The "you are not logged in" error now tells you to run `quilt login`. It named `quilt_rs`, the library crate, which ships no binary — so the command it handed you could not work (<https://github.com/quiltdata/quilt-rs/pull/943>)

### Changed

- A failure raised before a command runs — an unreadable domain, a rejected flag combination — now prints as a plain message on stderr, the same as any other command failure, instead of a decorated tracing line (`ERROR quilt: Failed to run command: ...`) (<https://github.com/quiltdata/quilt-rs/pull/941>)

### quilt-rs

- Updated [from v0.39.0 to v0.39.1](https://github.com/quiltdata/quilt-rs/compare/quilt-rs/v0.39.0...quilt-rs/v0.39.1) (see [quilt-rs/CHANGELOG.md](../quilt-rs/CHANGELOG.md))

### quilt-uri

- Updated [from v0.4.0 to v0.4.1](https://github.com/quiltdata/quilt-rs/compare/quilt-uri/v0.4.0...quilt-uri/v0.4.1) (see [quilt-uri/CHANGELOG.md](../quilt-uri/CHANGELOG.md))

## [v0.31.2] - 2026-09-15

### Fixed

- `quilt pull` can be retried after an interruption — a dropped connection, an expired credential, a kill, power loss. The apply deleted each path it was about to update before re-fetching it, so an interruption lost those paths from the working tree and from tracking at once, and every retry read that gap as a conflict and refused. A pull now fetches and stages the whole update before it writes anything into the working tree (<https://github.com/quiltdata/quilt-rs/pull/921>)
- `quilt pull` no longer disturbs a process that has a package file open. Each file is staged and renamed into place, so a reader sees one revision's bytes or the other's and never a prefix of either; a process with the file memory-mapped — ordinary for HDF5, Zarr, Arrow and `numpy` `mmap_mode` — used to die on `SIGBUS` (<https://github.com/quiltdata/quilt-rs/pull/921>)
- `quilt pull` no longer reports a conflict on a locally edited file whose content is already byte-identical to the incoming revision (<https://github.com/quiltdata/quilt-rs/pull/921>)

### Security

- The prebuilt binaries take rustls 0.23.45 for [RUSTSEC-2026-0285](https://rustsec.org/advisories/RUSTSEC-2026-0285), where TLS 1.3 handshake messages were accepted across encryption level boundaries. It arrives transitively through the AWS SDK's TLS stack; no manifest here selects it (<https://github.com/quiltdata/quilt-rs/pull/923>)

### quilt-rs

- Updated [from v0.38.0 to v0.39.0](https://github.com/quiltdata/quilt-rs/compare/quilt-rs/v0.38.0...quilt-rs/v0.39.0) (see [quilt-rs/CHANGELOG.md](../quilt-rs/CHANGELOG.md))

## [v0.31.1] - 2026-09-11

### quilt-rs

- Updated [from v0.37.0 to v0.38.0](https://github.com/quiltdata/quilt-rs/compare/quilt-rs/v0.37.0...quilt-rs/v0.38.0) (see [quilt-rs/CHANGELOG.md](../quilt-rs/CHANGELOG.md))

## [v0.31.0] - 2026-09-08

### Added

- `quilt pull` now names the files a revision brought — downloaded, updated, removed, and the ones left on the remote — where it printed the top-hash alone. See [`README.md`](README.md) (<https://github.com/quiltdata/quilt-rs/pull/898>)

### quilt-rs

- Updated [from v0.36.0 to v0.37.0](https://github.com/quiltdata/quilt-rs/compare/quilt-rs/v0.36.0...quilt-rs/v0.37.0) (see [quilt-rs/CHANGELOG.md](../quilt-rs/CHANGELOG.md))

## [v0.30.0] - 2026-09-02

### Added

- `-v` / `--verbose` shows INFO-level logs on stderr; `RUST_LOG` still takes precedence for target-specific filtering (<https://github.com/quiltdata/quilt-rs/pull/847>)
- `quilt log` lists a package's revisions, newest first, with the short hash, the commit message, and the date this copy obtained the revision. That date is what is recorded locally and nothing more: a manifest carries no timestamp of its own, so for a revision fetched from a remote it is the fetch time rather than the commit time — hence the `obtained` heading. A package's full published history is not available yet (<https://github.com/quiltdata/quilt-rs/pull/849>)
- `quilt list --json` and `quilt status --json` print a machine-readable form, so `quilt list --json | jq` works (<https://github.com/quiltdata/quilt-rs/pull/851>)
- `quilt undo-commit` discards the newest commit and restores the one before it. Works until a package's first push, and refuses rather than overwriting anything uncommitted (<https://github.com/quiltdata/quilt-rs/pull/860>)

### Changed

- `quilt list` prints a table of bucket, namespace and status — each bucket named once, spanning its packages' rows — instead of one `InstalledPackage<namespace>` line per package. The status compares commits: your last commit against the last-known remote tip, read from the local lineage record so listing stays offline. It is not the package's overall state — uncommitted edits are invisible to it, so a package with local changes still lists as `up_to_date`. `quilt status <namespace>` reads the working copy and refreshes the tip (<https://github.com/quiltdata/quilt-rs/pull/846>)
- Piping a command's output now works — stdout carries only the command's own output, and logs go to stderr at WARN and above (was INFO on stdout) (<https://github.com/quiltdata/quilt-rs/pull/847>)
- `quilt` no longer needs `--home` on first use — it defaults to `~/QuiltSync`, the same working-copy directory QuiltSync uses. Pass `--home` to choose a different one; it is still remembered after that (<https://github.com/quiltdata/quilt-rs/pull/848>)
- `commit`, `pull`, `push`, `status` and `uninstall` infer the package from the directory you are standing in, so `--namespace` is only needed from outside a package's working copy. Inference reads the two path components below the configured home; anywhere else it is an error naming the flag (<https://github.com/quiltdata/quilt-rs/pull/850>)

### Fixed

- A command that fails now exits non-zero: it used to report the error and exit 0, so a shell script could not tell success from failure
  - <https://github.com/quiltdata/quilt-rs/pull/848>
  - <https://github.com/quiltdata/quilt-rs/pull/851>

### quilt-rs

- Updated [from v0.35.0 to v0.36.0](https://github.com/quiltdata/quilt-rs/compare/quilt-rs/v0.35.0...quilt-rs/v0.36.0) (see [quilt-rs/CHANGELOG.md](../quilt-rs/CHANGELOG.md))

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
