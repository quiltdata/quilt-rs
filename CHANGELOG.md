# CHANGELOG

## [0.5.5] * 2024-02-29
* Handle sha2-256-chunked hashes in InstalledPackage::status()

## [0.5.4] * 2024-02-23
* Add support for sha2-256-chunked hashes

## [0.5.3] * 2024-02-01
* Fix incorrect paths when pushing to S3
* Create package directory even when no paths are installed

## [0.5.2] * 2024-01-26
* Make Change.{current,previous} properties public
* Fix object directory creation
* Update dependencies

## [0.5.1] * 2024-01-25

* Add Parquet support
* Remove dead code
* Cleanup tests

## [0.5.0] * 2023-12-20

* Remove Poem dependency

## [0.4.4] * 2023-12-20

* Use tracing::info! instead of println!

## [0.4.3] * 2023-12-19

* Expose quilt for QuiltSync transition

## [0.4.2] * 2023-12-18

* use aptos-openapi-link for poem serialization
* add back Removed fields (but skip)

## [0.4.1] * 2023-12-18

### Added

* serde serialization

### Removed

(to enable trivial serialization)

* object_store.Path
* aws.Region
* aws.S3Client
* arrow.RecordBatch
* Multihash

## [0.4.0] * 2023-12-17

* test get_manifest3
* add tests/example domain
* store shared test data in its own crate
* stub UPath
* implement UriParser and UriQuilt

## [0.3.2] * 2023-12-14

* get_client
* use client in UPath

## [0.3.1] * 2023-12-14

* expose quilt4 module

## [0.3.0] * 2023-12-14

* stub quilt4 module

## [0.2.0] * 2023-12-13

* export Manifest structs
* add installed_packages
* add manifest_from_uri
* main prints manifest for uri

## [0.1.1] * 2023-12-13

* Added metadata
* Added CHANGELOG.md
* Updated README

## [0.1.0] * 2023-12-13

* Initial release
* Imported from Project 4F
* Added Integration Tests
* Removed application code
