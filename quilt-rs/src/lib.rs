//! # Quilt Data Package Management System
//!
//! Quilt provides Git-like version control semantics for data files through content-addressed
//! storage with immutable objects and distributed collaboration via remote storage backends.
//!
//! ## Quick Start
//!
//! For all operations instantiate `LocalDomain` and then call some of its methods.
//!
//! ```rust
//! use std::path::PathBuf;
//! use quilt_rs::{LocalDomain, uri::{S3PackageUri, ManifestUri}};
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Create a local domain for package management
//! let path = PathBuf::from("/foo/bar");
//! let local_domain = LocalDomain::new(path);
//!
//! // Create a manifest URI from a package URI
//! let package_uri = S3PackageUri::try_from("quilt+s3://bucket#package=namespace@hash")?;
//! let manifest_uri = ManifestUri::try_from(package_uri)?;
//!
//! // Install the package
//! let installed_package = local_domain.install_package(&manifest_uri).await?;
//! # Ok(())
//! # }
//! ```
//!
//! ## Architecture Overview
//!
//! ### Content-Addressed Storage
//!
//! Quilt operates on the principle of **content-addressed storage** where files are identified
//! by their cryptographic hash rather than location. This enables:
//!
//! - **Immutable objects**: Once created, objects never change
//! - **Deduplication**: Identical content stored once regardless of logical paths
//! - **Integrity verification**: Content verified against cryptographic hashes
//! - **Distributed collaboration**: Content shared across storage locations
//!
//! ### Directory Structure
//!
//! The `.quilt` directory serves as the local repository:
//!
//! ```text
//! .quilt/
//! ├── packages/           # Cached manifests from remote
//! │   └── <bucket>/<hash>
//! ├── installed/          # Local package installations
//! │   └── <namespace>/<hash>
//! ├── objects/            # Content-addressed object store
//! │   └── <sha256>        # Immutable data files
//! └── lineage.json        # Package tracking and commit history
//! ```
//!
//! ### Key Concepts
//!
//! - **ManifestRow**: Represents a file with `logical_key` (virtual path) and `physical_key` (storage location)
//! - **Manifest**: Collection of ManifestRows describing a complete package state
//! - **PackageLineage**: Tracks installation history, modifications, and commits
//! - **Physical Keys**:
//!   - `file:///path/to/objects/hash` for local storage (before push)
//!   - `s3://bucket/path` for remote storage (after push)
//!
//! ### Workflow Stages
//!
//! 1. **Browse**: Discover remote packages (`flow::browse`)
//! 2. **Install**: Register package tracking (`flow::install_package`)
//! 3. **Install Paths**: Download content to working directory (`flow::install_paths`)
//! 4. **Status**: Detect modifications (`flow::status`)
//! 5. **Commit**: Create local package version (`flow::commit_package`)
//! 6. **Push**: Upload changes to remote (`flow::push_package`)
//!
//! ### Manifest Formats
//!
//! Manifests are stored in JSONL format for both local and remote storage.
//!
//! ## Hash Algorithms
//!
//! Supports multiple algorithms via [`checksum::ObjectHash`]:
//! - **SHA256**: General-purpose cryptographic hash
//! - **CRC64**: Fast checksum for large files
//! - **SHA256-Chunked**: Parallel hashing for very large files
//!
//! Algorithm selection based on file size and performance requirements.
//!
//! ## Error Handling
//!
//! The [`Error`] enum covers all failure modes:
//! - I/O operations, remote storage, manifest parsing
//! - Hash verification, package conflicts
//! - Comprehensive error context for debugging
//!
//! ## Extension Points
//!
//! - **Storage Backends**: Pluggable via [`io::storage::Storage`] trait
//! - **Remote Protocols**: Configurable via [`io::remote::Remote`] trait
//! - **Hash Algorithms**: Extensible [`checksum::ObjectHash`] enum
//! - **Metadata Schema**: User-defined metadata in manifests

pub mod flow;

pub mod auth;
pub mod checksum;
pub mod error;
mod installed_package;
pub mod io;
pub mod lineage;
mod local_domain;
pub mod manifest;
pub mod paths;
pub mod uri;

#[cfg(test)]
pub mod fixtures;

pub use error::Error;
pub use installed_package::InstalledPackage;
pub use local_domain::LocalDomain;

pub type Res<T = ()> = std::result::Result<T, Error>;
