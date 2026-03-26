# Design Spec: Creating New Packages from Scratch

## Problem

The CLI currently requires an existing remote package as a starting point. All
write operations (`commit`, `push`) gate on `get_installed_package`, which
requires a prior `install`. There is no way to create a new package from local
files alone.

## Goal

Allow a user to initialize a new package from a local directory and push it to
a remote bucket, without pulling anything first.

## Proposed Command

```bash
quilt create --namespace owner/name --remote quilt+s3://bucket [--source /path/to/dir]
```

Followed by the existing `commit` and `push` workflow.

## What Needs to Change

### 1. New `create` CLI subcommand

A new `Commands::Create` variant in `cli.rs` that accepts:

- `--namespace` — the `owner/name` to register locally
- `--remote` — the target S3 URI (`quilt+s3://bucket`) where it will be pushed
- `--source` *(optional)* — a local directory whose contents are copied into the
  package working directory before the flow returns

### 2. New `create` flow in the library

A `flow::create_package` function (analogous to `flow::install_package`) that
bootstraps a `PackageLineage` without pulling a remote manifest. It must:

- Validate the namespace does not already exist in the domain
- Construct an empty installed manifest (no rows)
- Write a synthetic `data.json` entry with:
  - `remote` pointing at the provided bucket/namespace
  - `base_hash` as empty/null (no prior revision)
  - `latest_hash` as empty/null
  - `commit = None` (no local commit yet)
- Create the working directory at `home/namespace/`
- If `--source` is provided, copy its contents (recursively, respecting
  `.quiltignore` if present) into the working directory

### 3. `push` must handle a missing remote base

Today `push` fetches the remote manifest for comparison (`fetch remote_manifest`
in the Push Phase). When there is no prior revision, this step must be skipped
— all rows are treated as new uploads.

### 4. `PackageLineage` null-remote state

`PackageLineage.remote` is currently a `ManifestUri` that always has a hash.
It needs to accommodate a "no prior revision" state (e.g., `Option<ManifestUri>`
or a sentinel). This affects serialization in `data.json` and any code that
dereferences `remote.hash`.

## Tests

### Fixtures (`quilt-cli/src/cli/fixtures/packages/`)

A new fixture module (e.g., `empty_remote`) pointing at a writable test bucket
namespace that has no existing revisions. This is needed by the create→push
integration test.

### `quilt-cli/src/cli/create.rs`

Unit tests covering:

- **`test_create_no_source`** — `create` with no `--source`; assert working
  directory exists and is empty, lineage is written to `data.json`
- **`test_create_with_source`** — `create` with `--source`; assert files from
  the source directory appear in the working directory
- **`test_create_duplicate_namespace`** — `create` on a namespace that is
  already installed returns an error

### `quilt-cli/src/cli/push.rs`

The existing push tests always start from an installed package (non-null remote
base). One new test is needed:

- **`test_push_first_revision`** — calls `create` (no prior remote revision),
  then `commit`, then `push`; asserts that the push succeeds and all rows are
  treated as new uploads (no remote comparison step)

### `quilt-cli/src/cli/cli.rs`

Integration test mirroring the pattern of `test_install` / `test_commit_valid`:

- **`test_create_and_push`** — full end-to-end via `init()`: `create` →
  `commit` → `push`, verifying stdout at each step

## Out of Scope

- Creating packages without a target remote (local-only packages)
- Any UI or Catalog changes

## Workflow After This Change

```bash
quilt create --namespace owner/name --remote quilt+s3://my-bucket
# working directory home/owner/name/ is created, empty

# user copies files in

quilt commit --namespace owner/name --message "Initial commit"
quilt push   --namespace owner/name
```

This mirrors the existing install → modify → commit → push workflow, with
`create` replacing `install`.
