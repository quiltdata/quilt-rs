# Types

> WORK IN PROGRESS
> 
The goal of this document is to have a common understanding of the types used in
the project. It is a draft and a work in progress. Please extend and improve if possible.

# Domain

A set of namespaces, packages, and lineage
Envelope for the entire system

# Logical Domain

Rust classes for manipulating the physical domain

# Physical Domain

Actual bytes that we can read or write.

## Local Domain

A filesystem with things stored in it.

## S3 Domain

A S3 bucket with things stored in it.

## Storage

A set of logical keys

## Registry
Metadata describing the packages

### Set<Namespace>
### Set<Package>
### Lineage

## Lineage

# Filesystem Namespace

Concrete instance of a namespace on the filesystem

# Namespace

An ordered list of package revisions

## Revision

Refers to a specific package by its hash

### Tag

### Timestamp


# Package

Contains multiple files
Installing a single file from a package is a common operation

## Hash: MultiHash (from IPFS) from the multihash crate

The version of the package

## Manifest

A list of package-level metadata and a list of entries

### Header

### Entries: List<Entry>

Name: Logical key
Place: Physical key 
Hash: MultiHash
Size: usize 
Info: Dictionary containing arbitrary JSON, might be of type `Info` in the future
Meta: Json with strings for keys: {} -> JsonDict

#### Logical Key

String, which cannot be empty

#### Physical Key

A URI that can be dereferenced to get a bag of bytes
The goal is to read, hash
Intended to be read-only, but not enforced
The goal is to enforce immutability, but not versioning
On a local file-system there is no versioning







# Lineage

# PackageRef
Handle
Address for package

# PackageLineage
Latest hash
History

# Domain
Might be able to get rid of it?
Stores only root directory where lineage and any file is stored
We can just pass the root handle to every function/method

# Package Flow
Contains all meaningful operations for working with packages

## Commit
## Install
Install a package
## Make changes
## Resolve conflicts
## Push
## Pull
## Browse contents of package

# Namespace Flow
Contains all meaningful operations for working with namespaces

## Info
## Commit
## Install
Install a package
## Make changes
## Resolve conflicts
## Push
## Pull
## List all packages



# Remote

-------------------

# Missing Types

We might add the following types in the future.

## Info

#[non_exhaustive]
enum Info {
    Version0(Version0),
    UnknownVersion(Json),
}

struct Version0 {
    message: String,
}

## UserMeta

Json Dictionary, which can be empty

