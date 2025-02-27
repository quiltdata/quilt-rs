# CHANGELOG

## [0.9.0]

* Bump a version number to highlight the accumutated changes of the 0.8.* versions

## [0.8.14]

* Add `display_for_host` method for `S3PackageUri`
* Remove default host from `display_for_host`, some host is always required

## [0.8.13]

* Handle missing keys/values in workflows config 
* Add helper function to display catalog URL

## [0.8.12]

* Added log messages in every "flow"
* Added tests for authentication
* Merged two errors into one `LoginRequired` error using `Option` argument

## [0.8.11]

* Add authentication to Quilt Stack preserving backward compatibility with getting credentials from `~/.aws`
* `domain` path is now required for every command internally, but is optional for users
   If `domain` is not provided, the default user data directory is used
* New command `login`

## [0.8.10]

* Fix workflow format by adding `schemas` property
* Security fix: update openssl

## [0.8.9]

* Remove unnecessary Mutex wrappers from `LocalDomain` and `InstalledPackage` structs since file I/O operations already provide synchronization through async/await
* Adds new test `test_spamming_commit_writes` in installed_package.rs to verify sequential commits work correctly without mutex protection
  `file.flush()` is what fixed the issue in the previous commit, not the Mutex.

## [0.8.8]

* Added "workflow" parameter for commit
* Implemented tests for CLI, treating them as integration tests. They use real packages from Quilt stack
* Increased test coverage to 79%
* Moved HTTPS and AWS S3 clients to the `RemoteS3`, and use `RwLock` there in struct
* Guard lineage with `Mutex` for `InstalledPackage`

## [0.8.7]

* Throw error if locally committed package and remote have different `top_hash`
* Fix calculating hashes for files <8Mb
* De-duplicate entries when user add files equal to the file that is not tracked and is already in a remote manifest

## [0.8.6]

* Copy package pushed to the remote to the local storage.
  Locally committed package and remote have different `top_hash`, because local manifest has `file://` physical keys.
* `package_s3_prefix` (CLI `package` command) accepts `--message` and `--user_meta` arguments similar to `commit` command

## [0.8.5]

* `package_s3_prefix` (CLI `package` command) will calculate checksum if missing

## [0.8.4]

* Handle `&catalog` in `quilt+s3` URI's `.to_string()`

## [0.8.3]

* Handle `&catalog` in `quilt+s3` URI

## [0.8.2]

* Chores: update dependencies

## [0.8.1]

* Test creating manifest with a billion rows via `quilt_rs benchmark` and improve performance https://github.com/quiltdata/quilt-rs/pull/179
* Use `Row::default_header()` instead of `Row::default()` https://github.com/quiltdata/quilt-rs/pull/182

## [0.8.0]

* More docs, move `LocalDomain` and `InstalledPackage` to modules  https://github.com/quiltdata/quilt-rs/pull/176
* Write manifests using Stream, de-couple read from write https://github.com/quiltdata/quilt-rs/pull/175
* Folders reorganization https://github.com/quiltdata/quilt-rs/pull/167
* Use `PathBuf` for paths where possible https://github.com/quiltdata/quilt-rs/pull/165
* Use `Namespace` struct instead of `String` https://github.com/quiltdata/quilt-rs/pull/166
* More tests: `install_paths` and `status`, cover more cases of `Storage`/`Remote` use https://github.com/quiltdata/quilt-rs/pull/164

## [0.7.0]

* Fix order of JSON in manifest to make checksums constant

## [0.6.0]

* Make `utils` module private as it only contains helper functions for testing. Remove dummy tests.
* Refactor code to introduce tests

## [0.5.8] - 2024-03-25

* Fixed calculating checksums for new files

## [0.5.7] - 2024-03-21

* Added command-line interface: `browse`, `install`, `list`, `package` and `uninstall` commands

## [0.5.6] - 2024-03-01

* Implement multipart uploads and server-side checksums
* Use sha2-256-chunked hashes for newly added files
* Fix Parquet->JSONL conversion unconditionally setting hash type to SHA256

## [0.5.5] - 2024-02-29

* Handle sha2-256-chunked hashes in InstalledPackage::status()

## [0.5.4] - 2024-02-23

* Add support for sha2-256-chunked hashes

## [0.5.3] - 2024-02-01

* Fix incorrect paths when pushing to S3
* Create package directory even when no paths are installed

## [0.5.2] - 2024-01-26

* Make Change.{current,previous} properties public
* Fix object directory creation
* Update dependencies

## [0.5.1] - 2024-01-25

* Add Parquet support
* Remove dead code
* Cleanup tests

## [0.5.0] - 2023-12-20

* Remove Poem dependency

## [0.4.4] - 2023-12-20

* Use tracing::info! instead of println!

## [0.4.3] - 2023-12-19

* Expose quilt for QuiltSync transition

## [0.4.2] - 2023-12-18

* use aptos-openapi-link for poem serialization
* add back Removed fields (but skip)

## [0.4.1] - 2023-12-18

### Added

* serde serialization

### Removed

(to enable trivial serialization)

* object_store.Path
* aws.Region
* aws.S3Client
* arrow.RecordBatch
* Multihash

## [0.4.0] - 2023-12-17

* test get_manifest3
* add tests/example domain
* store shared test data in its own crate
* stub UPath
* implement UriParser and UriQuilt

## [0.3.2] - 2023-12-14

* get_client
* use client in UPath

## [0.3.1] - 2023-12-14

* expose quilt4 module

## [0.3.0] - 2023-12-14

* stub quilt4 module

## [0.2.0] - 2023-12-13

* export Manifest structs
* add installed_packages
* add manifest_from_uri
* main prints manifest for uri

## [0.1.1] - 2023-12-13

* Added metadata
* Added CHANGELOG.md
* Updated README

## [0.1.0] - 2023-12-13

* Initial release
* Imported from Project 4F
* Added Integration Tests
* Removed application code
