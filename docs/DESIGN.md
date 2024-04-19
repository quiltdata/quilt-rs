# quilt-rs Design Overview

## I. Introduction

The primary purpose of quilt-rs is to provide a simple and efficient Rust API for managing Quilt packages and registries.

In particular, we need objects that represent both on-disk data structures and the operations that can be performed on them. Importantly, we ultimately want the ability to transparently support multiple storage backends (local filesystem, S3, mock, etc.) and registry types (i.e., flat vs versioned).

This document captures the current "as implemented" architecture, and identifies the key areas that may need to be refactored to meet the above requirements.

## II. Key Concepts

### II.A. Packages

Quilt provides a universal data abstraction layer for managing "data packages", immutable, self-describing data containers whose cryptographically secure checksum acts a unique identifier.
Crucially, each Package is a logical collection that abstracts away the physical location of the data, allowing users to interact with the data consistently, regardless of where and how it is stored.

### II.B. Manifests

Manifests are the primary data structure used to define a Quilt package. They contain:

- standard `info` about the package: version, commit message
- optional package-level user-specified `meta` data
- one `Entry` for each object "contained" in the package

### II.C. Entries

Each Entry in a manifest represents a single object in the package. It contains:

- name: the logical name of the object
- place: the physical URI of the object
- hash: the hash of the object (as a `MultiHash`)
- size: the size of the object in bytes
- info: optional system-generated metadata
- meta: optional user-specified metadata

### II.D. Registries

Registries are special folders within a storage system that contain Manifests,
and associate them with the Namespaces used to identify packages.
They may also contain configuration information for systems that work with packages.

### II.E. Domains

A Domain is a location (e.g., an S3 bucket or local folder) that contains both a Registry
and a Store for the actual data objects.

### II.F. Stores

Each Store may be Versioned or Flat. Versioned Stores assign a `versionId` to reach revision of an object, while Flat Stores simply overwrite them.  Currently, the system assumes S3 stores are versioned, while local stores are flat.  When using a flat store, the Registry caches each known version of the object to avoid overwrites.

### II.G. Revisions

A Revision is a specific version of a Package.  It is identified by a hash of the package contents, and may be tagged with a human-readable name.  The most recent Revision is called `latest`.

Unlike software packages, Quilt packages can be extremely large (thousands of files, terabytes of data).
Therefore, the Quilt API must make it easy to only download and modify the parts of a package that are needed.  This complicates the semantics of updating Revisions stored across different systems, as Manifests do not currently keep track of their entire Revision history.

### II.H. Lineages

To address the Revision history problem, `quilt_rs` added a new concept called Lineages. The Lineage file for a Domain tracks which Packages have been downloaded and installed, and from where.
It also tracks which Revisions of each Package have been "checked out" for users to edit.

## III. Current Architecture

### III.A. Library Module

The `lib.rs` file contains the following modules and exported types:

> EP: Descriptions by ChatGPT.  Can someone cross-check?


1. `paths` (*private*): abstracts away the local filesystem
2. `quilt` (**public**): legacy module, primarily focused on managing the local cache
   1. `InstalledPackage`: represents a package that has been installed in the local cache.  It contains a `PackageRef` and a `PackageInstance`, and is used to track the installed packages.
   2. `LocalDomain`: represents the local cache, and contains a list of `InstalledPackages`
   3. `Manifest`: represents the on-disk manifest for a Quilt package. This needs to exist at the Storage layer, because Manifests may be lazily loaded and queried in by chunks or columns.
   4. `RemoteManifest`: references a remote manifest, and is used to create a `PackageRef`.
   5. `S3PackageUri`: represents a Quilt+ URI, which is a URL with a `quilt+` scheme.  This is used to identify a package or registry in a Quilt-compatible storage system.
3. `quilt4` (*private*): this is the newer module, primary focused on managing Parquet manifests
   1. `manifest::Manifest4`: represents the high-level manifest object
   2. `row4::Row4`: represents a row in a Parquet manifest.
   3. `table::Table`: represents a low-level Parquet table.
   4. `uri::UriParser`: parses a URI into a `UriQuilt` object.
   5. `uri::UriQuilt`: represents a Quilt URI, which is a URL with a `quilt` scheme.  This is used to uniquely identify a package, registry, or path.
4. `s3_utils` (**public**): contains utilities for working with S3
5. `utils` (**public**): contains general utilities

It also defines the `Error` type and two high-level functions:

- `install_temporarily`: installs a package into a temporary folder
- `installed_packages`: returns a list of all currently installed packages

### III.B. Main Module

`quilt_rs` provides a simple CLI interface for interacting with Quilt packages.
The functionality for this is in the `cli` module, which supports the following commands:

- `Browse`
- `Install`
- `List`
- `Package`
- `Uninstall`

> EP: Is there a `Push` command?

## IV. Open Issues

### IV.A. Generic Storage Layer

1. Is Storage a single pure function?  Or a set of methods on `UPath`? Or does it manage concurrency?
2. Do we need to lock Storage? One lock per Domain?
3. Should 'Domain' contain a mutable cache? Or should it store all its state in the filesystem, and read it in each time?
4. Should `execute` be on the Domain object, since it typically needs to update both Contents and Registry?
5. Do we need something like `OpenDAL` to unify S3 buckets and local filesystems into a single Storage trait?

### IV.B. Object Lifecycles



