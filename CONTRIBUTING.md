# Contributing to quilt-rs

## Testing

### Testing Individual Crates

Since this is a workspace with multiple crates, you can test them independently:

```bash
# Test only the library
cargo test -p quilt-rs

# Test only the CLI
cargo test -p quilt-cli

# Test all crates
cargo test --all
```

### Test Coverage

```bash
cargo install cargo-tarpaulin
cargo tarpaulin --out html
open tarpaulin-report.html
```

### Running Tests

```bash
cargo test
```

## Release Process

### Creating New Releases

1. **Update the changelog**: Add new section to [CHANGELOG.md](CHANGELOG.md) following
   <https://keepachangelog.com> format with PR links
2. **Bump version**: Update version in workspace root `Cargo.toml` (shared across all crates)
3. **Create release**:
   a. **Create and push git tag** (optional):
      `git tag v0.x.x && git push origin v0.x.x`
      This is cosmetic and makes it easier to compare releases, but doesn't affect
      the build process.
   b. **Create release via GitHub Actions**:
      * Go to the Actions tab: <https://github.com/quiltdata/quilt-rs/actions/workflows/release.yaml>
      * Click "Run workflow" button
      * The workflow will build and publish the library crate to crates.io
4. **Publish release**: Create a GitHub release with the changelog content

The release workflow publishes only the `quilt-rs` library crate to crates.io.
The CLI is not published and users compile it from source.

### Version Management

- **Library (`quilt-rs`)**: Versioned and published to crates.io
- **CLI (`quilt-cli`)**: Not published, inherits version from workspace but not released

## Development Workflows

### Building

```bash
# Build entire workspace
cargo build

# Build individual crates
cargo build -p quilt-rs
cargo build -p quilt-cli
```

### Linting and Formatting

```bash
# Check formatting
cargo fmt --check

# Format code
cargo fmt

# Run clippy lints
cargo clippy -- --deny warnings
```

## File Integrity Verification

For debugging and verification purposes:

### SHA256-Chunked Verification

#### 0Mb Files

```bash
sha256sum ./FILE | xxd -r -p | base64
```

#### <= 8Mb Files

```bash
sha256sum ./FILE | xxd -r -p | sha256sum | xxd -r -p | base64
```

#### > 8Mb Files

```bash
split -b 8388608 ./FILE --filter='sha256sum' | xxd -r -p | sha256sum | xxd -r -p | base64
```

### Verify Packages

```bash
split -l 1 ~/MANIFEST.jsonl --filter="jq -cSM 'del(.physical_keys)'" | tr -d '\n' | sha256sum
```

### CRC64/NVMe Verification

**TODO**: CRC64/NVMe checksum verification procedures are not yet documented. This is an area for future contribution and documentation improvements.
