<!--
     Follow keepachangelog.com format.
     Use GitHub autolinks for PR references.
     Use nested lists when there are multiple PR links.
     Use alpha pre-release versions (e.g. v0.27.2-alpha1) instead of [Unreleased]
     to keep changelog in sync with Cargo.toml version.
-->
<!-- markdownlint-disable MD013 -->
# Changelog

## [v0.30.1-alpha1] - 2026-04-23

### Changed

- Extract URI types (`Host`, `S3Uri`, `S3PackageUri`, `ManifestUri`, `ObjectUri`, `TagUri`, `Namespace`, `RevisionPointer`, `UriError`) into a new WASM-safe `quilt-uri` crate; `quilt_rs::uri::*` now re-exports them, so call sites keep compiling unchanged.

## [v0.30.0] - 2026-04-22

### Added

- New `flow::publish_package` that composes `commit_package` + `push_package` into a single call, returning a `PublishOutcome` enum (`CommittedAndPushed` / `PushedOnly`) so callers know whether pending changes were committed as part of the publish (<https://github.com/quiltdata/quilt-rs/pull/634>)
- Automatic retry with exponential backoff and request timeouts for HTTP calls, reducing spurious failures on flaky networks (<https://github.com/quiltdata/quilt-rs/pull/630>)

### Changed

- Split the monolithic `Error` enum into focused domain enums wrapped transparently by the top-level `Error` (<https://github.com/quiltdata/quilt-rs/pull/627>)

### Fixed

- Fix `ExpiredToken` 400s on S3 by refreshing credentials per request via a `ProvideCredentials` adapter over `Auth`, with single-flight per-host refreshes so concurrent S3 operations racing past token expiry trigger only one registry mint call (<https://github.com/quiltdata/quilt-rs/pull/631>, <https://github.com/quiltdata/quilt-rs/pull/632>)
- Retry a transient 4xx from the token or credentials endpoint once before concluding `Login required`, and surface the AWS error code, `x-amz-request-id`, and HTTP status in wrapped error messages and logs (<https://github.com/quiltdata/quilt-rs/pull/629>, <https://github.com/quiltdata/quilt-rs/pull/631>)

## [v0.29.0] - 2026-04-16

### Fixed

- Always certify latest on first push, even when the remote already has a different "latest" tag — fixes incorrect "Behind" status after pushing a local package with the same name as an existing remote package

## [v0.28.1] - 2026-04-08

### Changed

- Extract `InstallPackageError` and `InstallPathError` from the monolithic `Error` enum and return `InstallOutcome` instead of erroring when a different version is already installed (<https://github.com/quiltdata/quilt-rs/pull/605>)

## [v0.28.0] - 2026-04-07

### Added

- Support `.quiltignore` files for excluding files from package status and commits (<https://github.com/quiltdata/quilt-rs/pull/589>)
- Detect junk files (OS metadata, editor temps, build artifacts) via the `junk` module and surface them in package status with suggested `.quiltignore` patterns (<https://github.com/quiltdata/quilt-rs/pull/593>)
- Add `flow::create` for creating new local-only packages from scratch, with optional source directory import (<https://github.com/quiltdata/quilt-rs/pull/596>)
- Add `InstalledPackage::set_remote` for configuring remote origin and bucket on local-only packages (<https://github.com/quiltdata/quilt-rs/pull/596>)
- Support first push of local-only packages by skipping remote manifest fetch when hash is empty (<https://github.com/quiltdata/quilt-rs/pull/596>)

### Changed

- Make `PackageLineage.remote` optional to support local-only packages without a remote origin (<https://github.com/quiltdata/quilt-rs/pull/594>)

## [v0.27.4] - 2026-03-19

### Added

- OAuth 2.1 Authorization Code flow with PKCE and Dynamic Client Registration (<https://github.com/quiltdata/quilt-rs/pull/539>)

### Fixed

- Redact secrets (`access_token`, `refresh_token`, `access_key`, `secret_key`, `token`) from debug logs via custom `Debug` impls on `Tokens` and `Credentials` (<https://github.com/quiltdata/quilt-rs/pull/559>)
- Extend secret redaction to `RemoteTokens`, `OAuthTokenResponse`, and `RemoteCredentials` — transient token and credential response types were still leaking secrets via the derived `Debug` impl (<https://github.com/quiltdata/quilt-rs/pull/565>)

### Changed

- `Auth` now holds `Arc<S>` instead of `S`, removing the `Clone` bound on `Storage` and ensuring all `AuthIo` instances share the same storage (<https://github.com/quiltdata/quilt-rs/pull/563>)
- Add blanket `Storage` impl for `Arc<S>` so `AuthIo<Arc<S>>` works without extra wrapping (<https://github.com/quiltdata/quilt-rs/pull/563>)
- Add `AuthError::TokensExchange` variant for failures during authorization code exchange, replacing the misused `TokensRefresh` variant in that path (<https://github.com/quiltdata/quilt-rs/pull/566>)
- Use `finish_non_exhaustive()` in all custom `Debug` impls for secret-bearing types to signal intentional field omission (<https://github.com/quiltdata/quilt-rs/pull/566>)
- Document the single-label prefix assumption in `connect_host`, caller CSRF responsibility in `login_oauth`, and caller ownership of `OAuthParams::client_id` (<https://github.com/quiltdata/quilt-rs/pull/566>)
- Log a clearer message when token refresh fails due to missing OAuth client registration (<https://github.com/quiltdata/quilt-rs/pull/566>)

## [v0.27.3] - 2026-03-09

### Changed

- Remove `read_file` and `write_file` from `Storage` trait
  in favor of `read_byte_stream` and `write_byte_stream`
- Add `StorageExt` trait with `read_bytes` convenience method
- `write_byte_stream` now creates parent directories automatically
- `write_byte_stream` now uses atomic writes (temp file + rename)
  to prevent corruption on interruption

### Removed

- Remove unused `MissingParentPath` error variant

## [v0.27.2] - 2026-03-03

### Added

- Add `UpstreamState::Error` variant for packages that cannot check remote status
  (<https://github.com/quiltdata/quilt-rs/pull/523>)
- Add `InstalledPackage::set_origin()` method to update a package's catalog origin
  (<https://github.com/quiltdata/quilt-rs/pull/523>)

## [v0.27.1] - 2026-02-18

### Added

- Add comprehensive hash reference fixtures and tests for
  manifest combinations (<https://github.com/quiltdata/quilt-rs/pull/482>)

### Changed

- Clean up tests and remove legacy .parquet fixtures following
  manifest format migration (<https://github.com/quiltdata/quilt-rs/pull/477>)
- Add contextual error handling with file paths for better
  debugging of IO operations (<https://github.com/quiltdata/quilt-rs/pull/485>)
- Add comprehensive logging to LocalDomain operations with
  debug/info messages (<https://github.com/quiltdata/quilt-rs/pull/486>)

### Removed

- Removed parquet dependencies from Cargo.toml and leftovers
  from source code (<https://github.com/quiltdata/quilt-rs/pull/480>)

### Fixed

- Re-fetch manifest from remote when cached file is
  unreadable (e.g. legacy Parquet format) (<https://github.com/quiltdata/quilt-rs/pull/492>)

## [v0.27.0](https://github.com/quiltdata/quilt-rs/releases/tag/quilt-rs/v0.27.0) - 2025-02-04

### Changed

- Migrated manifest format from Parquet to JSONL (<https://github.com/quiltdata/quilt-rs/pull/476>)

## [v0.26.0](https://github.com/quiltdata/quilt-rs/releases/tag/quilt-rs/v0.26.0) - 2025-01-26

### Fixed

- Fixed commit logic to respect crc64Checksums configuration
  from host config (<https://github.com/quiltdata/quilt-rs/pull/461>)

## [v0.25.0](https://github.com/quiltdata/quilt-rs/releases/tag/quilt-rs/v0.25.0) - 2025-01-07

### Added

- Support for timestamp tags in package URIs (e.g., `package@1697916638`) (<https://github.com/quiltdata/quilt-rs/pull/429>)

### Changed

- Export `Tag` enum and `LATEST_TAG` constant from uri module (<https://github.com/quiltdata/quilt-rs/pull/429>)

## [v0.24.0](https://github.com/quiltdata/quilt-rs/releases/tag/quilt-rs/v0.24.0) - 2025-12-30

### Fixed

- Fix manifest hash mismatch for packages containing diacritic characters (<https://github.com/quiltdata/quilt-rs/pull/413>)
- Support for `:tag` syntax in package URI parsing with mutual exclusivity
  from `@hash` syntax (<https://github.com/quiltdata/quilt-rs/pull/400>)

## [v0.23.0](https://crates.io/crates/quilt-rs/0.23.0) - 2025-11-28

- Support for creating packages with "CRC64/NVMe" object hash
- Update dependencies including "aws-sdk-rust" monorepo and "arrow/parquet"

## [v0.22.0](https://crates.io/crates/quilt-rs/0.22.0) - 2025-11-13

- Support reading manifests with CRC64/NVMe hash/checksum types

## [v0.21.1](https://crates.io/crates/quilt-rs/0.21.1) - 2025-11-03

- Make `AUTH_DIR` constant public
- Fix typos, write tests

## [v0.21.0](https://crates.io/crates/quilt-rs/0.21.0) - 2025-10-21

- Chores: update dependencies

## [v0.20.0](https://crates.io/crates/quilt-rs/0.20.0) - 2025-10-20

### Changed

- Fix the incorrect Multihash code for SHA256

## [v0.19.0](https://crates.io/crates/quilt-rs/0.19.0) - 2025-04-02

### Changed

- Group errors for local credentials under `AuthError` and specific S3 errors
  under `S3Error`
- Add `Host` or `Option<Host>` for every such error

## [v0.18.0](https://crates.io/crates/quilt-rs/0.18.0) - 2025-04-01

### Refactored

- Replace poor fixtures with better ones

## [v0.17.0](https://crates.io/crates/quilt-rs/0.17.0) - 2025-03-31

### Fixed

- Sort metadata keys recursively

## [v0.16.0](https://crates.io/crates/quilt-rs/0.16.0) - 2025-03-31

### Fixed

- One more fix for optional schema in workflow

## [v0.15.0](https://crates.io/crates/quilt-rs/0.15.0) - 2025-03-31

### Fixed

- Make Metadata Schema optional in workflow
- Schema id is different from workflow id

## [v0.14.0](https://crates.io/crates/quilt-rs/0.14.0) - 2025-03-27

### Fixed

- Sort file entries in manifest by logical key

## [v0.13.0](https://crates.io/crates/quilt-rs/0.13.0) - 2025-03-26

### Fixed

- Fix missing files when pushing modified files

## [v0.12.0](https://crates.io/crates/quilt-rs/0.12.0) - 2025-03-25

### Fixed

- Fix validating hash while pushing installed files
- Fix entry meta

## [v0.11.0](https://crates.io/crates/quilt-rs/0.11.0) - 2025-03-25

### Fixed

- Handle `user_meta: null` and `message: null`

## [v0.10.0](https://crates.io/crates/quilt-rs/0.10.0) - 2025-03-17

### Changed

- Add `"home"` directory in lineage `data.json` and make it required.
  Home directory is a place where to put mutable files. Previously, they were
  stored in the root alongside the `.quilt` directory.

## [v0.9.1](https://crates.io/crates/quilt-rs/0.9.1) - 2025-03-03

### Fixed

- Fix hashing the `user_meta` when caching the package by sorting the keys.
  The bug didn't affect the workflow, because the manifests were written to the
  correct place anyway (by hash derived from the remote location).

### Changed

- Refactor directories scaffolding: paths are now scaffolded before every
  operation, and we imply the file structure is correct during the operation.
- Refactor mocks and fixtures. They are more organized now.

## [v0.9.0](https://crates.io/crates/quilt-rs/0.9.0) - 2025-02-27

Bump a version number to highlight the accumulated changes of the 0.8.\* versions.

### Changed

- Reduce log output when caching the package

## [v0.8.14]

- Add `display_for_host` method for `S3PackageUri`
- Remove default host from `display_for_host`, some host is always required

## [v0.8.13]

- Handle missing keys/values in workflows config
- Add helper function to display catalog URL

## [v0.8.12]

- Added log messages in every "flow"
- Added tests for authentication
- Merged two errors into one `LoginRequired` error using `Option` argument

## [v0.8.11]

- Add authentication to Quilt Stack preserving backward compatibility with
  getting credentials from `~/.aws`
- `domain` path is now required for every command internally, but is optional
  for users
  If `domain` is not provided, the default user data directory is used

## [v0.8.10]

- Fix workflow format by adding `schemas` property
- Security fix: update openssl

## [v0.8.9]

- Remove unnecessary Mutex wrappers from `LocalDomain` and `InstalledPackage`
  structs since file I/O operations already provide synchronization through
  async/await
- Adds new test `test_spamming_commit_writes` in installed_package.rs to verify
  sequential commits work correctly without mutex protection
  `file.flush()` is what fixed the issue in the previous commit, not the Mutex.

## [v0.8.8]

- Added "workflow" parameter for commit
- Increased test coverage to 79%
- Moved HTTPS and AWS S3 clients to the `RemoteS3`, and use `RwLock` there in struct
- Guard lineage with `Mutex` for `InstalledPackage`

## [v0.8.7]

- Throw error if locally committed package and remote have different `top_hash`
- Fix calculating hashes for files <8Mb
- De-duplicate entries when user add files equal to the file that is not
  tracked and is already in a remote manifest

## [v0.8.6]

- Copy package pushed to the remote to the local storage.
  Locally committed package and remote have different `top_hash`, because local
  manifest has `file://` physical keys.

## [v0.8.5]

- `package_s3_prefix` will calculate checksum if missing

## [v0.8.4]

- Handle `&catalog` in `quilt+s3` URI's `.to_string()`

## [v0.8.3]

- Handle `&catalog` in `quilt+s3` URI

## [v0.8.2]

- Chores: update dependencies

## [v0.8.1]

- Test creating manifest with a billion rows via `quilt_rs benchmark` and
  improve performance <https://github.com/quiltdata/quilt-rs/pull/179>
- Use `Row::default_header()` instead of `Row::default()`
  <https://github.com/quiltdata/quilt-rs/pull/182>

## [v0.8.0]

- More docs, move `LocalDomain` and `InstalledPackage` to modules
  <https://github.com/quiltdata/quilt-rs/pull/176>
- Write manifests using Stream, de-couple read from write
  <https://github.com/quiltdata/quilt-rs/pull/175>
- Folders reorganization <https://github.com/quiltdata/quilt-rs/pull/167>
- Use `PathBuf` for paths where possible
  <https://github.com/quiltdata/quilt-rs/pull/165>
- Use `Namespace` struct instead of `String`
  <https://github.com/quiltdata/quilt-rs/pull/166>
- More tests: `install_paths` and `status`, cover more cases of
  `Storage`/`Remote` use <https://github.com/quiltdata/quilt-rs/pull/164>

## [v0.7.0]

- Fix order of JSON in manifest to make checksums constant

## [v0.6.0]

- Make `utils` module private as it only contains helper functions for
  testing. Remove dummy tests.
- Refactor code to introduce tests

## [v0.5.8] - 2024-03-25

- Fixed calculating checksums for new files

## [v0.5.7] - 2024-03-21

## [v0.5.6] - 2024-03-01

- Implement multipart uploads and server-side checksums
- Use sha2-256-chunked hashes for newly added files
- Fix Parquet->JSONL conversion unconditionally setting hash type to SHA256

## [v0.5.5] - 2024-02-29

- Handle sha2-256-chunked hashes in InstalledPackage::status()

## [v0.5.4] - 2024-02-23

- Add support for sha2-256-chunked hashes

## [v0.5.3] - 2024-02-01

- Fix incorrect paths when pushing to S3
- Create package directory even when no paths are installed

## [v0.5.2] - 2024-01-26

- Make Change.{current,previous} properties public
- Fix object directory creation
- Update dependencies

## [v0.5.1] - 2024-01-25

- Add Parquet support
- Remove dead code
- Cleanup tests

## [v0.5.0] - 2023-12-20

- Remove Poem dependency

## [v0.4.4] - 2023-12-20

- Use tracing::info! instead of println!

## [v0.4.3] - 2023-12-19

- Expose quilt for QuiltSync transition

## [v0.4.2] - 2023-12-18

- use aptos-openapi-link for poem serialization
- add back Removed fields (but skip)

## [v0.4.1] - 2023-12-18

### Added

- serde serialization

### Removed

(to enable trivial serialization)

- object_store.Path
- aws.Region
- aws.S3Client
- arrow.RecordBatch
- Multihash

## [v0.4.0] - 2023-12-17

- test get_manifest3
- add tests/example domain
- store shared test data in its own crate
- stub UPath
- implement UriParser and UriQuilt

## [v0.3.2] - 2023-12-14

- get_client
- use client in UPath

## [v0.3.1] - 2023-12-14

- expose quilt4 module

## [v0.3.0] - 2023-12-14

- stub quilt4 module

## [v0.2.0] - 2023-12-13

- export Manifest structs
- add installed_packages
- add manifest_from_uri
- main prints manifest for uri

## [v0.1.1] - 2023-12-13

- Added metadata
- Added CHANGELOG.md
- Updated README

## [v0.1.0] - 2023-12-13

- Initial release
- Imported from Project 4F
- Added Integration Tests
- Removed application code
