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
Return: InstalledPackageStatus with ChangeSet
```

**Interaction with `.quiltignore`**: Ignored files are excluded from the
directory walk. If a previously tracked file matches a new `.quiltignore`
pattern, it will not be found during the walk and will appear as `Removed`.
This is intentional — `.quiltignore` controls which files belong in the
package, so an ignored file should not remain in the manifest. This differs
from `.gitignore`, which does not untrack already-tracked files.

### 6. Commit Phase

**Entry Point**: `flow::commit_package`
**Purpose**: Create new local package version with changes

```text
flow::commit_package(lineage, changes, message, user_meta)
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

**Entry Point**: `flow::push_package`
**Purpose**: Upload local changes to remote storage

```text
flow::push_package(lineage, local_manifest, remote)
    ↓
Check: lineage.commit exists (has changes to push)
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
If first push (base_hash empty):
  Set base_hash = new_hash (prevents Diverged status)
    ↓
certify_latest() if base_hash == latest_hash OR no existing latest tag
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

### 9. Uninstall Phase

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
}
```

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
