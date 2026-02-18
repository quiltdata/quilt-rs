<!--
     Follow keepachangelog.com format.
     Use GitHub autolinks for PR references.
     Use nested lists when there are multiple PR links.
     Put quilt-rs updates under `### quilt-rs` section.
-->
# Changelog

## [v0.13.1-alpha1] - 2026-02-18

### Fixed

- Fixed deep link handler failing on macOS/Linux
  due to `tauri://` scheme not matching `http` check

## [v0.13.0]

### Changed

- Updated to use quilt-rs v0.27.0 with JSONL manifest format migration

### quilt-rs

- Updated [from v0.26.0 to v0.27.0](https://github.com/quiltdata/quilt-rs/compare/quilt-rs/v0.26.0...quilt-rs/v0.27.0)
  (see [../quilt-rs/CHANGELOG.md#v0.27.0](../quilt-rs/CHANGELOG.md#v0.27.0))
  - Migrated manifest format from Parquet to JSONL for improved performance
    and compatibility

## [v0.12.0]

### Fixed

- Fixed redirect after package pull to avoid 'package already installed' error

### quilt-rs

- Updated [from v0.25.0 to v0.26.0](https://github.com/quiltdata/quilt-rs/compare/quilt-rs/v0.25.0...quilt-rs/v0.26.0)
  (see [../quilt-rs/CHANGELOG.md#v0.26.0](../quilt-rs/CHANGELOG.md#v0.26.0))
  - Fixed commit logic to respect crc64Checksums configuration from host config

## [v0.11.2]

### Fixed

- Fixed Windows deep link navigation issue when app is launched via deep link

## [v0.11.1]

### Changed

- Bumped patch release version to test auto-updater functionality
- Minor dependency updates

## [v0.11.0]

### Added

- Added auto-updater functionality for seamless application updates (#447)

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
  (see [../quilt-rs/CHANGELOG.md#v0.25.0](../quilt-rs/CHANGELOG.md#v0.25.0))
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
  (see [../quilt-rs/CHANGELOG.md#v0.24.0](../quilt-rs/CHANGELOG.md#v0.24.0))
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
