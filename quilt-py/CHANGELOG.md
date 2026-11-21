<!--
     Follow keepachangelog.com format, as quilt-rs/CHANGELOG.md does.
     Head unreleased changes with the Cargo.toml version.
-->
<!-- markdownlint-disable MD013 -->
# Changelog

## [v3.0.0-dev]

### Fixed

- An install whose read fails (a `pre-read` hook, or the remote) raises `quilt.QuiltError` instead of returning a dict with `"outcome": "failed"`, so quilt3's `try`/`except` sees it. #1490

### Added

- The `quilt` Python module over quilt-rs's dataspaces: `install` (with `access`, default `sync`), `create` (a build: a new package and its first commit), `commit`, `default_dataspace` (the one chosen for the application folder), `list_registries` and `list_stored`, each blocking on a runtime the module owns with the GIL released; failures raise `quilt.QuiltError`. Wheels are built for macOS (arm64, x86_64), manylinux (x86_64, aarch64) and Windows; nothing is published to PyPI yet. #1490

[v3.0.0-dev]: https://github.com/quiltdata/quilt-rs/tree/main/quilt-py
