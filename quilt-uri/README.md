# quilt-uri

Parser and types for [Quilt+ URIs](https://docs.quilt.bio/quilt-platform-catalog-user/uri).

WASM-safe leaf crate: pure types and string parsing, no I/O.

## URI types

- `S3PackageUri` — `quilt+s3://bucket#package=prefix/name@hash` or `…:tag`,
  optionally with a path. Used for both packages and files in packages. The
  hash is a whole top hash (64 lowercase hex characters) or, as in quilt3, a
  prefix of 6 to 63 (`RevisionPointer::HashPrefix`). Parsing never asks the
  network which revision a prefix names; `quilt-rs` resolves it against the
  registry, and the resolved address carries the whole hash.
- `S3PackageHandle` — package without a revision (`bucket` + `prefix/name`).
- `ManifestUri` — points to an immutable manifest by hash.
- `TagUri` — points to a tag file (`latest` or a timestamp) that
  references a manifest hash.
- `ObjectUri` — logical-key location of an object inside a package;
  converts to `S3Uri` for fetching bytes.
- `S3Uri` — plain `s3://bucket/key`.

Most of these convert to one another.

## Example

```rust
use quilt_uri::S3PackageUri;

let uri: S3PackageUri = "quilt+s3://my-bucket#package=team/dataset@abc123"
    .parse()
    .unwrap();
```

## Stability

Published on crates.io as a transitive dependency of `quilt-rs`.
Public visibility is a Cargo constraint, not a SemVer commitment —
the API may evolve independently of `quilt-rs`.
