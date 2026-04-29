<!--
     Follow keepachangelog.com format.
     Use GitHub autolinks for PR references.
     Use nested lists when there are multiple PR links.
     quilt-uri inherits its version from the workspace, so entries are
     listed directly without per-version headers.
-->
<!-- markdownlint-disable MD013 -->
# Changelog

- [Added] Initial release. String-only, WASM-safe URI types extracted from `quilt-rs` so both the Rust backend and the Leptos UI can share the same parser (<https://github.com/quiltdata/quilt-rs/pull/641>)
- [Changed] Migrated to the Rust 2024 edition; building from source now requires Rust 1.85+ (<https://github.com/quiltdata/quilt-rs/pull/646>)
