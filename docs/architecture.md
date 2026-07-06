# Quilt Architecture Specification

> **Prerequisite reading**: This document assumes familiarity with Quilt's
> [Mental Model](https://docs.quilt.bio/mentalmodel) — packages, manifests,
> logical/physical keys, registries, and the bucket-as-branch workflow are
> introduced there and used here without re-definition.
>
> **Audience**: Contributors seeking a system overview without reading the code,
> and technical stakeholders who need to understand exact workflow behavior.
> Because every file and manifest is identified by its cryptographic hash,
> subtle pipeline differences (byte ordering, JSON canonicalization, line
> endings) produce a different hash and break compatibility — so precise
> knowledge of what happens at each phase matters.
>
> **See also**: [`docs/mental-model.md`](mental-model.md) for the four-hash
> state model (`commit`, `remote.hash`, `base_hash`, `latest_hash`) and the
> `UpstreamState` classifier — content that changes far less often than the
> phase walkthroughs below.

## Overview

Quilt is a data package management system that provides Git-like version control
semantics for data files. Packages can be extremely large (thousands of files,
terabytes of data), so the system is designed for partial downloads and
incremental modifications. It implements content-addressed storage with
immutable objects and supports distributed collaboration through remote storage
backends (primarily S3).

## Mental Model

The Quilt system operates on the principle of **content-addressed storage**
where files are identified by their cryptographic hash rather than their
location. This enables:

- **Immutable objects**: Once created, objects never change
- **Deduplication**: Identical content is stored once regardless of logical paths
- **Integrity verification**: Content can be verified against its hash
- **Distributed collaboration**: Content can be shared across different storage
  locations

## Directory Structure (.quilt)

The `.quilt` directory serves as the local repository for package management:

```text
.quilt/
├── packages/           # Cached manifests from remote storage
│   └── <bucket>/
│       └── <hash>      # Manifest files (downloaded from remote)
├── installed/          # Local package installations
│   └── <namespace>/
│       └── <hash>      # Manifest files (local format)
├── objects/            # Local content-addressed object store
│   └── <sha256>        # Immutable data files
└── data.json           # Package installation and modification tracking
```

### Directory Responsibilities

- **packages/**: Immutable cache of remote manifests, organized by bucket
- **installed/**: Local copies of package manifests, organized by namespace
- **objects/**: Local object store containing actual file content,
  deduplicated by hash
- **data.json**: Tracks package installations, modifications, and commit history

## Domain

A Domain is the top-level envelope for the entire system: a set of namespaces,
packages, and lineage rooted at a single directory. In code, this is represented
by `LocalDomain` (resolving filesystem paths and tracking all installed
packages within the `.quilt` directory).

## Workspace Crate Layout

The workspace splits along an I/O boundary:

- **WASM-safe leaf crates** (`quilt-uri` today): no I/O, compile to
  `wasm32-unknown-unknown`. We expect 1–2 more such extractions —
  likely candidates are checksum / hashing helpers and manifest
  types — but the bar is "clean API and reuse value", not a default
  path for every portable subset.
- **`quilt-rs`**: native-only library; depends on the leaf crates plus
  `aws-sdk-s3`, `tempfile`, `ignore`, `tokio`.
- **Native consumers**: `quilt-cli`, `quilt-sync/src-tauri`.
- **WASM consumer**: `quilt-sync/ui` depends on the leaf crates only.

### Why separate crates, not feature flags

The serious alternative is one crate with target-gated deps and
`#[cfg]`-stripped I/O modules — `aws-sdk-s3` and friends listed under
`[target.'cfg(not(target_arch = "wasm32"))'.dependencies]`. That
compiles on both targets and sidesteps Cargo's feature unification.
Extraction is preferred when a subset has a coherent external API;
small or tightly-coupled subsets stay as `#[cfg]`-gated modules in
`quilt-rs`. The extraction case rests on:

- **Named, discoverable surface.** `quilt-uri` has its own `cargo
  doc`, README, and changelog. With cfg-stripping, the WASM-available
  subset is implicit — a UI author has to grep `#[cfg]` attributes
  across `quilt-rs` to know what compiles where.
- **Write-time discipline.** Crate boundaries make portability
  structural: you cannot import `tokio::fs` into `quilt-uri` because
  it is not in `Cargo.toml`. With cfg-stripping you can import it
  into a portable module and only learn at WASM build time.
- **Independent versioning and reuse.** UI does not churn on
  `quilt-rs` releases that do not touch URI logic; external Rust
  consumers can depend on `quilt-uri` without the rest.

Feature flags remain right for variant *implementations* of one fixed
API (TLS backend, allocator). Use crates when *which* API exists
varies by target; use flags when *how* it is implemented varies by
build.

> Release mechanics (which crates are published, where, and how) live
> in [docs/releases.md](releases.md).

## Core Data Structures

### ManifestRow

```rust
pub struct ManifestRow {
    pub logical_key: PathBuf,    // User-visible file path
    pub physical_key: String,    // Actual storage location (URL)
    pub size: u64,              // File size in bytes
    pub hash: ObjectHash,       // Content hash (SHA256/CRC64/SHA256-Chunked)
    pub meta: Option<serde_json::Value>, // User metadata
}
```

### PackageLineage

```rust
pub struct PackageLineage {
    pub commit: Option<CommitState>,          // Current local commit
    pub remote_uri: Option<ManifestUri>,      // Remote URI; None = local-only
    pub base_hash: String,                    // Hash when package was installed
    pub latest_hash: String,                  // Latest known remote hash
    pub paths: LineagePaths,                  // Tracking of installed files
}
```

### Manifest

A manifest is a collection of ManifestRows that describes a complete package
state. Each row represents a file with:

- **logical_key**: Virtual path inside the package
- **physical_key**: Actual storage location — a URI that can be dereferenced
  to fetch the file's bytes. The URI is treated as read-only: on S3, object
  versioning makes accidental overwrites recoverable; on local filesystems,
  immutability is only a convention enforced by content-addressed naming.
  - `s3://bucket/path` for remote storage (after push)
  - `file:///path/to/local/objects/hash` for local storage (before push)

**Format Notes**:

- Manifests are stored in JSONL format
- All manifests are content-addressed by their top-level hash (`top_hash`)

### Workflow

A **workflow** is a package-level configuration stored as a YAML file in the
remote registry and referenced from the package's manifest header. It defines
how the package must be validated on push (required metadata fields, file
constraints) and supplies default property values when the user does not
specify them. The push and recommit flows resolve the active workflow from
the remote and apply it to the manifest before upload.

## Operational Phases

Phase headings use the public `flow::<name>` paths re-exported from
`quilt-rs/src/flow.rs`. Those short names (`flow::commit`, `flow::push`,
`flow::status`, …) are aliases; the underlying function may have a
longer name (`commit_package`, `push_package`, `create_status`). The
short form is the canonical public path and is what every call site
uses.

### 1. Browse Phase

**Entry Point**: `flow::browse`
**Purpose**: Discover and fetch remote package manifests

```text
User Request: quilt browse s3://bucket/namespace@latest
    ↓
flow::browse(remote_uri)
    ↓
resolve_tag(remote, "latest") → ManifestUri with specific hash
    ↓
cache_remote_manifest(manifest_uri)
    ↓
Download manifest → .quilt/packages/bucket/hash
    ↓
Return: Manifest object for inspection
```

### 2. Install Phase

**Entry Point**: `flow::install_package`
**Purpose**: Register package for local tracking and copy manifest to
installed location

```text
flow::install_package(manifest_uri)
    ↓
Check: Package not already in lineage
    ↓
cache_remote_manifest(manifest_uri) [if not cached]
    ↓
copy_cached_to_installed() → .quilt/installed/namespace/hash
  (copies manifest from packages/ to installed/)
    ↓
resolve_tag("latest") → latest_hash
    ↓
Update data.json:
  - packages[namespace] = PackageLineage
  - base_hash = manifest_uri.hash
  - latest_hash = resolved latest
    ↓
Return: Updated DomainLineage
```

### 3. Install Paths Phase

**Entry Point**: `flow::install_paths`
**Purpose**: Download actual file content to working directory

```text
flow::install_paths(package_lineage, paths_to_install, working_dir)
    ↓
For each path in paths_to_install:
    ↓
  stream_remote_with_installed_rows()
    ↓
  Check if file exists locally in objects/
    ↓
  If missing: download from remote physical_key → objects/hash
    ↓
  Copy objects/hash → working_dir/logical_key (mutable copy)
    ↓
  Update lineage.paths[logical_key] = PathState {
    timestamp: now,
    hash: content_hash
  }
    ↓
Save updated data.json
```

### 4. Create Phase

**Entry Point**: `flow::create`
**Purpose**: Create a new local-only package (alternative to Browse + Install
for remote packages)

```text
flow::create(lineage, paths, storage, namespace, source?, message?)
    ↓
Check: namespace not already in lineage
    ↓
scaffold_for_installing() → create directories
    ↓
If source directory provided:
  walk_source_dir():
    - Scan source recursively (respects .quiltignore)
    - For each file:
      ↓ calculate_hash() → ManifestRow
      ↓ Copy file → objects/content_hash
      ↓ Copy file → package_home/logical_key (working copy)
      ↓ Track in lineage.paths
    ↓
build_manifest_from_rows_stream()
  → .quilt/installed/namespace/hash
    ↓
Create initial CommitState (like `git init` + initial commit)
    ↓
Insert PackageLineage into DomainLineage:
  - commit = initial commit
  - remote_uri = None (local-only)
  - base_hash = "" (no remote)
  - latest_hash = "" (no remote)
    ↓
Return: Updated DomainLineage
```

The resulting package has a clean status (no changes) because the initial
commit captures all files from the source directory. If no source is provided,
an empty package with an empty manifest is created.

### 5. Modification Detection

**Entry Point**: `flow::status`
**Purpose**: Detect changes in working directory compared to installed state

```text
flow::status(lineage, working_dir)
    ↓
Load .quiltignore from working_dir (if present)
    ↓
locate_files_in_package_home():
  - Scan working directory recursively
  - Skip ignored directories (prune entire subtrees) and files
  - Classify each remaining file as: Tracked, NotTracked, New, Removed
    ↓
fingerprint_files():
  - Calculate hash for each file
  - Compare with lineage.paths[file] hash
  - Generate Change enum: Modified, Added, Removed
    ↓
Return: (PackageLineage, InstalledPackageStatus)
```

**Return shape**: the tuple is a remnant — `flow::status` does not
mutate the input lineage, and a TODO in `quilt-rs/src/flow/status.rs`
plans to drop the first element. New callers should treat the lineage
as opaque and discard it. The `InstalledPackageStatus` carries
`upstream_state`, `changes` (a `ChangeSet`), `ignored_files`,
`junky_changes`, and `most_recent_mtime`.

**Interaction with `.quiltignore`**: Ignored files are excluded from the
directory walk. If a previously tracked file matches a new `.quiltignore`
pattern, it will not be found during the walk and will appear as `Removed`.
This is intentional — `.quiltignore` controls which files belong in the
package, so an ignored file should not remain in the manifest. This differs
from `.gitignore`, which does not untrack already-tracked files.

### 6. Commit Phase

**Entry Point**: `flow::commit`
**Purpose**: Create new local package version with changes

```text
flow::commit(lineage, changes, message, user_meta)
    ↓
For each change:
  - Added/Modified: create_immutable_object_copy()
    ↓ Copy working_dir/file → objects/content_hash
    ↓ Update ManifestRow.physical_key = file:///path/to/objects/content_hash
    ↓ Update lineage.paths[file] = new PathState
  - Removed: remove from lineage.paths
    ↓
stream_local_with_changes():
  - Merge original manifest + modifications + new files
  - Sort by logical_key for deterministic ordering
    ↓
build_manifest_from_rows_stream():
  - Create new manifest with updated Header
  - Calculate top-level hash
  - Store → .quilt/installed/namespace/new_hash
    ↓
Update lineage:
  - commit = CommitState { hash: new_top_hash, timestamp, prev_hashes }
  - Save data.json
    ↓
Return: Updated PackageLineage
```

### 7. Set Remote Phase

**Entry Point**: `InstalledPackage::set_remote`
**Purpose**: Connect a local-only package to a remote origin so it can be
pushed

```text
set_remote(origin, bucket)
    ↓
Check: package not already pushed (remote hash must be empty)
    ↓
Set lineage.remote_uri = ManifestUri { origin, bucket, namespace, hash: "" }
    ↓
Persist lineage (remote_uri saved even if recommit fails)
    ↓
If lineage.commit exists:
  flow::recommit(lineage, manifest, host_config, workflow)
    ↓
  Fetch remote's HostConfig (checksum algorithm) and workflow config
    ↓
  rehash_rows():
    For each row in current manifest:
      - If hash algorithm matches remote: pass through unchanged
      - If different: re-hash file from objects/ with remote algorithm
    ↓
  build_manifest_from_rows_stream() → new manifest with updated hashes
    ↓
  Update CommitState:
    - hash = new top hash
    - prev_hashes = [old_hash, ...old_prev_hashes]
    ↓
  Persist updated lineage
    ↓
Return: Package ready to push (status = Ahead)
```

The recommit step ensures the manifest's checksums match what the remote
expects. Without it, push would produce a different top hash than the local
commit, causing a hash mismatch error.

### 8. Push Phase

**Entry Point**: `flow::push`
**Purpose**: Upload local changes to remote storage

```text
flow::push(lineage, local_manifest, remote)
    ↓
If lineage.commit is None: return early — a no-op success
  (certified_latest = true), not an error
    ↓
If remote_uri.hash is empty (first push):
  Use empty Manifest as remote_manifest
Else:
  fetch remote_manifest for comparison
    ↓
stream_uploaded_local_rows():
  For each local row:
    - If identical to remote: reuse remote s3:// physical_key
    - If different/new: upload_row() → new s3:// physical_key
    - Convert file:// physical_keys to s3:// locations
    ↓
build_manifest_from_rows_stream() with uploaded rows
    ↓
upload_manifest() → remote .quilt/packages/bucket/new_hash
    ↓
tag_timestamp() → remote .quilt/named_packages/namespace/timestamp
    ↓
resolve_tag("latest") → latest_hash re-read from the remote
  (the certify decision below uses this fresh value, not the
  possibly-stale one from the last status check)
    ↓
If first push (base_hash empty):
  Set base_hash = new_hash (prevents Diverged status)
    ↓
certify_latest() if any of:
  - first push (base_hash was empty on entry)
  - base_hash == latest_hash (we were tracking the tip)
  - no existing latest tag
    ↓
Update lineage:
  - remote.hash = new_hash
  - commit = None (changes now pushed)
  - latest_hash = updated if certified
    ↓
Return: PushResult { lineage, certified_latest }
  - certified_latest = true  → revision is the new "latest"
  - certified_latest = false → push succeeded but someone else
    pushed in the meantime, so the "latest" tag was not updated
```

**First push always certifies.** `is_first_push` is captured before
`base_hash` is filled in, and it short-circuits the tracking check: the
very first push of a package certifies its revision as `latest`
unconditionally — even when the remote already carries a different
`latest` (the state the classifier reports as `Diverged` when a teammate
published the same namespace first). The rationale recorded in the code:
the user explicitly pushed this version. Only *subsequent* pushes respect
a moved `latest` and return `certified_latest: false` instead.

### 9. Publish Phase

**Entry Point**: `flow::publish`
**Purpose**: Commit any pending working-directory changes and push the
resulting revision to the remote in a single call

```text
flow::publish(lineage, manifest, …, status, namespace, host_config, commit_opts)
    ↓
Classify state:
    (A) status.changes non-empty           → commit + push
    (B) status.changes empty, lineage.commit present
                                            → push only
    (C) no changes and no pending commit   → Err(PackageOpError::Publish)
    ↓
(A) flow::commit(…, commit_opts.message, user_meta, workflow)
    ↓
   reload manifest from disk at the new commit hash so push uploads
   the post-commit rows, not the stale pre-commit manifest
    ↓
(A)/(B) flow::push(…, host_config)
    ↓
Return: PublishOutcome {
          committed,   // true in state (A), false in state (B)
          push,        // PushResult from flow::push
        }
```

The library function is the authoritative place for the combined
behavior. Both desktop (`quilt-sync`) and future consumers (web, CLI)
share this code path instead of re-sequencing commit + push at the
call site.

Error semantics:

- **Commit failure** aborts before any upload is attempted; the caller's
  on-disk lineage is unchanged from the remote's perspective.
- **Push failure** leaves the local commit in place (it was already
  persisted by `commit_package`), so re-running Publish with no further
  changes reaches state (B) and succeeds.

`InstalledPackage::publish` is the method wrapper used by the desktop
layer; it resolves `host_config` from the remote when the caller does
not supply one, mirroring `InstalledPackage::{commit, push}`.

### 10. Certify Latest Phase

**Entry Point**: `InstalledPackage::certify_latest`
(wraps `flow::push` + `flow::certify_latest`)
**Purpose**: Make the local revision the remote's shared `latest`,
uploading the underlying manifest first if it has not been pushed.

**Direction**: local → remote. The flow may upload manifest data
(if there's an unpushed commit) and always rewrites the remote
`named_packages/<ns>/latest` tag.

```text
certify_latest(self)
    ↓
read lineage from .quilt/data.json
    ↓
If lineage.commit is Some:
    flow::push(...)
      → upload manifest + objects to remote
      → set lineage.remote_uri = new manifest URI
      → may internally certify if base == latest; harmless either way
    ↓
re-read lineage (post-push state)
    ↓
flow::certify_latest(lineage, remote, lineage.remote_uri)
    ↓
  tag_latest(remote, manifest_uri)
    → write `s3://bucket/.quilt/named_packages/<ns>/latest`
      containing manifest_uri.hash
    ↓
  lineage.update_latest(manifest_uri)
    → base_hash  = manifest_uri.hash
    → latest_hash = manifest_uri.hash
    ↓
persist lineage
    ↓
Return: updated ManifestUri
```

**Relationship to Push Phase**: Push Phase runs an inline
`flow::certify_latest` when `base_hash == latest_hash` (or there's no
existing latest tag) — see §8. This phase is the user-invoked
composition of the same two primitives, run from the merge page; it
forces certification even when push declined to (remote `latest` has
moved since the user's base).

**Concurrency**: this phase is intentionally last-writer-wins on the
remote `named_packages/<ns>/latest` tag. If another client moves
`latest` between the inner push (when one runs) and the outer
`tag_latest`, the outer call silently overwrites that move. This is
the semantic the merge page asks for — the user explicitly chose
"promote my revision" over the remote — but it means a
non-certifying push and the merge-page action are not symmetric:
push respects a concurrent `latest`; the merge-page action does
not.

### 11. Pull Phase

**Entry Point**: `flow::pull`
**Purpose**: Fast-forward an installed package whose `base_hash` still
matches `remote.hash` to the remote's current `latest`.

**Direction**: remote → local. Pull only succeeds on a clean working
tree with no pending commit; the diverged case is handled by
`Reset Local Phase` (§12), not here.

**Preconditions** (each surfaces as `PackageOpError::Package`):

- `status.changes` is empty (no unstaged edits).
- `lineage.commit` is `None` (no pending commit).
- `remote.hash == base_hash` (otherwise the package has diverged).
- `base_hash != latest_hash` (otherwise nothing to do).

```text
flow::pull(lineage, manifest, paths, storage, remote, home, status, ns)
    ↓
guard: no changes, no pending commit, base == remote.hash, base ≠ latest
    ↓
uninstall_paths()
  → delete working-tree files for the package
  → drop entries from lineage.paths
    ↓
lineage.remote.hash = latest_hash
lineage.base_hash   = latest_hash
    ↓
resolve_tag("latest")      → ManifestUri for the new revision
cache_remote_manifest()    → .quilt/packages/<bucket>/<latest>
copy_cached_to_installed() → .quilt/installed/<ns>/<latest>
    ↓
reload manifest from disk at the new hash
  → required so install_paths reads row hashes from the NEW revision;
    without this, install_paths would copy OLD object bytes over the
    (just-deleted) working-tree files
    ↓
install_paths()
  → for each previously-installed path that still exists in the new
    manifest: re-download object content and re-populate working tree
  → paths that no longer exist in `latest` are logged but not errored
    ↓
Return: updated PackageLineage
```

**Effect on local state**: working-tree files for this namespace are
replaced with bytes from the new revision. The content-addressed
`objects/` cache is not pruned (other packages may share content).
Old local manifests under `.quilt/installed/<ns>/` are not
garbage-collected.

**Cross-reference**: `Reset Local Phase` (§12) is the force-remote-wins
variant — it discards the local commit chain instead of erroring on
divergence.

### Auxiliary: Refresh Latest

**Entry Point**: `flow::refresh_latest_hash` (re-exported from
`quilt-rs/src/flow.rs`).

Resolves the remote `latest` tag for an installed package and writes
the result to `lineage.latest_hash`. Touches no files, no objects, no
working tree — only the lineage. It is the primitive every status
check uses to keep `latest_hash` current.

Autosync's autopull tick is a state-driven dispatcher built on this
primitive: each tick calls `refresh_latest_hash`, classifies via
`UpstreamState`, then routes to `flow::pull` (when `Behind` and the
working tree is clean) or to `flow::publish` (when there are changes
or a pending commit and the working tree is quiet) — both the same
operations the manual UI buttons invoke. `Diverged` pauses the
namespace and surfaces in the UI; resolution is user-action only
(§10 Certify Latest or §12 Reset Local).

`refresh_latest_hash` is the only path that mutates `latest_hash`
without an explicit user action, which is why the classifier in
§"Upstream Tracking" treats `latest_hash` as authoritative for
`Behind`/`Diverged` even though it has no freshness model.

### 12. Reset Local Phase

**Entry Point**: `flow::reset_to_latest`
**Purpose**: Discard all local commits and working-tree state for a
package, replacing them with the remote's current `latest` revision.

**Direction**: remote → local (overwrites the working tree). Local
commits and any uncommitted edits are dropped.

```text
reset_to_latest(lineage, manifest, paths, storage, remote, home, ns)
    ↓
resolve_tag(remote, "latest") → latest ManifestUri
    ↓
If latest.hash == lineage.remote.hash: return unchanged (no-op)
    ↓
flow::uninstall_paths(lineage, home, storage, installed_paths)
  → delete working-tree files for the package
  → drop entries from lineage.paths
    ↓
lineage.base_hash   = latest.hash
lineage.latest_hash = latest.hash
lineage.commit      = None
  → local commit chain is discarded (since #677)
    ↓
cache_remote_manifest(paths, storage, remote, latest)
  → .quilt/packages/<bucket>/<latest.hash>
    ↓
copy_cached_to_installed(...)
  → .quilt/installed/<ns>/<latest.hash>
    ↓
lineage.remote_uri  = Some(latest)
  → remote.hash now matches latest.hash, so a subsequent reset
    short-circuits at the no-op guard above
    ↓
For each path previously installed that still exists in the new
manifest: flow::install_paths(...)
  → re-download object content from remote
  → re-populate working tree
    ↓
Return: updated PackageLineage
```

**Effect on local state**: working-tree files for this namespace are
deleted and re-installed from the remote `latest`. The
content-addressed `objects/` store is not pruned (other packages may
share content). `.quilt/installed/<ns>/` keeps the new manifest;
the old local manifests are not garbage-collected by this flow.

### 13. Uninstall Phase

**Entry Point**: `flow::uninstall_package`
**Purpose**: Remove package from local tracking and delete working directory files

```text
flow::uninstall_package(lineage, paths, storage, namespace)
    ↓
Remove namespace from lineage.packages
    ↓
remove_dir_all(.quilt/installed/namespace/)
  (deletes all installed manifest files for this package)
    ↓
remove_dir_all(home/namespace/)
  (deletes the working directory with user-visible files)
    ↓
NOTE: .quilt/objects/ are NOT removed (shared, may be used by other packages)
NOTE: .quilt/packages/ cache is NOT removed
    ↓
Return: Updated DomainLineage
```

## Key Architectural Patterns

### Content Addressability

- All objects (files and manifests) are identified by cryptographic hash
- Enables deduplication, integrity verification, and immutable references
- Physical storage location can change without affecting logical references

### Lineage Tracking

- `PackageLineage` tracks installation and modification history
- Enables Git-like operations: status, diff, merge conflict detection
- Supports both local commits and remote synchronization

### Stream Processing

- Large manifests processed as streams to handle datasets at scale
- Chunked processing prevents memory exhaustion
- Async/await throughout for non-blocking I/O

### Hash Algorithm Flexibility

Three checksum algorithms are supported, reflecting the historical evolution
of the format:

1. **SHA256** — the original algorithm: a single digest over the entire file.
2. **SHA256-Chunked** — added to speed up large-file uploads. The file is
   split into fixed-size chunks, each chunk is hashed, and the chunk hashes
   are combined into a top digest (aligning with S3 multipart-upload
   boundaries so chunks can be hashed in parallel with the upload).
3. **CRC64** — the current default. AWS computes CRC64 server-side on every
   object, so we can trust S3's checksum rather than re-hashing the file
   client-side.

Each new algorithm became the default for freshly created packages, but all
three remain fully supported for reading existing packages. The `ObjectHash`
enum provides a unified interface across them.

Which algorithm a new upload is *written* with is not a client-side
choice: the remote host's config declares the algorithm it expects, and
the client conforms. This is why `Set Remote Phase` (§7) re-hashes an
existing local commit before the first push — a local-only package was
hashed under the local default, which may differ from what its new
remote requires.

### Network Resilience

Remote I/O goes through a shared layer rather than being inlined per call site:

- **Shared HTTP client**: a single reqwest client with connect and per-request
  timeouts, plus exponential-backoff retries for transient failures. Non-2xx
  responses are logged with status, URL, and a truncated body so failures
  remain diagnosable.
- **Fresh S3 credentials**: every signed request fetches current credentials
  from the Quilt auth backend instead of a cached snapshot, avoiding
  `ExpiredToken` errors in long sessions.
- **Single-flight refresh per host**: when many requests hit expired
  credentials at once, they coalesce onto one refresh per host rather than
  stampeding the auth backend.

## State Management

### Storage: Versioned vs Flat

S3 stores are versioned — each revision of an object gets a `versionId`.
Local stores are flat and simply overwrite objects. When using a flat store,
the registry caches each known version to avoid overwrites.

### Local vs Remote State

- **Local State**: data.json tracks local modifications and commits
- **Remote State**: authoritative package versions in remote storage
- **Synchronization**: push/pull operations reconcile differences

### Package Versioning

- Packages identified by content hash (immutable)
- Tags provide mutable references (latest, timestamp-based)
- Lineage tracks version relationships and history

### Upstream Tracking

```rust
enum UpstreamState {
    UpToDate,   // local == remote
    Ahead,      // local has unpushed commits
    Behind,     // remote has newer version
    Diverged,   // both local and remote have changes
    Local,      // no remote configured, or remote set but never pushed
    Error,      // surfaced when status computation itself fails
}
```

A `Diverged` package leaves that state via one of two remediation
flows: `Certify Latest Phase` (§10) biases local-wins by pushing the
local commit (if any) and tagging it as `latest`; `Reset Local Phase`
(§12) biases remote-wins by discarding the local commit chain and
re-installing the remote `latest`. `Pull Phase` (§11) is the
non-conflict path from `Behind` to `UpToDate` — fast-forward only.

### Resolving Diverged: differences from Git-style merge

The two `Diverged` remediation flows (§10, §12) intentionally do
**not** implement merge in the Git/Mercurial/SVN sense. Users
familiar with those tools should expect the following gaps:

- **No merge operation, no merge commit.** Resolving `Diverged` is a
  binary, package-level choice — Promote (local wins) or Overwrite
  (remote wins) — that produces no node combining the two sides.
  After Promote, the previously-certified remote hash is unreachable
  from any lineage entry; there is no graph node linking the two
  sides of a `Diverged` resolution.
- **No per-file granularity.** "Pick file A from mine, file B from
  theirs" is not expressible. The manifest is the unit of choice,
  not the file.
- **No integrative pull from Diverged.** Pull is fast-forward only.
  From `Diverged`, the only "remote-wins" path is Reset Local (§12),
  which discards the local commit chain — there is no equivalent of
  `git pull --rebase` that would replay local commits on top of
  remote.
- **Promote silently overwrites the remote tag.** A concurrent
  teammate certification is overwritten last-writer-wins (§10
  "Concurrency"); there is no `--force` opt-in gesture.
- **Reset has no reflog.** Git's `reset --hard` is reflog-recoverable
  for ~90 days; Reset Local (§12) has no such safety net. The
  discarded local commit's installed manifest may linger on disk
  under `.quilt/installed/<ns>/<hash>` but is unreachable from any
  lineage state — effectively orphaned garbage.

The design rationale is that data packages are predominantly binary
or large (Parquet, FASTQ, HDF5), where line-level three-way merge is
meaningless. Forcing a whole-manifest choice avoids
"stuck-in-the-middle" partial-merge state and keeps the resolution
model simple, at the cost of any granularity finer than "your
manifest or theirs."

## Error Handling

Custom error types are defined in two locations:

- **`quilt-rs/src/error.rs`** — core library errors, grouped by concern:
  S3 and remote storage, authentication, filesystem I/O, manifest and package
  management, serialization and parsing, workflow operations, lineage and
  domain state, and integrity checking.

- **`quilt-sync/src-tauri/src/error.rs`** — Tauri application errors, including
  UI routing, OAuth, and a `Quilt` variant that wraps the core library errors.

Both use `thiserror` for ergonomic `#[derive(Error)]` definitions.

The core `Error` type also provides `is_not_found()` for classifying S3
`NoSuchKey` responses — used by the push flow to detect a missing `latest`
tag on first push.

## Performance Considerations

- **Deduplication**: Identical content stored once in objects/
- **Object Cache**: Files downloaded once to objects/, then copied to working directory
- **Streaming**: Large manifests processed incrementally
- **Caching**: Remote manifests cached locally in packages/
- **Lazy Operations**: Hash calculations and downloads performed on-demand

## Security Model

- **Content Integrity**: All content verified against cryptographic hashes
- **Immutable Objects**: Objects in the local store are immutable by convention
  (content-addressed naming discourages overwriting, but the filesystem does
  not enforce it)
- **No Credentials in Manifests**: Manifests contain only file metadata (paths,
  hashes, sizes). Authentication is handled externally (AWS credentials, OAuth)
