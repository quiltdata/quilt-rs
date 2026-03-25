# Quilt Architecture Specification

> **Audience**: Contributors seeking a system overview without reading the code,
> and technical stakeholders who need to understand exact workflow behavior.
> Quilt identifies every file and manifest by its cryptographic hash — a small
> change in any step can break the computed hash, so precise knowledge of what
> happens at each phase matters.

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
    pub commit: Option<CommitState>,     // Current local commit
    pub remote: ManifestUri,            // Remote package reference
    pub base_hash: String,              // Hash when package was installed
    pub latest_hash: String,            // Latest known remote hash
    pub paths: LineagePaths,            // Tracking of installed files
}
```

### Manifest

A manifest is a collection of ManifestRows that describes a complete package
state. Each row represents a file with:

- **logical_key**: Virtual path inside the package (user-visible file path)
- **physical_key**: Actual storage location URL — a URI that can be
  dereferenced to get a bag of bytes. Physical keys are intended to be
  read-only and immutable (though not enforced). On local filesystems there
  is no versioning; immutability is enforced by content-addressing only.
  - `s3://bucket/path` for remote storage (after push)
  - `file:///path/to/local/objects/hash` for local storage (before push)

**Format Notes**:

- Manifests are stored in JSONL format
- All manifests are content-addressed by their top-level hash

## Complete Workflow

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

### 4. Modification Detection

**Entry Point**: `flow::status`
**Purpose**: Detect changes in working directory compared to installed state

```text
flow::status(lineage, working_dir)
    ↓
locate_files_in_package_home():
  - Scan working directory recursively
  - Classify each file as: Tracked, NotTracked, New, Removed
    ↓
fingerprint_files():
  - Calculate hash for each file
  - Compare with lineage.paths[file] hash
  - Generate Change enum: Modified, Added, Removed
    ↓
Return: InstalledPackageStatus with ChangeSet
```

### 5. Commit Phase

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

### 6. Push Phase

**Entry Point**: `flow::push_package`
**Purpose**: Upload local changes to remote storage

```text
flow::push_package(lineage, local_manifest, remote)
    ↓
Check: lineage.commit exists (has changes to push)
    ↓
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
Optional: certify_latest() if tracking latest
    ↓
Update lineage:
  - remote.hash = new_hash
  - commit = None (changes now pushed)
  - latest_hash = updated if certified
    ↓
Return: Updated PackageLineage
```

### 7. Uninstall Phase

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

- Supports multiple hash algorithms: SHA256, CRC64, SHA256-Chunked
- Algorithm selection based on file size and performance requirements
- `ObjectHash` enum provides unified interface

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
}
```

## Error Handling

Custom error types are scattered across the monorepo but concentrated in two
locations:

- **`quilt-rs/src/error.rs`** — core library errors, grouped by concern:
  - S3 and remote storage (`S3`, `S3Raw`, `RemoteInit`)
  - Authentication (`Auth`, `LoginRequired`)
  - Filesystem I/O (`Io`, `FileRead`, `FileWrite`, `FileCopy`, `FileNotFound`)
  - Manifest and package management (`ManifestHeader`, `ManifestLoad`, `Table`,
    `PackageAlreadyInstalled`, `PackageNotInstalled`)
  - Serialization and parsing (`Json`, `Yaml`, `Utf8`, `UrlParse`)
  - Workflow operations (`Commit`, `Push`, `Uninstall`)
  - Lineage and domain state (`LineageMissing`, `LineageParse`)
  - Integrity (`Checksum`, `ChecksumMissing`)

- **`quilt-sync/src-tauri/src/error.rs`** — Tauri application errors, including
  UI routing, OAuth, and a `Quilt` variant that wraps the core library errors.

Both use `thiserror` for ergonomic `#[derive(Error)]` definitions.

## Performance Considerations

- **Deduplication**: Identical content stored once in objects/
- **Object Cache**: Files downloaded once to objects/, then copied to working directory
- **Streaming**: Large manifests processed incrementally
- **Caching**: Remote manifests cached locally in packages/
- **Lazy Operations**: Hash calculations and downloads performed on-demand

## Security Model

- **Content Integrity**: All content verified against cryptographic hashes
- **Immutable Objects**: Prevents tampering with historical data
- **No Secret Storage**: No credentials or sensitive data in manifests
- **Configurable Backends**: Supports different remote storage authentication
  methods

## Extension Points

- **Storage Backends**: Pluggable storage implementations (S3, local
  filesystem, etc.)
- **Hash Algorithms**: Extensible hash algorithm support
- **Remote Protocols**: Configurable remote storage protocols
- **Metadata Schema**: User-defined metadata in manifests
- **Workflow Integration**: Custom workflow definitions in package headers
