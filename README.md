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

#### Valid:

- [] Installing multiple paths simultaneously
- [] Installing with custom namespace
- [] Installing large packages
- [] Installing nested directory structures
- [] Re-installing with different paths

#### Invalid:

- [] Network failures
- [] Invalid URI format
- [] Non-existent package
- [] Invalid paths
- [] Permission issues
- [] Installing with special characters in paths
- [] Installing with empty paths list
- [] Installing with non-existent paths

### Commit

The `commit` command creates a new revision of an installed package by capturing changes to tracked files along with metadata.

#### Options:

1. Commit the package with message only
2. Commit the package with message and metadata
3. Commit the package with message and workflow
4. Commit the package with message, metadata and workflow

#### Technical Details:

- Generates hashes for each file and copy files into `.quilt/objects/<hash>`
- Generate hash for the commit and store manifest in `.quilt/installed/<namespace>/<hash>`
- Tracks the latest commit in the lineage file `.quilt/data.json`
- Tracks the list of local commits (hashes only) in `.quilt/data.json`
- Handles unchanged files by reusing previous hashes

#### Test cases TBD:

#### Valid:

- [] Commit the package with message only, or with a combination of user meta and/or workflow.
  Consider, that workflows config can exist or not exist, and it affects the commit hash.
- [] Commit modified files
- [] Commit new files
- [] Commit file deletions
- [] Commit unchanged files (produces same hash)

#### Invalid:

- [] Commit package that doesn't exist
- [] When workflow ID is provided but no workflow config exists
- [] When workflow ID doesn't match configured workflows
- [] When workflow config is invalid/malformed
- [] When user metadata is not a valid JSON object
- [] IO permissions errors
- [] Network failures (commit checks the workflows config)
- [] Concurrent commit attempts

### Status

The preparation step for the commit. It calculates all the necessary hashes for the files, but does not create a new commit.

### Push

The `push` command uploads committed manifests and files to the remote S3 storage. It reuses existing objects, and tag the remote package as latest if tracking.

#### Options:

1. Push committed changes to remote storage

#### Technical Details:

- Verifies commit exists before pushing
- Copies modified and hashed files from `.quilt/objects/<hash>` to remote S3 storage
- Generates new manifest, but it _must_ stays the same as the local one
- Updates `remote` package lineage
- Tags new version as "latest" if tracking
- Maintains base/latest hash references
- Reuses existing remote objects to minimize data transfer (in other words, do nothing for the files that are not present in local filesystem)

#### Test cases TBD:

##### Valid:

- [] Push one commit to remote
- [] Push multiple commits to remote
- [] Push the package without commits (no-op)
- [] Push the package with local changes (pushed only committed changes (?))
- [] Push outdated package (will not be tracked as latest)
- [] Push with large files
- [] Push with many files
- [] Push concurrent changes
- [] Push to update latest tag (when we made a commit on top of the latest)

##### Invalid:

- [] Push package that doesn't exist
- [] Push to non-versioned bucket
- [] Network failures during push
- [] Permission issues
- [] Version conflicts (push 1 slowly, then push 2 fast, latest will be 1?))
- [] Interrupted pushes
