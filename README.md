# quilt-rs

## Rust library, CLI, and GUI for managing Quilt data packages

Library, CLI, and GUI provide tools for managing data packages,
allowing users to install, commit, push, and pull packages from S3 storage,
as well as browse and track changes in package contents.

It supports operations like installing specific paths from packages,
managing package metadata, and tracking package lineage
with features for viewing status and handling workflows.

## Repository Structure

This is a Cargo workspace containing:

- **`quilt-rs/`** - Library crate (published to [crates.io](https://crates.io/crates/quilt-rs))
- **`quilt-cli/`** - CLI application (compile from source)
- **`quilt-sync/`** - Cross-platform desktop GUI application (QuiltSync)

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

## Documentation

- [Architecture](docs/architecture.md) — system design, data structures, and workflow internals
- [Artifacts](docs/artifacts.md) — file and directory inventory (local and remote)
- [Verification](docs/verification.md) — SHA256-chunked, CRC64/NVMe, and manifest hash recipes
- [Windows Signing](docs/windows-signing.md) — Azure Trusted Signing setup for QuiltSync

## Contributing

For maintainers and contributors, see [CONTRIBUTING.md](CONTRIBUTING.md) for testing
procedures, release processes, and development workflows.
