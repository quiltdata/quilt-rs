# CHANGELOG

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
