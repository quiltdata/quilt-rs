<!--
     Follow keepachangelog.com format.
     Use GitHub autolinks for PR references.
     Use nested lists when there are multiple PR links.
     Put quilt-rs updates under `### quilt-rs` section.
     Use alpha pre-release versions (e.g. v0.13.2-alpha1) instead of [Unreleased]
     to keep changelog in sync with Cargo.toml version.
-->
<!-- markdownlint-disable MD013 -->
# Changelog

## [v0.17.1-alpha3] - 2026-04-24

### Changed

- The "Set remote" popup now validates that the bucket exists on S3 before saving, so a typo fails at save time with a clear "bucket not reachable" message instead of surfacing later as an opaque error during push (<https://github.com/quiltdata/quilt-rs/pull/640>)

### quilt-rs

- Updated [from v0.30.1-alpha1 to v0.30.1-alpha2](https://github.com/quiltdata/quilt-rs/compare/quilt-rs/v0.30.1-alpha1...quilt-rs/v0.30.1-alpha2) (see [quilt-rs/CHANGELOG.md](../quilt-rs/CHANGELOG.md))

## [v0.17.1-alpha2] - 2026-04-23

### Added

- Edit a package's remote from the Installed Package toolbar before push (with current host and bucket pre-filled for in-place correction), or view it read-only as "Show remote" once pushed, since the remote is pinned to the package's lineage after that point (<https://github.com/quiltdata/quilt-rs/pull/640>)

### Changed

- Standardize on "remote" in UI copy — drop "origin" from button labels and status banners (<https://github.com/quiltdata/quilt-rs/pull/640>)

### quilt-rs

- Updated [from v0.30.0 to v0.30.1-alpha1](https://github.com/quiltdata/quilt-rs/compare/quilt-rs/v0.30.0...quilt-rs/v0.30.1-alpha1) (see [quilt-rs/CHANGELOG.md](../quilt-rs/CHANGELOG.md))

## [v0.17.1-alpha1] - 2026-04-23

### Fixed

- Let clicks on the empty area around the notification fall through to the dismiss overlay so notifications can be closed by clicking outside them (<https://github.com/quiltdata/quilt-rs/pull/636>)

## [v0.17.0] - 2026-04-22

### Added

- New `[Commit and Push]` action that commits local changes (if any) and pushes in a single step — available as a one-click button on the Installed Packages List, as a primary CTA on the Commit form alongside the existing `[Commit]`, and on the Installed Package page's bottom action bar alongside `[Create new revision]` (<https://github.com/quiltdata/quilt-rs/pull/634>)
- Commit and Push defaults under Settings: message template (supports `{date}`/`{time}`/`{datetime}`/`{namespace}`/`{changes}` placeholders with a live preview), default workflow picker, and default metadata (<https://github.com/quiltdata/quilt-rs/pull/634>)

### quilt-rs

- Updated [from v0.29.0 to v0.30.0](https://github.com/quiltdata/quilt-rs/compare/quilt-rs/v0.29.0...quilt-rs/v0.30.0) (see [quilt-rs/CHANGELOG.md](../quilt-rs/CHANGELOG.md))
  - New `flow::publish_package` that composes commit + push in a single call
  - Error enum split into focused domain enums
  - Automatic retry with exponential backoff and request timeouts for HTTP calls; `ExpiredToken` S3 errors fixed via per-request credential refresh with single-flight per-host deduplication

## [v0.16.0] - 2026-04-16

### Changed

- Replace Askama templates, TypeScript, and npm with a Leptos WASM client-side rendered frontend (<https://github.com/quiltdata/quilt-rs/pull/606>)
- Reorganize button components into a `buttons` submodule with `ButtonKind` enum, `IconButton` and `ButtonCta` base components, and specific button components for every UI button (<https://github.com/quiltdata/quilt-rs/pull/613>)
- Update logo to Quilt.bio branding (<https://github.com/quiltdata/quilt-rs/pull/612>)
- Disable Commit button on commit page when message is empty (<https://github.com/quiltdata/quilt-rs/pull/619>)
- Highlight Commit link as primary on packages list when package has uncommitted changes (<https://github.com/quiltdata/quilt-rs/pull/619>)
- Disable Pull button with popover when package has uncommitted local changes (<https://github.com/quiltdata/quilt-rs/pull/619>)
- Extract popover CSS into shared `.qui-popover` component (<https://github.com/quiltdata/quilt-rs/pull/619>)
- Render packages list instantly from cached lineage, then refresh status per-package in background (<https://github.com/quiltdata/quilt-rs/pull/622>)

### Fixed

- Fix updater log spam by prioritizing HubSpot endpoint

### quilt-rs

- Updated [from v0.28.1 to v0.29.0](https://github.com/quiltdata/quilt-rs/compare/quilt-rs/v0.28.1...quilt-rs/v0.29.0) (see [quilt-rs/CHANGELOG.md](../quilt-rs/CHANGELOG.md))
  - Always certify latest on first push, fixing incorrect "Behind" status after pushing a local package with the same name as an existing remote package

## [v0.15.1] - 2026-04-08

### Changed

- Show a notification instead of an error when the installed package version differs from the requested one or is local-only (<https://github.com/quiltdata/quilt-rs/pull/605>)

## [v0.15.0] - 2026-04-07

### Added

- Add `.quiltignore` affordance: junk file detection badges, ignore/un-ignore popups, and server-side entry filtering via URL fragment parameters (<https://github.com/quiltdata/quilt-rs/pull/593>)
- Add create local package UI with optional source directory picker and set remote popup for first-push workflow (<https://github.com/quiltdata/quilt-rs/pull/596>)

### Changed

- Show release notes in an in-app popup instead of linking to private GitHub releases page (<https://github.com/quiltdata/quilt-rs/pull/603>)

### quilt-rs

- Updated [from v0.27.4 to v0.28.0](https://github.com/quiltdata/quilt-rs/compare/quilt-rs/v0.27.4...quilt-rs/v0.28.0) (see [quilt-rs/CHANGELOG.md](../quilt-rs/CHANGELOG.md))
  - `.quiltignore` support, junk file detection
  - Local-only package creation and first-push workflow
  - `PackageLineage.remote` is now optional

## [v0.14.5] - 2026-03-25

### Added

- Collect diagnostic logs from the Settings page and send them via Sentry (with zip attachment) or email (<https://github.com/quiltdata/quilt-rs/pull/581>, <https://github.com/quiltdata/quilt-rs/pull/583>)

### Changed

- Replace debug toolbar with a dedicated Settings page accessible from the app bar (<https://github.com/quiltdata/quilt-rs/pull/581>)
- Simplify app initialization: remove `Globals` struct and `AppAssets` trait, use fallible `App::create()` (<https://github.com/quiltdata/quilt-rs/pull/581>)
- Login from a package error state now redirects back to that package instead of the packages list (<https://github.com/quiltdata/quilt-rs/pull/585>)

## [v0.14.4] - 2026-03-19

### Added

- Browser-based OAuth 2.1 login via `quilt://` deep link callback with code-based login as a fallback for stacks that do not support OAuth (<https://github.com/quiltdata/quilt-rs/pull/539>)
- Add `flow` field (`"oauth"` or `"legacy"`) to the `UserLoggedIn` telemetry event to track which login path was used (<https://github.com/quiltdata/quilt-rs/pull/562>)

### Fixed

- Reject unsolicited `quilt://auth/callback` deep links with a clear error instead of falling back to the legacy code-based login (<https://github.com/quiltdata/quilt-rs/pull/570>)
- Show a proper error page instead of an infinite spinner when OAuth login fails, reusing the generic error page with a "Login failed" title (<https://github.com/quiltdata/quilt-rs/pull/569>)
- Fix silent navigation failure after OAuth login: `navigate_after_login` now accepts a typed `routes::Paths` instead of a raw string; on an unexpected redirect value an `error!`-level log is emitted and the user is sent to the default page rather than being left on the login screen (<https://github.com/quiltdata/quilt-rs/pull/568>)
- Return `Err` instead of `Ok(None)` when an OAuth state entry has expired in `take_params`, preventing a timed-out callback from bypassing PKCE+CSRF verification (<https://github.com/quiltdata/quilt-rs/pull/567>)
- Evict expired OAuth state entries (TTL: 10 min) to prevent unbounded memory growth in long-running sessions (<https://github.com/quiltdata/quilt-rs/pull/558>)
- URL-encode host in OAuth redirect URI to handle special characters correctly (<https://github.com/quiltdata/quilt-rs/pull/560>)

### Changed

- Flatten `navigate_after_login` from four nested `match` blocks to early-return style; missing main window now returns `Err(Error::Window)` instead of silently succeeding (<https://github.com/quiltdata/quilt-rs/pull/564>)

### quilt-rs

- Updated [from v0.27.2 to v0.27.4](https://github.com/quiltdata/quilt-rs/compare/quilt-rs/v0.27.2...quilt-rs/v0.27.4) (see [quilt-rs/CHANGELOG.md](../quilt-rs/CHANGELOG.md))
  - OAuth 2.1 Authorization Code flow with PKCE and Dynamic Client Registration
  - Redact secrets (`access_token`, `refresh_token`, etc.) from debug logs via custom `Debug` impls
  - `Auth` now holds `Arc<S>` instead of `S`, removing the `Clone` bound on `Storage`
  - Replace `read_file`/`write_file` in `Storage` with `read_byte_stream`/`write_byte_stream`; writes are now atomic (temp file + rename)

## [v0.14.3] - 2026-03-03

### Fixed

- Replace `window.prompt` with inline form for setting catalog origin,
  fixing broken prompt on macOS in Tauri
  (<https://github.com/quiltdata/quilt-rs/pull/529>)

## [v0.14.2] - 2026-03-03

### Changed

- Show update notification with Download/Dismiss buttons
  instead of auto-installing updates
  (<https://github.com/quiltdata/quilt-rs/pull/520>)
- Gracefully handle packages without catalog origin: show "Set origin" button
  instead of failing, remove bogus open.quilt.bio fallback
  (<https://github.com/quiltdata/quilt-rs/pull/523>)

### quilt-rs

- Updated [from v0.27.1 to v0.27.2](https://github.com/quiltdata/quilt-rs/compare/quilt-rs/v0.27.1...quilt-rs/v0.27.2)
  (see [quilt-rs/CHANGELOG.md](../quilt-rs/CHANGELOG.md))
  - Add `UpstreamState::Error` variant and
    `InstalledPackage::set_origin()` for packages without catalog origin

## [v0.14.1] - 2026-02-26

### Changed

- Upgrade Azure code signing action to v1 (Artifact Signing) (<https://github.com/quiltdata/quilt-rs/pull/513>)

## [v0.14.0] - 2026-02-25

### Added

- Add Windows code signing for release installers (<https://github.com/quiltdata/quilt-rs/pull/484>)

## [v0.13.2] - 2026-02-25

### Added

- Commit page now pre-fills the message field with an
  auto-generated summary of changed files (<https://github.com/quiltdata/quilt-rs/pull/504>)

## [v0.13.1] - 2026-02-19

### Fixed

- Fixed deep link handler failing on macOS/Linux
  due to `tauri://` scheme not matching `http` check (<https://github.com/quiltdata/quilt-rs/pull/491>)

### quilt-rs

- Updated [from v0.27.0 to v0.27.1](https://github.com/quiltdata/quilt-rs/compare/quilt-rs/v0.27.0...quilt-rs/v0.27.1)
  (see [quilt-rs/CHANGELOG.md](../quilt-rs/CHANGELOG.md))
  - Fixed stale Parquet manifest cache preventing app startup (<https://github.com/quiltdata/quilt-rs/pull/492>)

## [v0.13.0]

### Changed

- Updated to use quilt-rs v0.27.0 with JSONL manifest format
  migration (<https://github.com/quiltdata/quilt-rs/pull/476>)

### quilt-rs

- Updated [from v0.26.0 to v0.27.0](https://github.com/quiltdata/quilt-rs/compare/quilt-rs/v0.26.0...quilt-rs/v0.27.0)
  (see [quilt-rs/CHANGELOG.md](../quilt-rs/CHANGELOG.md))
  - Migrated manifest format from Parquet to JSONL for improved performance
    and compatibility

## [v0.12.0]

### Fixed

- Fixed redirect after package pull to avoid
  'package already installed' error (<https://github.com/quiltdata/quilt-rs/pull/459>)

### quilt-rs

- Updated [from v0.25.0 to v0.26.0](https://github.com/quiltdata/quilt-rs/compare/quilt-rs/v0.25.0...quilt-rs/v0.26.0)
  (see [quilt-rs/CHANGELOG.md](../quilt-rs/CHANGELOG.md))
  - Fixed commit logic to respect crc64Checksums configuration from host config

## [v0.11.2]

### Fixed

- Fixed Windows deep link navigation issue when app is launched
  via deep link (<https://github.com/quiltdata/quilt-rs/pull/455>)

## [v0.11.1]

### Changed

- Bumped patch release version to test auto-updater
  functionality (<https://github.com/quiltdata/quilt-rs/pull/454>)
- Minor dependency updates

## [v0.11.0]

### Added

- Added auto-updater functionality for seamless application updates (<https://github.com/quiltdata/quilt-rs/pull/447>)

## [v0.10.0]

This version increment consolidates many small changes from previous patch releases.

### Changed

- Auto-select all checkboxes on page load for file installation (<https://github.com/quiltdata/quilt-rs/pull/437>)
- Add autofocus to commit message input field (<https://github.com/quiltdata/quilt-rs/pull/436>)

### Fixed

- Fixed handling of macOS deep links on first application start (<https://github.com/quiltdata/quilt-rs/pull/433>)

## [v0.9.9](https://github.com/quiltdata/quilt-rs/releases/tag/QuiltSync/v0.9.9) - 2025-01-07

### Fixed

- Handle S3 package URIs with tags that don't have explicit hashes (<https://github.com/quiltdata/quilt-rs/pull/429>)

### quilt-rs

- Updated from [v0.24.0 to v0.25.0](https://github.com/quiltdata/quilt-rs/compare/quilt-rs/v0.24.0...quilt-rs/v0.25.0)
  (see [quilt-rs/CHANGELOG.md](../quilt-rs/CHANGELOG.md))
  - Support for timestamp tags in package URIs
  - Export `Tag` enum and `LATEST_TAG` constant

## [v0.9.8](https://github.com/quiltdata/quilt-rs/releases/tag/QuiltSync/v0.9.8) - 2025-12-30

### Added

- Mixpanel analytics tracking:
  - <https://github.com/quiltdata/QuiltSync/pull/363>
  - <https://github.com/quiltdata/QuiltSync/pull/366>
- More verbose debug logs for failed post-login redirects
  (<https://github.com/quiltdata/QuiltSync/pull/372>)

### Fixed

- Deep link navigation when app hasn't started yet
  (<https://github.com/quiltdata/QuiltSync/pull/372>)
- macOS startup deep link handler - now properly handles deep links on app
  launch (<https://github.com/quiltdata/QuiltSync/pull/394>)
- Better detailed error handling for route parsing
  (<https://github.com/quiltdata/QuiltSync/pull/392>)

### quilt-rs

- Updated from v0.21.1 to [v0.24.0](https://github.com/quiltdata/quilt-rs/releases/tag/quilt-rs%2Fv0.24.0)
  (see [quilt-rs/CHANGELOG.md](../quilt-rs/CHANGELOG.md))
  - Fixed `quilt+s3://` URL parsing with `:tag` syntax
  - Fixed hash mismatch for packages with diacritic characters
  - Support for reading manifest with CRC64/NVMe checksums
  - Support for creating packages with CRC64/NVMe object hashes

### Changed

- Updated macOS build target from macos-13 to macos-15
  (<https://github.com/quiltdata/QuiltSync/pull/393>)
- Can now pass `HostConfig` argument to commands, requesting specific checksums
  (crc64 or sha256)
- Updated GitHub Actions workflows (<https://github.com/quiltdata/QuiltSync/pull/370>)
- Minor dependency updates:
  - <https://github.com/quiltdata/QuiltSync/pull/368>
  - <https://github.com/quiltdata/QuiltSync/pull/373>
  - <https://github.com/quiltdata/QuiltSync/pull/364>
  - <https://github.com/quiltdata/QuiltSync/pull/365>

## [v0.9.7](https://github.com/quiltdata/QuiltSync/releases/tag/v0.9.7) - 2025-11-11

- Add Quilt+S3 URI resolver in the header
- Integrate Sentry error tracker

## [v0.9.6](https://github.com/quiltdata/QuiltSync/releases/tag/v0.9.6) - 2025-11-03

- Improve error handling
- Make writing logs more robust
- Minor updates of dependencies

## [v0.9.5](https://github.com/quiltdata/QuiltSync/releases/tag/v0.9.5) - 2025-10-22

- Update quilt-rs
- Minor updates of dependencies
- Make metadata editor UI more consistent with the other input fields

## [v0.9.4](https://github.com/quiltdata/QuiltSync/releases/tag/v0.9.4) - 2025-04-14

### Fixed

- Show progressbar, make it consistent with the theme
- Reload page after downloading files
- Fixed `parcel` build watcher

## [v0.9.3](https://github.com/quiltdata/QuiltSync/releases/tag/v0.9.3) - 2025-04-10

### Changed

- Migrate from `maud` to `askama` for templating

## [v0.9.2](https://github.com/quiltdata/QuiltSync/releases/tag/v0.9.2) - 2025-04-02

### Added

- Login button on S3 or Auth error

## [v0.9.1](https://github.com/quiltdata/QuiltSync/releases/tag/v0.9.1) - 2025-04-01

- Minor improvements in Metadata editor

## [v0.9.0](https://github.com/quiltdata/QuiltSync/releases/tag/v0.9.0) - 2025-04-01

- Mark the minor version change as it contains many fixes in previous patches

### Changed

- Show buttons "Add object"/"Add array"
- Make metadata editor styles consistent with the rest of the app

## [v0.8.6](https://github.com/quiltdata/QuiltSync/releases/tag/v0.8.6) - 2025-03-31

### Fixed

- quilt_rs: sort keys recursively in metadata

### Changed

- Log files end with `.log` extension
- Don't pre-set commited message

## [v0.8.5](https://github.com/quiltdata/QuiltSync/releases/tag/v0.8.5) - 2025-03-31

- Minor UI improvements
- quilt_rs: Fix serialization/deserialization of the workflow:
  - metadata schema is optional
  - metadata id can be different from workflow id

## [v0.8.4](https://github.com/quiltdata/QuiltSync/releases/tag/v0.8.4) - 2025-03-27

### Fixed

- quilt_rs: Fix sorting files entries in manifest
- Fix checkbox handler for installing specific files
- Pre-set home directory

## [v0.8.3](https://github.com/quiltdata/QuiltSync/releases/tag/v0.8.3) - 2025-03-26

### Fixed

- quilt_rs: Fix uploading modified files during push

### Changed

- Don't open empty package folder if no files were installed

## [v0.8.2](https://github.com/quiltdata/QuiltSync/releases/tag/v0.8.2) - 2025-03-25

### Breaking

- Set Home directory for mutable files first time application starts

### Added

- Open the file with default application if deep link has path

### Changed

- On deep link redirect to installed package if it exists
- Refactor UI: make it wider, move primary buttons to the bottom, add metadata editor

### Fixed

- Update quilt_rs with fixes for hashing `{ "message": null, "user_meta": null }`

## [v0.8.1](https://github.com/quiltdata/QuiltSync/releases/tag/v0.8.1) - 2025-03-03

Update `quilt_rs` to 0.9.1

### Fixed

- Fix wrong hash calculation during caching manifest (won't affect users) via `quilt_rs`

### Added

- Add button to open `.quilt` directory on Error page

## [v0.8.0](https://github.com/quiltdata/QuiltSync/releases/tag/v0.8.0) - 2025-02-27

Consolidate the vast majority of "patch" changes into a single "minor" release.

### Fixed minor bugs

## [v0.7.10](https://github.com/quiltdata/QuiltSync/releases/tag/v0.7.10) - 2025-02-27

### Changed

- Remove `Settings` page: use `&catalog` URI parameter as a source of truth
  for the catalog origin.
- Rotate logs daily and keep last 10 files only

### Fixed

- Fix/remove redirect on failed commit
- Make "workflow" field editable during commit

### Refactored

- Simplify and straighten the JS API

[View full diff](https://github.com/quiltdata/QuiltSync/compare/v0.7.9...v0.7.10)

## [v0.7.9](https://github.com/quiltdata/QuiltSync/compare/v0.7.8...v0.7.9) (2025-02-20)

- Add authentication based on `&catalog` URI parameter
- Add logs in `~/user-data-dir/com.quiltdata.quilt-sync/logs`
- Fixed duplication of deep link handling

## [v0.7.8](https://github.com/quiltdata/QuiltSync/compare/v0.7.7...v0.7.8) (2025-02-07)

- Update `quilt_rs`
- Fix aarch builds

## [v0.7.7](https://github.com/quiltdata/QuiltSync/compare/v0.7.6...v0.7.7) (2025-01-24)

- Fix duplication of commands
- Rewrite frontend: `htmx` → `typescript`
- Fix linux builds

## [v0.7.6](https://github.com/quiltdata/QuiltSync/compare/v0.7.5...v0.7.6) (2025-01-20)

## [v0.7.5](https://github.com/quiltdata/QuiltSync/compare/v0.7.3...v0.7.5) (2025-01-02)

- Update quilt-rs with `&catalog` URI parameter support
- Fix Github release workflow

## [v0.7.3](https://github.com/quiltdata/QuiltSync/compare/v0.7.2...v0.7.3) (2024-06-20)

- Update quilt-rs
- Minor updates of dependencies

## [v0.7.2](https://github.com/quiltdata/QuiltSync/compare/v0.7.1...v0.7.2) (2024-05-13)

- Update quilt-rs
- Minor updates of dependencies

## [v0.7.1](https://github.com/quiltdata/QuiltSync/compare/v0.7.0...v0.7.1) (2024-04-25)

- Use `macos-12` for Intel builds, and `macos-latest` (14) for ARM builds

## [v0.7.0](https://github.com/quiltdata/QuiltSync/compare/v0.6.14...v0.7.0) (2024-04-25)

- Update quilt-rs (v0.7.0) with fixed checksums

## [v0.6.14](https://github.com/quiltdata/QuiltSync/compare/v0.6.13...v0.6.14) (2024-04-24)

- Update quilt-rs with major refactoring <https://github.com/quiltdata/QuiltSync/pull/123>
- Minor update of dependencies

## [v0.6.13](https://github.com/quiltdata/QuiltSync/compare/v0.6.12...v0.6.13) (2024-03-25)

- <https://github.com/quiltdata/QuiltSync/pull/116>
  - Fix JS-imports
  - Fix calculating checksums by updating quilt-rs

## [v0.6.12](https://github.com/quiltdata/QuiltSync/compare/v0.6.11...v0.6.12) (2024-03-05)

- Update minor deps
  - <https://github.com/quiltdata/QuiltSync/pull/103>
  - <https://github.com/quiltdata/QuiltSync/pull/104>
- Fix build crash by removing tauri-cli installation with `postInstall` <https://github.com/quiltdata/QuiltSync/pull/105>

## [v0.6.11](https://github.com/quiltdata/QuiltSync/compare/v0.6.10...v0.6.11) (2024-03-04)

- Update quilt_rs to 0.5.6
  - <https://github.com/quiltdata/QuiltSync/pull/99>
  - <https://github.com/quiltdata/QuiltSync/pull/101>
  - Chunksums support and fixes
- Bugfixes, improvements, and refactoring
  - <https://github.com/quiltdata/QuiltSync/pull/100>
  - <https://github.com/quiltdata/QuiltSync/pull/102>
  - Offline support

## [v0.6.10](https://github.com/quiltdata/QuiltSync/compare/v0.6.9...v0.6.10) (2024-02-24)

- <https://github.com/quiltdata/QuiltSync/pull/98> Bugfixes
  - Fix inability to select checkboxes outside viewport
  - Don't throw exception when Quilt URI path is directory
  - Improve scroll UX
  - Move Commit and Merge pages under installed package breadcrumb
  - Hide "Select all" when nothing to install

## [v0.6.9](https://github.com/quiltdata/QuiltSync/compare/v0.6.8...v0.6.9) (2024-02-23)

- <https://github.com/quiltdata/QuiltSync/pull/97> Refactor UI action buttons
- <https://github.com/quiltdata/QuiltSync/pull/96> Update `quilt-rs` with new
  S3 checksums support

## [v0.6.8](https://github.com/quiltdata/QuiltSync/compare/v0.6.7...v0.6.8) (2024-02-22)

- <https://github.com/quiltdata/QuiltSync/pull/94>
  - Show "disabled" and "in-progress" states
  - Fix scrolling entries list and improve checkbox layout
  - Use warning/error colors

## [v0.6.7](https://github.com/quiltdata/QuiltSync/compare/v0.6.6...v0.6.7) (2024-02-21)

- <https://github.com/quiltdata/QuiltSync/pull/93>
  - Use inline importmaps to please Microsoft Edge
  - Improve installing paths UX and edge cases

## [v0.6.6](https://github.com/quiltdata/QuiltSync/compare/v0.6.5...v0.6.6) (2024-02-20)

- <https://github.com/quiltdata/QuiltSync/pull/92> Select and bulk install paths
- <https://github.com/quiltdata/QuiltSync/pull/86> Show deleted files
- <https://github.com/quiltdata/QuiltSync/pull/91>
  - Hide additional actions in menu: for app toolbar, and file entry
  - Enable DevTools
- <https://github.com/quiltdata/QuiltSync/pull/90> Notarize Mac builds

## [0.6.5](https://github.com/quiltdata/QuiltSync/compare/v0.6.4...v0.6.5) (2024-02-16)

- <https://github.com/quiltdata/QuiltSync/pull/82>: Add "Open Catalog" link
  to the installed packages list page
- <https://github.com/quiltdata/QuiltSync/pull/85> Sign Mac apps

## [0.6.4](https://github.com/quiltdata/QuiltSync/compare/v0.6.3...v0.6.4) (2024-02-15)

- <https://github.com/quiltdata/QuiltSync/pull/81>:
  - Increase tests coverage
  - Bugfix: prevent submitting form on Enter

## [0.6.3](https://github.com/quiltdata/QuiltSync/compare/v0.6.2...v0.6.3) (2024-02-14)

- <https://github.com/quiltdata/QuiltSync/pull/80> Increase tests coverage,
  remove temp_dir and manage to not touch real I/O in unit tests

## [0.6.2](https://github.com/quiltdata/QuiltSync/compare/v0.6.1...v0.6.2) (2024-02-13)

- <https://github.com/quiltdata/QuiltSync/pull/70> Update crate `tokio` to 1.36.0
- <https://github.com/quiltdata/QuiltSync/pull/79> Update "Golden path" to CONTRIBUTING.md

## [0.6.1](https://github.com/quiltdata/QuiltSync/compare/v0.6.0...v0.6.1) (2024-02-13)

- <https://github.com/quiltdata/QuiltSync/pull/76>
- <https://github.com/quiltdata/QuiltSync/pull/77>
- <https://github.com/quiltdata/QuiltSync/pull/78>

## [0.6.0](https://github.com/quiltdata/QuiltSync/compare/v0.5.7...v0.6.0) (2024-02-09)

- <https://github.com/quiltdata/QuiltSync/pull/75>

## [0.5.7](https://github.com/quiltdata/QuiltSync/compare/v0.5.6...v0.5.7) (2024-02-08)

## [0.5.6](https://github.com/quiltdata/QuiltSync/compare/v0.5.4...v0.5.6) (2024-02-08)

## [0.5.4](https://github.com/quiltdata/QuiltSync/compare/v0.5.3...v0.5.4) (2024-02-02)

## [0.5.3](https://github.com/quiltdata/QuiltSync/compare/v0.5.2...v0.5.3) (2024-01-30)

## [0.5.2](https://github.com/quiltdata/QuiltSync/compare/v0.5.1...v0.5.2) (2024-01-26)

## [0.5.1](https://github.com/quiltdata/QuiltSync/compare/v0.5.0...v0.5.1) (2024-01-23)

## [0.5.0](https://github.com/quiltdata/QuiltSync/compare/v0.4.15...v0.5.0) (2024-01-25)

## [0.4.15](https://github.com/quiltdata/QuiltSync/compare/v0.4.0...v0.4.15) (2024-02-18)

## [0.4.0] - (2023-12-21)

- Pure HTMX
- Mockup UI in HTML
- Add Styling

## [0.3.x] - (2023-12-19)

- Add Git Tag
- Fix release workflow
- Set identifier; with "-" instead of "\_"
- Update Tauri version / non-Draft release

## [0.3.0] - (2023-12-19)

- Add lib, integration tests
- Replace direct JavaScript with HTMX

## [0.2.0] - (2023-12-19)

- create-tauri-app
- Quilt icons
- Detailed README

## [0.1.0] - (2023-12-19)

- cargo init
- CHANGELOG
- GitHub Actions
- Dependent crates

### Changed

- Internal name to quilt_sync (snake_case)
