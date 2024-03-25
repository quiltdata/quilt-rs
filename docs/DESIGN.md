# quilt-rs v0.7 Design Review

## I. Objectives

### quilt-rs 

The purpose of quilt-rs is to provide a simple and efficient API for managing Quilt packages and registries.
In particular, we need objects that represent both on-disk data structures and the operations that can be performed on them. Importantly, we also want the ability to transpasrently support multiple storage backends (local filesystem, S3, mock, etc.) and registry types (i.e., flat vs versioned).

### Refactoring

The objective of this design is to refactor the existing codebase to it easier to understand, test, and extend. Specifically:

1. Create standalone  components that can be unit-tested individually
2. Use Rust's type system to enforce invariants
3. Use Rust's ownership model to manage memory, to avoid unecessary locks
4. Use single-threaded sync calls wherever possible, and only introduce concurrency where absolutely necessary
5. Decouple the storage layer from the domain layer, so that we can easily swap out storage backends

## II. Architecture

The new architecture has three layer:

1. Storage: the interface for interacting with storage layers, and the corresponding implementations and support objects
2. Model: the in-memory data structures that represent and cache on-disk packages and registries
3. Commands: the high-level operations that can be performed on the model objects

Unlike in Python, the Model objects do not manage their own Storage.  Instead, Commands manage the Storage, and the Model objects are passed in as arguments.

### II.A. Storage Types

1. **UPath**: an type representing a path in any supported storage layer (currently filesystems and S3, plus Mock).  
   1. A UPath can be created from any supported URL (currently only `file:` and `s3:`), and can be converted back to a URL.
   2. There are subtypes for `UPathObject` and `UPathPrefix`, which are used to distinguish between objects ("files") and prefixes ("folders")
   3. UPath will probably be an Enum with variants for each supported storage type

2. **Store**: a trait that defines the interface for interacting with storage layers. 
   1. The three basic methods are `read`, `write`, and `hash` for  `UPathObject`, plus `list` for `UPathPrefix`.  Each of these takes a `Vec<UPath>` as input, allowing the Storage layer to optimize for batch operations (using concurrency if necessary).
   2. The concrete implementations are:
      1. `StoreFiles`: a local filesystem implementation
      2. `StoreS3Objects`: an S3 implementation
      3. `StoreMock`: a mock implementation for testing
   3. UPath objects have a `defaultStore` method that returns the appropriate Store implementation for that UPath, though users can instead use `StoreMock`.
   4. Store objects are stateless [and thread-safe?], so they can be shared across threads.
3. **QuiltURI**: a type that represents a Quilt+ URI, which is a URL with a `quilt+` scheme.  This is used to identify a package or registry in a Quilt-compatible storage system.
   1. Provides accessor methods for the `domain`, `package`, and (optional) `revision` components of the URI.
   2. May also contain one more `path` fragments, which are used to identify specific objects within a package.
   3. Used to locate a `Manifest` object
   4. In general, requires Storage access to find the hash necessary to create a `PackageRef`
4. **PackageName**: a string containing exactly one slash, and only alphanumeric characters plus hyphen and underscore.
5. **PackageRef**: a type that represents a reference to a Quilt package.  This is a combination of a `UPath` to the `Registry` and a `hash`, as well as the original `QuiltURI` and `PackageName`. It is used to identify a specific package instance, and can be dereferenced to a `Manifest`. [Might be called a RemotePackage?]
6. **Manifest**: a type that represents the on-disk manifest for a Quilt package. This needs to exist a the Storage layer, because Manifests may be lazily loaded and queried in by chunks or columns.

### II.B. Model Objects

#### Immutable

1. **Entry**: a type that represents a single object in a package.  This is a lightweight object defined by its `name` (with one or more slashes) and `place` (a valid URL). It also contains the object's `hash`, `size`, system `info` and user `meta`.
2. **Header**: a special Entry beginning with `.` that represents the package's metadata (the `size`, `hash`, and `place` fields are currently unused).
3. **PackageInstance**: a type that corresponding to a single `Manifest` (dereferenced `PackageRef`) that contains:
   1. Hash
   2. Header (currently only one)
   3. Entries [or a lazy-list pulled dynamically from the Manifest?]

#### Mutable

These objects each are created from a `UPathPrefix`, and represent on-disk folders.

1. **Namespace**: a container that uses a `Revision` (Tag or Timestamp) to find the hash of a specific package.  It can be iterated to get a list of package `Revisions`, and is used to construct a `PackageRef`.
2. **PackageStore**: a container that caches `PackageInstance` objects, and can look them up (or load them) by hash.
3. **Registry**: a container for the Namespace and PackageStore
4. **Lineage**: a data structure (Manifest?) tracking the the most recent actions for each Namespace, as well as any WorkingFolders
5. **Domain**: a container for the Registry and Store, which is used to manage the local cache and remote storage.
6. **WorkingFolder**: the editable folder users use to edit package contents before committing them to the Registry.
7. **InstalledPackage**: a type that represents a package that has been installed in the local cache.  It contains a `PackageRef` and a `PackageInstance`, and is used to track the installed packages.

### II.C. Command Objects

These command objects are returned by model objects, then called with the default or user-provided Store and one or more Domain objects.

1. **Browse**: a command that translates a `PackageRef` into a `PackageInstance`, and stores it in the local cache.
2. **Install**: a command that installs a `PackageInstance` into an `InstalledPackage` in the local cache, and copies it into a `WorkingFolder`.
3. **Commit**: a command that commits a `WorkingFolder` to the `Registry`, creating a new `Revision` in the corresponding `Namespace`.
4. **Push**: a command that pushes a `Revision` to a remote `Domain`.
5. **Pull**: a command that pulls newer `Revision` from a remote `Domain` to the local cache.
6. **List**: return all the `names` in a `PackageInstance`
