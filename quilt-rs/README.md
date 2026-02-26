# quilt-rs

Rust library for accessing [Quilt](https://www.quiltdata.com) data packages.

Quilt provides Git-like version control semantics for data files through
content-addressed storage with immutable objects and distributed collaboration
via remote storage backends.

## Quick Start

For all operations, instantiate `LocalDomain` and then call some of its methods.

```rust
use std::path::PathBuf;
use quilt_rs::{LocalDomain, uri::{S3PackageUri, ManifestUri}};

# async fn example() -> Result<(), Box<dyn std::error::Error>> {
// Create a local domain for package management
let path = PathBuf::from("/foo/bar");
let local_domain = LocalDomain::new(path);

// Create a manifest URI from a package URI
let package_uri = S3PackageUri::try_from("quilt+s3://bucket#package=namespace@hash")?;
let manifest_uri = ManifestUri::try_from(package_uri)?;

// Install the package
let installed_package = local_domain.install_package(&manifest_uri).await?;
# Ok(())
# }
```

## Workflow

1. **Browse** — discover remote packages (`flow::browse`)
2. **Install** — register package tracking (`flow::install_package`)
3. **Install Paths** — download content to working directory (`flow::install_paths`)
4. **Status** — detect modifications (`flow::status`)
5. **Commit** — create a local package version (`flow::commit_package`)
6. **Push** — upload changes to remote (`flow::push_package`)

## Hash Algorithms

Supports multiple algorithms via `checksum::ObjectHash`:

- **SHA256** — general-purpose cryptographic hash
- **CRC64** — fast checksum for large files
- **SHA256-Chunked** — parallel hashing for very large files

## Further Reading

- [Architecture](../docs/architecture.md) — detailed design, data structures, and workflow
  internals
- [API docs](https://docs.rs/quilt-rs) — full API reference
- [quiltdata.com](https://www.quiltdata.com) — product documentation
