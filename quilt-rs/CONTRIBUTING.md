# Contributing to quilt-rs

This document covers releasing the crates published to crates.io:
`quilt-uri`, `quilt-rs` and `quilt-cli`.

For testing, development workflows, and other general information, see the main
[Contributing Guide](../CONTRIBUTING.md).

## Release Process

The full procedure, including what each workflow job does, is in
[docs/releases.md](../docs/releases.md). What is specific to these crates:

- **Each crate has its own version**, in its own `Cargo.toml`
  (`quilt-uri/`, `quilt-rs/`, `quilt-cli/`); there is no shared workspace
  version. Each also has its own `CHANGELOG.md`.
- **Release upstream first**: `quilt-uri` → `quilt-rs` → `quilt-cli`.
  `quilt-rs` and `quilt-cli` depend on the crates before them through path
  dependencies with a `version =` specifier, which has to name the upstream
  version being released (and, between releases, its `-dev` version).
- **One workflow for all three**:
  [`release-crate.yaml`](https://github.com/quiltdata/quilt-rs/actions/workflows/release-crate.yaml),
  run manually with the `crate` input. It creates a draft GitHub Release
  tagged `<crate>/v<version>` from the `CHANGELOG.md` section, and after
  approval of the `release-approval` environment publishes the draft and runs
  `cargo publish`. Don't create the tag or the GitHub Release by hand.
- **`quilt-cli` also ships prebuilt binaries** for macOS (x86_64, aarch64) and
  Linux (x86_64-gnu), attached to its draft release before the approval gate,
  for `cargo binstall quilt-cli`.

Before running the workflow, drop `-dev` from the crate's `Cargo.toml`
version and from the top heading of its `CHANGELOG.md`, and add today's date
to that heading.
