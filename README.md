# quilt-rs

Library and CLI provide a set of commands for managing data packages, allowing users to install, commit, push, and pull packages from S3 storage, as well as browse and track changes in package contents.

It supports operations like installing specific paths from packages, managing package metadata, and tracking package lineage with features for viewing status and handling workflows.

## Testing

```bash
cargo test
cargo install cargo-watch
cargo watch # -x test
```

## Publishing

```bash
cargo update
cargo test
cargo publish
```

## Coverage

```bash
cargo install taurpalin
cargo tarpaulin --out html
open tarpaulin-report.html
```

## Update Dependencies

```bash
cargo install cargo-upgrades
cargo upgrades
```

## Verify files integrity

- `sha256sum` calculates SHA256 hash of a file.
- `base64` converts binary data to base64.
- `xxd -r -p` converts HEX produced by SHA256 to binary
- `split -b 8388608` splits file into `8 * 1024 * 1024` bytes

### 0Mb

```bash
sha256sum ./FILE | xxd -r -p | base64
```

### <= 8Mb

```bash
sha256sum ./FILE | xxd -r -p | sha256sum | xxd -r -p | base64
```

### > 8Mb

```bash
split -b 8388608 ./FILE --filter='sha256sum' | xxd -r -p | sha256sum | xxd -r -p | base64
```

## Verify packages

```bash
split -l 1 ~/MANIFEST.jsonl --filter="jq -cSM 'del(.physical_keys)'" | tr -d '\n' | sha256sum
```

## Commands

### Install

Install a package using a `quilt+s3://bucket#package=namespace/name@hash&path=some/file.txt` URI.

#### Options:

1. Install a package without paths
2. Install a pacakge with specific paths
3. Install a package then install paths
4. Install a package specifying a different namespace
5. Re-use existing package when installing the same package

#### Technical details:

- Track a package and installed paths in the lineage file `.quilt/data.json`
- Cache manifests locally under `.quilt/packages/<bucket>/<hash>`
- Install the manifest into `.quilt/installed/<namespace>/<hash>`
- Store immutable files under `.quilt/objects/<hash>`
- Create working copies of files in the package's working directory `<namespace>/<name>`

#### Test cases TBD:

#### Valid workflow:
- [] Installing multiple paths simultaneously
- [] Installing with custom namespace
- [] Installing large packages
- [] Installing nested directory structures
- [] Re-installing with different paths

#### Invalid workflow:
- [] Network failures
- [] Invalid URI format
- [] Non-existent package
- [] Invalid paths
- [] Permission issues
- [] Installing with special characters in paths
- [] Installing with empty paths list
- [] Installing with non-existent paths

### Commit
