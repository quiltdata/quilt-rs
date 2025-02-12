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
2. Install a package with specific paths
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

- [ ] Install a package without paths (`cli::install::tests::test_valid_command`)
- [ ] Installing multiple paths
- [ ] Installing with custom namespace
- [ ] Installing large packages
- [ ] Installing nested directory structures
- [ ] Re-installing with different paths

#### Invalid:

- [ ] Network failures
- [ ] Invalid URI format (`cli::install::tests::test_invalid_command`)
- [ ] Non-existent package
- [ ] Invalid paths
- [ ] Permission issues (`flow::install::tests::test_installing_when_no_permissions`)
- [ ] Installing with special characters in paths
- [ ] Installing with empty paths list
- [ ] Installing with non-existent paths

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

- [ ] Commit the package with message only, or with a combination of user meta and/or workflow.
  Consider, that workflows config can exist or not exist, and it affects the commit hash.
- [ ] Commit modified files
- [ ] Commit new files
- [ ] Commit file deletions
- [ ] Commit unchanged files (produces same hash)

#### Invalid:

- [ ] Commit package that doesn't exist
- [ ] When workflow ID is provided but no workflow config exists
- [ ] When workflow ID doesn't match configured workflows
- [ ] When workflow config is invalid/malformed
- [ ] When user metadata is not a valid JSON object
- [ ] IO permissions errors
- [ ] Network failures (commit checks the workflows config)
- [ ] Concurrent commit attempts

### Status

The preparation step for the commit. It calculates all the necessary hashes for the files, but does not create a new commit.

### Push

The `push` command uploads committed manifests and files to the remote S3 storage. It reuses existing objects, and tag the remote package as latest if tracking.

#### Options:

1. Push committed changes to remote storage

#### Technical Details:

- Verifies commit exists before pushing
- Copies modified and hashed files from `.quilt/objects/<hash>` to remote S3 storage
- Generates new manifest, but it _must_ stay the same as the local one
- Updates `remote` package lineage
- Tags new version as "latest" if tracking
- Maintains base/latest hash references
- Reuses existing remote objects to minimize data transfer (in other words, do nothing for the files that are not present in local filesystem)

#### Test cases TBD:

##### Valid:

- [ ] Push one commit to remote
- [ ] Push multiple commits to remote
- [ ] Push the package without commits (no-op)
- [ ] Push the package with local changes (pushed only committed changes (?))
- [ ] Push outdated package (will not be tracked as latest)
- [ ] Push with large files
- [ ] Push with many files
- [ ] Push concurrent changes
- [ ] Push to update latest tag (when we made a commit on top of the latest)

##### Invalid:

- [ ] Push package that doesn't exist
- [ ] Push to non-versioned bucket
- [ ] Network failures during push
- [ ] Permission issues
- [ ] Version conflicts (push 1 slowly, then push 2 fast, latest will be 1?)
- [ ] Interrupted pushes

### Pull

The `pull` command downloads the latest version of a package (manifest and installed files) from remote storage. It disallow to pull when there are uncommitted local changes or pending commits.

#### Options:

1. Pull latest version of a package

#### Technical Details:

- Skips pull if already up-to-date
- Verifies no uncommitted local changes exist
- Verifies no pending commits exist
- Verifies remote is tracking (FIXME: I DON'T UNDERSTAND THIS: when `remote.hash` != `base_hash`)
- Updates package to the latest remote version
- Re-installs tracked paths from new version
- Updates local manifest and lineage

#### Test cases TBD:

##### Valid:

- [ ] Pull when behind remote
- [ ] Pull unchanged package (no-op)
- [ ] Pull with tracked paths
- [ ] Pull with removed paths
- [ ] Pull with large files
- [ ] Pull to update latest

##### Invalid:

- [ ] Pull with uncommitted changes
- [ ] Pull with new files (they will be deleted)
- [ ] Pull with pending commits
- [ ] Pull diverged package
- [ ] Pull package that doesn't exist
- [ ] Network failures during pull, or pull interrupted
- [ ] Permission issues

### Reset to latest

The `reset` command forcefully updates a package to match the remote latest version, discarding any local changes or commits.

#### Options:

1. Reset package to the latest remote version

#### Technical Details:

- Verifies remote latest version differs from current
- Removes all local files in working directory
- Re-downloads manifest from remote latest
- Re-installs tracked paths from latest version
- Installs new manifest and update lineage's `remote`

#### Test cases TBD:

##### Valid:

- [ ] Reset when behind remote
- [ ] Reset unchanged package (no-op)
- [ ] Reset with tracked paths
- [ ] Reset with removed paths
- [ ] Reset with local changes
- [ ] Reset with pending commits

##### Invalid:

- [ ] Reset package that doesn't exist (invalid namespace)
- [ ] Network failures during reset
- [ ] Permission issues

### Certify latest

The `certify` command marks a specific package version as the "latest" in remote storage.

#### Options:

1. Certify current version as latest

#### Technical Details:

- Updates remote "latest": put the hash value into the "latest" file
- Updates lineage's `latest_hash` and `base_hash`

#### Test cases TBD:

##### Valid:

- [ ] Certify current version
- [ ] Certify outdated version
- [ ] Certify with concurrent updates
- [ ] Re-certify same version

##### Invalid:

- [ ] Certify package that doesn't exist
- [ ] Network failures
- [ ] Permission issues

### List

The `list` command displays all packages installed in the local domain.

#### Options:

1. List all installed packages

#### Technical Details:

- Reads package information from lineage file `.quilt/data.json`
- Shows package namespaces for all installed packages
- Displays "No installed packages" when domain is empty

#### Test cases TBD:

##### Valid:

- [ ] List empty domain
- [ ] List single installed package
- [ ] List multiple installed packages
- [ ] List after package removal
- [ ] List packages with special characters in names

##### Invalid:

- [ ] List with invalid domain path
- [ ] List with permission issues
- [ ] List with corrupted lineage file

### Browse

The `browse` command displays the contents and metadata of a remote package manifest using `quilt+s3://bucket#package=namespace/name@hash` URI

#### Options:

1. Browse remote package manifest

#### Technical Details:

- Downloads and caches remote manifest locally in `.quilt/packages/<bucket>/<hash>`
- Displays manifest header information (message, user meta, workflow)
- Shows list of files with their locations and sizes
- Supports both Parquet and JSONL manifest formats

#### Test cases TBD:

##### Valid:

- [ ] Browse package with message only
- [ ] Browse package with user metadata
- [ ] Browse package with workflow
- [ ] Browse package with multiple files
- [ ] Browse cached manifest
- [ ] Browse Parquet manifest
- [ ] Browse JSONL manifest

##### Invalid:

- [ ] Browse with invalid URI
- [ ] Browse non-existent package
- [ ] Browse with network failures
- [ ] Browse with permission issues
