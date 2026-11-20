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

// Create a manifest URI from a package URI with a whole hash
let package_uri = S3PackageUri::try_from(
    "quilt+s3://bucket#package=team/dataset@0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
)?;
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

## Short hashes

A package URI may name a revision by a short hash, as in quilt3:
`quilt+s3://bucket#package=team/dataset@abcdef`, 6 to 63 lowercase hex
characters. Parsing keeps it as written (`RevisionPointer::HashPrefix`) and
never touches the network; `ManifestUri::try_from` refuses it. Use
`io::manifest::resolve_manifest_uri`, which asks the bucket's registry through
`flow::resolve_hash` and returns a `ManifestUri` with the whole hash, so
displaying or storing the result never carries the prefix. A prefix that no
revision starts with, or that several do, is a `HashPrefixError`, as is one
built by hand that is the wrong length or not hex; the parser refuses those
two itself.

A package's own revisions resolve without a remote:
`LocalDomain::resolve_local_hash` (or `flow::resolve_local_hash`, given the
package's manifests folder) matches a prefix against the manifests installed
for that package, with the same rules and errors. Only that package's
revisions count, so a prefix another package shares is not ambiguous.

## Reading an object

`flow::read_object` streams one object's bytes, and checks them against the
row's hash as they arrive. The check is on by default;
`ReadOptions::unverified()` turns it off.

The stream's last item says how a read that fails did so:

- `ChecksumError::HashMismatch`: the bytes do not hash to the row. For a
  `physical_key` with no `versionId` it says the object may have been replaced
  since, as install does; for a pinned version, that the package may be corrupt.
- `ChecksumError::SizeMismatch`: the object is not the size the row records;
  with no `versionId` it also says the object may have been replaced.
- Any other error (a failed read, a hasher that could not run) is not a verdict
  on the bytes.

Bytes from the object store are **not** hash-checked. The store holds files
install verified, and also the copies a commit files of the working files it
hashed, which are not hashed again. So `read_object` only compares a stored
file's length to the row's size; when they differ it ignores the stored file and
reads, and verifies, the remote object instead. A stored file of the right
length but wrong bytes is not caught.

## Without a package home

These calls work with no `InstalledPackage`, lineage or managed working
folder:

- `flow::push_files` uploads rows from files or bytes the caller supplies, and
  publishes the manifest. A row with no hash gets S3's checksum of the host's
  configured type (sha2-256-chunked unless the host enables `crc64Checksums`,
  the preferred direction); a row that already carries a real hash keeps it,
  and the upload is an error unless the bytes hash to it. `latest` moves to
  the new revision only if it still names the revision the push was built on
  when read just before the tag write, or names nothing. The write is
  conditional on that read, so a push that lands between the read and the
  write is not overwritten; a role that may not read `latest` writes it
  unchecked, and
  says so in `PushOutcome::latest_unchecked`. If `latest` has moved past the
  parent, the revision is still saved, `latest` is left alone, and the error
  (`PackageOpError::PushedNotLatest`) carries the pushed top hash. If writing
  a tag fails after the manifest is stored, `PackageOpError::PushTagFailed`
  carries the top hash too.
- `flow::install_to` places a manifest's files in any folder, checking each
  against its row's hash first, and refusing a folder that is a symbolic link
  between the destination and a target. Fetching is all-or-nothing, but the
  renames into place are not: a failure partway leaves the files already
  placed.
- `dataspace::read::read` reads a `data.toml` entry under `read` or `sync`
  into its working folder: the revision's files under the entry's paths, and
  all that is not a file as `metadata.toml`. A file unchanged since the base
  the lock names takes the revision's version, and a local change to a file
  the revision also changed blocks the read before anything is written. Under
  `sync` a file deleted from the folder is a local change like any other, kept
  or colliding, and a missing or empty folder, or a base the bucket no longer
  holds, is refused.
- `dataspace::read_all::read_all` reads every `read` entry of a dataspace,
  then writes `data.lock.toml`, moving a package's base only where its read
  completed. A missing or empty working folder is filled at the revision the
  lock names, where that can still be read whole. A lock written for another
  `data.toml` is used; one written for another folder, host or user only while
  no working folder holds files and it names no unpushed commits.
- `dataspace::sync::sync` ticks every `sync` entry of a dataspace with a
  Domain: it reads `latest` where it moved past the lock's base, commits the
  working folder's changes, deletions included and without what the ignore
  rules leave out of a commit, into the Domain's store for the bucket, and
  pushes the lock's last commit against the base, writing the lock after each
  step. A push `latest` moved past is left stored.
- `dataspace::publish::publish` publishes a `data.toml` entry under `write`
  or `replace` from its working folder, against the `latest` read just before.
  `write` lays the files under the entry's paths over `latest` and keeps its
  other keys, composing again on a `latest` that moves meanwhile; `replace`
  publishes the folder as the whole package. `metadata.toml` sets the message
  and the metadata whole, under the entry's paths for its files and folders,
  and with no such file the metadata is kept. What the ignore rules leave out
  of a commit is not published. On `Base::Locked` it publishes only while
  `latest` is that base, and `Published::Behind` says when it is not. A
  revision that does not become `latest` is still stored, and
  `Published::Stored` carries its hash.
- `dataspace::publish_all::publish_all` publishes every `write` and `replace`
  entry of a dataspace, then writes `data.lock.toml`, moving a package's base
  only where its revision became `latest`. A package also under `read`
  publishes only on its base, read first where `latest` has moved past it.
  A lock written for another folder, host or user is refused.
- `dataspace::reattach::unattached` lists a package's commits in its bucket's
  `RegistryStore` that the bucket does not hold and `data.lock.toml` does not
  name, as after the lock was deleted. `dataspace::reattach::reattach`
  attaches one to the lock on a base the caller names, which the next publish
  expects `latest` to be.

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
