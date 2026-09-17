<!--
     Follow keepachangelog.com format.
     Use GitHub autolinks for PR references.
     Use nested lists when there are multiple PR links.
     Use alpha pre-release versions (e.g. v0.1.1-alpha1) instead of [Unreleased]
     to keep changelog in sync with Cargo.toml version.
-->
<!-- markdownlint-disable MD013 -->
# Changelog

## [v0.4.1-alpha1] - 2026-09-17

### Added

- `Namespace::prefix()` and `Namespace::name()` lend the halves the type already validated. The struct had no `impl` block at all, so a caller wanting either had to split `Display`'s output — the split `try_from` has already done and already rejected a second slash for (<https://github.com/quiltdata/quilt-rs/pull/946>)
- `Namespace` derives `Hash`, so it can key a map. A caller with a namespace per row previously had to key by a string spelling of it (<https://github.com/quiltdata/quilt-rs/pull/946>)

## [v0.4.0] - 2026-07-22

### Changed

- Replaced the test-only `Default` impl for `Host` with `fixtures::host()` behind the `test-support` feature (<https://github.com/quiltdata/quilt-rs/pull/797>)

## [v0.3.0] - 2026-05-06

### Changed

- Renamed `paths::get_manifest_key_legacy` to `paths::get_manifest_key`; the `_legacy` suffix had no non-legacy counterpart to disambiguate from (<https://github.com/quiltdata/quilt-rs/pull/664>)
- `Namespace::try_from` now rejects empty prefixes, empty names, and inputs with extra slashes; existing valid namespaces are unaffected (<https://github.com/quiltdata/quilt-rs/pull/664>)
- `RevisionPointer::Tag` now carries a structured `Tag` instead of a raw string; URIs with arbitrary tag strings (anything other than `latest` or a Unix timestamp) now fail at parse time. Wire format unchanged (<https://github.com/quiltdata/quilt-rs/pull/664>)
- Gated `Default` impls for `ManifestUri`, `S3Uri`, `Namespace`, and `RevisionPointer` behind `#[cfg(test)]` / `feature = "test-support"`; production code that relied on these defaults must use explicit constructors (<https://github.com/quiltdata/quilt-rs/pull/664>)

## [v0.2.0] - 2026-05-04

### Changed

- `TagUri` constructors (`new`, `latest`, `timestamp`) now accept `impl Into<S3PackageHandle>` for a uniform shape, and `TagUri` exposes `From<&TagUri> for S3Uri` and `From<TagUri> for S3PackageHandle` (<https://github.com/quiltdata/quilt-rs/pull/660>)

## [v0.1.0] - 2026-04-29

### Added

- First standalone release on crates.io. WASM-safe URI types extracted from `quilt-rs` so both the Rust backend and the Leptos UI can share the same parser (<https://github.com/quiltdata/quilt-rs/pull/641>)

### Changed

- Migrated to the Rust 2024 edition; building from source now requires Rust 1.85+ (<https://github.com/quiltdata/quilt-rs/pull/646>)
