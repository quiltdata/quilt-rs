# quilt-rs

## Rust library and CLI for managing Quilt data packages

Library and CLI provide a set of commands for managing data packages,
allowing users to install, commit, push, and pull packages from S3 storage,
as well as browse and track changes in package contents.

It supports operations like installing specific paths from packages,
managing package metadata, and tracking package lineage
with features for viewing status and handling workflows.

## Repository Structure

This is a Cargo workspace containing:

- **`quilt-rs/`** - Library crate (published to [crates.io](https://crates.io/crates/quilt-rs))
- **`quilt-cli/`** - CLI application (compile from source)

## Usage

### Library

```bash
cargo add quilt-rs
```

### CLI

Compile from source:

```bash
cargo build -p quilt-cli --release
./target/release/quilt --help
```

## Contributing

For maintainers and contributors, see [CONTRIBUTING.md](CONTRIBUTING.md) for testing
procedures, release processes, and development workflows.
