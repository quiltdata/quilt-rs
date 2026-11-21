<!--
     Follow keepachangelog.com format, as quilt-rs/CHANGELOG.md does.
     Head unreleased changes with the Cargo.toml version.
-->
<!-- markdownlint-disable MD013 -->
# Changelog

## [v3.0.0-rc.1] - 2026-11-21

### Changed

- The version is `3.0.0-rc.1` (`3.0.0rc1` on PyPI), the first release candidate of `quilt` 3.0: a pre-release, so `pip install quilt` keeps resolving to Quilt 2 until 3.0.0 ships with the quilt3 major release. #1500
- `create` and `commit` return a `warnings` list: what checking the commit against its bucket's current workflows config found (`inconsistent` or `unchecked`), the commit being recorded either way. #1495

### Fixed

- An install whose read fails (a `pre-read` hook, or the remote) raises `quilt.QuiltError` instead of returning a dict with `"outcome": "failed"`, so quilt3's `try`/`except` sees it. #1490

### Added

- `browse` also returns `base`, the lock's base revision of the package, or `None`. #1501
- `browse` opens a dataspace's package at its last local commit, else its base, returning its manifest as JSON Lines; `resolve_hash` resolves a short top hash in a package's local store; `delete` removes a package, or one revision of it, from every local Package Store. #1498
- `entries` lists the packages a dataspace's contract declares, with their access, folder and paths; `set_package_metadata` replaces the package metadata in a working folder's `metadata.toml` for its next commit to publish. #1499
- The `quilt` Python module over quilt-rs's dataspaces: `install` (with `access`, default `sync`), `create` (a build: a new package and its first commit), `commit`, `default_dataspace` (the one chosen for the application folder), `list_registries` and `list_stored`, each blocking on a runtime the module owns with the GIL released; failures raise `quilt.QuiltError`. Wheels are built for macOS (arm64, x86_64), manylinux (x86_64, aarch64) and Windows; nothing is published to PyPI yet. #1490

[v3.0.0-rc.1]: https://github.com/quiltdata/quilt-rs/tree/main/quilt-py
