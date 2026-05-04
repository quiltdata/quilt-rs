<!--
     Follow keepachangelog.com format.
     Use GitHub autolinks for PR references.
     Use nested lists when there are multiple PR links.
     Use alpha pre-release versions (e.g. v0.1.1-alpha1) instead of [Unreleased]
     to keep changelog in sync with Cargo.toml version.
-->
<!-- markdownlint-disable MD013 -->
# Changelog

## [v0.2.0] - 2026-05-04

### Changed

- `TagUri` constructors (`new`, `latest`, `timestamp`) now accept `impl Into<S3PackageHandle>` for a uniform shape, and `TagUri` exposes `From<&TagUri> for S3Uri` and `From<TagUri> for S3PackageHandle` (<https://github.com/quiltdata/quilt-rs/pull/660>)

## [v0.1.0] - 2026-04-29

### Added

- First standalone release on crates.io. WASM-safe URI types extracted from `quilt-rs` so both the Rust backend and the Leptos UI can share the same parser (<https://github.com/quiltdata/quilt-rs/pull/641>)

### Changed

- Migrated to the Rust 2024 edition; building from source now requires Rust 1.85+ (<https://github.com/quiltdata/quilt-rs/pull/646>)
