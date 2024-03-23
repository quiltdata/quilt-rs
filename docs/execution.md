# Quilt Execution Flow

Unlike Python, Rust prefers to have immutable objects and not share mutable state.  In addition, there is no Rust equivalent of `universal_pathlib` in Python -- object_store and open_dal are not yet mature -- so we need to manage our own storage.

## Architecture

To address these concerns, we propose the following execution architecture.

1. Commmand: a user-level activity, such as `quilt install` or `quilt push`.  It is a high-level construct that is composed of multiple Tasks.
1. Operation: a low-level activity that interacts with the storage layer using UPaths.
1. UPath: an Enum for representing an object or prefix in any supported storage layer (currently filesystems and S3, plus Mock).
1. Storage: a trait that defines the interface for interacting with storage layers.

## Example

## Syntactic Operations

These do not perform I/O operations.

```rust
let pkg_name: PackageName = "foo/bar";
let remote: Domain = "s3://my-bucket";
let revision = Some::latest();
let ref = PackageRef::new(pkg_name, remote, revision);
let cmd: Command = rev.cmd_browse();
let local = Domain::tmp();
```

## Storage Operations

This performs I/O operations.

```rust
let result = local.execute(cmd).await?;
```

The `browse` command translates a `PackageRef` into an in-memory `Package` object, and stores it in the local cache.

1. Construct the remote revision path for `latest`
2. Use remote Storage to read the hash value
3. Check local Storage to see if that hash is already present (if so, return that)
4. Construct the manifest path for that hash
5. Use remote Storage to read the manifest
6. Construct the Package
7. Cache it in local Storage

## Open Questions

1. Is Storage a single pure function?  Or a set of methods on UPath? Or does it manage concurrency?
2. Do we need to lock Storage? One lock per Domain?
3. Should 'Domain' contain a mutable cache? Or should it store all its state in the filesystem, and read it in each time?
4. Should `execute` be on the Domain object, since it typically needs to update both Contents and Registry?