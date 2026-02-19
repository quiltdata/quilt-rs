# Contributing to quilt_rs

This document covers the release process for the quilt_rs Rust library and CLI components.

For testing, development workflows, and other general information, see the main
[Contributing Guide](../CONTRIBUTING.md).

## Release Process

### Creating New Releases

1. **Update the changelog**: Add new section to [CHANGELOG.md](CHANGELOG.md) following
   <https://keepachangelog.com> format with PR links
2. **Bump version**: Update version in workspace root `Cargo.toml` (shared across
   all crates)
3. **Create release**:
   a. **Create and push git tag** (optional):
      `git tag v0.x.x && git push origin v0.x.x`
      This is cosmetic and makes it easier to compare releases, but doesn't affect
      the build process.
   b. **Create release via GitHub Actions**:
   * Go to the Actions tab: <https://github.com/quiltdata/quilt-rs/actions/workflows/release-quilt-rs.yaml>
   * Click "Run workflow" button
   * The workflow will build and publish the library crate to crates.io
4. **Publish release**: Create a GitHub release with the changelog content

The release workflow publishes only the `quilt-rs` library crate to crates.io.
The CLI is not published and users compile it from source.
