# quilt-rs

Rust library for accessing [Quilt](https://www.quiltdata.com) data packages.

Quilt provides Git-like version control semantics for data files through
content-addressed storage with immutable objects and distributed collaboration
via remote storage backends.

## Quick Start

For all operations, instantiate `LocalDomain` and then call some of its methods.

```rust
use std::path::PathBuf;
use quilt_rs::LocalDomain;
use quilt_uri::{S3PackageUri, ManifestUri};

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

## Without a package home

Two calls work from a manifest alone, with no `InstalledPackage`, lineage or
managed working folder:

- `flow::push_files` uploads rows from files or bytes the caller supplies, and
  publishes the manifest. A row with no hash gets S3's checksum of the host's
  configured type; a row that already carries a real hash keeps it, and the
  upload is an error unless the bytes hash to it. `latest` moves to the new
  revision only if it still names the revision the push was built on when read
  just before the tag write, or names nothing. That read and the write are two
  requests, not one atomic step, so a push that lands between them can still be
  overwritten. If `latest` has moved past the parent, the revision is still
  saved, `latest` is left alone, and the error (`PackageOpError::PushedNotLatest`)
  carries the pushed top hash. If writing a tag fails after the manifest is
  stored, `PackageOpError::PushTagFailed` carries the top hash too.
- `flow::install_to` places a manifest's files in any folder, checking each
  against its row's hash first. Fetching is all-or-nothing, but the renames
  into place are not: a failure partway leaves the files already placed.

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
