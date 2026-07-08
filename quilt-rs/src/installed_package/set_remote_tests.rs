//! Tests for configuring a package's remote via `set_remote`.

use super::*;

use test_log::test;

use aws_sdk_s3::primitives::ByteStream;

use crate::io::remote::WorkflowIntent;
use crate::io::remote::mocks::MockRemote;
use crate::lineage::DomainLineageIo;
use crate::lineage::Home;
use crate::lineage::PackageLineageIo;
use crate::manifest::ManifestHeader;
use crate::paths::DomainPaths;

#[test(tokio::test)]
async fn test_set_remote_on_local_package() -> Res {
    let (home, _temp_dir1) = Home::from_temp_dir()?;
    let (paths, _temp_dir2) = DomainPaths::from_temp_dir()?;

    let storage = LocalStorage::new();
    let remote = MockRemote::default();
    let namespace: Namespace = ("test", "local").into();

    paths
        .scaffold_for_installing(&storage, &home, &namespace)
        .await?;

    let lineage_json = r#"{
        "packages": {
            "test/local": {
                "commit": null,
                "remote": null,
                "base_hash": "",
                "latest_hash": "",
                "paths": {}
            }
        },
        "home": "/tmp/working_dir"
    }"#;
    storage
        .write_byte_stream(&paths.lineage(), lineage_json.as_bytes().to_vec().into())
        .await?;

    let domain_lineage_io = DomainLineageIo::new(paths.lineage());
    let package = InstalledPackage {
        lineage: PackageLineageIo::new(domain_lineage_io, namespace.clone()),
        paths,
        remote,
        storage,
        namespace,
    };

    package
        .set_remote(
            "my-bucket".to_string(),
            Some("example.com".parse()?),
            WorkflowIntent::BucketDefault,
        )
        .await?;

    let lineage = package.lineage().await?;
    let remote_uri = lineage
        .remote_uri
        .as_ref()
        .expect("remote_uri should be set");
    assert_eq!(
        remote_uri.origin.as_ref().unwrap().to_string(),
        "example.com"
    );
    assert_eq!(remote_uri.bucket, "my-bucket");
    assert_eq!(remote_uri.hash, "");

    Ok(())
}

#[test(tokio::test)]
async fn test_set_remote_empty_bucket_error() -> Res {
    let (home, _temp_dir1) = Home::from_temp_dir()?;
    let (paths, _temp_dir2) = DomainPaths::from_temp_dir()?;

    let storage = LocalStorage::new();
    let remote = MockRemote::default();
    let namespace: Namespace = ("test", "local").into();

    paths
        .scaffold_for_installing(&storage, &home, &namespace)
        .await?;

    let lineage_json = r#"{
        "packages": {
            "test/local": {
                "commit": null,
                "remote": null,
                "base_hash": "",
                "latest_hash": "",
                "paths": {}
            }
        },
        "home": "/tmp/working_dir"
    }"#;
    storage
        .write_byte_stream(&paths.lineage(), lineage_json.as_bytes().to_vec().into())
        .await?;

    let domain_lineage_io = DomainLineageIo::new(paths.lineage());
    let package = InstalledPackage {
        lineage: PackageLineageIo::new(domain_lineage_io, namespace.clone()),
        paths,
        remote,
        storage,
        namespace,
    };

    let result = package
        .set_remote(
            String::new(),
            Some("example.com".parse()?),
            WorkflowIntent::BucketDefault,
        )
        .await;

    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Bucket cannot be empty"),
        "Error should mention empty bucket"
    );

    Ok(())
}

#[test(tokio::test)]
async fn test_set_remote_rejects_unreachable_bucket() -> Res {
    use crate::error::RemoteCatalogError;

    /// Remote that rejects any `verify_bucket` call — models the case
    /// where the user typed a bucket that doesn't resolve on S3.
    struct BadBucketRemote;

    impl Remote for BadBucketRemote {
        async fn exists(&self, _host: &Option<Host>, _s3_uri: &S3Uri) -> Res<bool> {
            unreachable!("test only exercises verify_bucket")
        }
        async fn get_object_stream(
            &self,
            _host: &Option<Host>,
            _s3_uri: &S3Uri,
        ) -> Res<crate::io::remote::RemoteObjectStream> {
            unreachable!("test only exercises verify_bucket")
        }
        async fn resolve_url(&self, _host: &Option<Host>, _s3_uri: &S3Uri) -> Res<S3Uri> {
            unreachable!("test only exercises verify_bucket")
        }
        async fn put_object(
            &self,
            _host: &Option<Host>,
            _s3_uri: &S3Uri,
            _contents: impl Into<aws_sdk_s3::primitives::ByteStream>,
        ) -> Res {
            unreachable!("test only exercises verify_bucket")
        }
        async fn upload_file(
            &self,
            _host_config: &crate::io::remote::HostConfig,
            _source_path: impl AsRef<std::path::Path>,
            _dest_uri: &S3Uri,
            _size: u64,
        ) -> Res<(S3Uri, crate::checksum::ObjectHash)> {
            unreachable!("test only exercises verify_bucket")
        }
        async fn host_config(&self, _host: &Option<Host>) -> Res<crate::io::remote::HostConfig> {
            Ok(crate::io::remote::HostConfig::default())
        }
        async fn verify_bucket(&self, bucket: &str) -> Res {
            Err(RemoteCatalogError::BucketUnreachable(bucket.to_string()).into())
        }
    }

    let (home, _temp_dir1) = Home::from_temp_dir()?;
    let (paths, _temp_dir2) = DomainPaths::from_temp_dir()?;

    let storage = LocalStorage::new();
    let namespace: Namespace = ("test", "badbucket").into();

    paths
        .scaffold_for_installing(&storage, &home, &namespace)
        .await?;

    let lineage_json = r#"{
        "packages": {
            "test/badbucket": {
                "commit": null,
                "remote": null,
                "base_hash": "",
                "latest_hash": "",
                "paths": {}
            }
        },
        "home": "/tmp/working_dir"
    }"#;
    storage
        .write_byte_stream(&paths.lineage(), lineage_json.as_bytes().to_vec().into())
        .await?;

    let domain_lineage_io = DomainLineageIo::new(paths.lineage());
    let package = InstalledPackage {
        lineage: PackageLineageIo::new(domain_lineage_io, namespace.clone()),
        paths,
        remote: BadBucketRemote,
        storage,
        namespace,
    };

    let result = package
        .set_remote(
            "typo-bucket".to_string(),
            Some("example.com".parse()?),
            WorkflowIntent::BucketDefault,
        )
        .await;

    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("typo-bucket") && msg.contains("not reachable"),
        "error should name the bucket and say it's unreachable, got: {msg}"
    );

    // The remote must NOT have been persisted — pre-flight should fail
    // before any lineage write.
    let lineage = package.lineage().await?;
    assert!(
        lineage.remote_uri.is_none(),
        "remote_uri should not be persisted when verify_bucket fails",
    );

    Ok(())
}

#[test(tokio::test)]
async fn test_set_remote_rejects_change_on_pushed_package() -> Res {
    let (home, _temp_dir1) = Home::from_temp_dir()?;
    let (paths, _temp_dir2) = DomainPaths::from_temp_dir()?;

    let storage = LocalStorage::new();
    let remote = MockRemote::default();
    let namespace: Namespace = ("test", "overwrite").into();

    paths
        .scaffold_for_installing(&storage, &home, &namespace)
        .await?;

    let lineage_json = r#"{
        "packages": {
            "test/overwrite": {
                "commit": null,
                "remote": {
                    "bucket": "old-bucket",
                    "namespace": "test/overwrite",
                    "hash": "abc123",
                    "origin": "old.host"
                },
                "base_hash": "abc123",
                "latest_hash": "abc123",
                "paths": {}
            }
        },
        "home": "/tmp/working_dir"
    }"#;
    storage
        .write_byte_stream(&paths.lineage(), lineage_json.as_bytes().to_vec().into())
        .await?;

    let domain_lineage_io = DomainLineageIo::new(paths.lineage());
    let package = InstalledPackage {
        lineage: PackageLineageIo::new(domain_lineage_io, namespace.clone()),
        paths,
        remote,
        storage,
        namespace,
    };

    let result = package
        .set_remote(
            "new-bucket".to_string(),
            Some("new.host".parse()?),
            WorkflowIntent::BucketDefault,
        )
        .await;

    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Cannot change remote"),
        "Should reject changing remote on a pushed package"
    );

    Ok(())
}

#[test(tokio::test)]
async fn test_set_remote_is_idempotent_on_pushed_package() -> Res {
    let (home, _temp_dir1) = Home::from_temp_dir()?;
    let (paths, _temp_dir2) = DomainPaths::from_temp_dir()?;

    let storage = LocalStorage::new();
    let remote = MockRemote::default();
    let namespace: Namespace = ("test", "idempotent").into();

    paths
        .scaffold_for_installing(&storage, &home, &namespace)
        .await?;

    let lineage_json = r#"{
        "packages": {
            "test/idempotent": {
                "commit": null,
                "remote": {
                    "bucket": "my-bucket",
                    "namespace": "test/idempotent",
                    "hash": "abc123",
                    "origin": "my.host"
                },
                "base_hash": "abc123",
                "latest_hash": "abc123",
                "paths": {}
            }
        },
        "home": "/tmp/working_dir"
    }"#;
    storage
        .write_byte_stream(&paths.lineage(), lineage_json.as_bytes().to_vec().into())
        .await?;

    let domain_lineage_io = DomainLineageIo::new(paths.lineage());
    let package = InstalledPackage {
        lineage: PackageLineageIo::new(domain_lineage_io, namespace.clone()),
        paths,
        remote,
        storage,
        namespace,
    };

    // Same bucket+origin as existing — should be a no-op
    package
        .set_remote(
            "my-bucket".to_string(),
            Some("my.host".parse()?),
            WorkflowIntent::BucketDefault,
        )
        .await?;

    let lineage = package.lineage().await?;
    let remote_uri = lineage
        .remote_uri
        .as_ref()
        .expect("remote_uri should be set");
    assert_eq!(remote_uri.hash, "abc123", "hash should be preserved");

    Ok(())
}

#[test(tokio::test)]
async fn test_set_remote_overwrites_unpushed_remote() -> Res {
    let (home, _temp_dir1) = Home::from_temp_dir()?;
    let (paths, _temp_dir2) = DomainPaths::from_temp_dir()?;

    let storage = LocalStorage::new();
    let remote = MockRemote::default();
    let namespace: Namespace = ("test", "unpushed").into();

    paths
        .scaffold_for_installing(&storage, &home, &namespace)
        .await?;

    let lineage_json = r#"{
        "packages": {
            "test/unpushed": {
                "commit": null,
                "remote": {
                    "bucket": "old-bucket",
                    "namespace": "test/unpushed",
                    "hash": "",
                    "origin": "old.host"
                },
                "base_hash": "",
                "latest_hash": "",
                "paths": {}
            }
        },
        "home": "/tmp/working_dir"
    }"#;
    storage
        .write_byte_stream(&paths.lineage(), lineage_json.as_bytes().to_vec().into())
        .await?;

    let domain_lineage_io = DomainLineageIo::new(paths.lineage());
    let package = InstalledPackage {
        lineage: PackageLineageIo::new(domain_lineage_io, namespace.clone()),
        paths,
        remote,
        storage,
        namespace,
    };

    package
        .set_remote(
            "new-bucket".to_string(),
            Some("new.host".parse()?),
            WorkflowIntent::BucketDefault,
        )
        .await?;

    let lineage = package.lineage().await?;
    let remote_uri = lineage
        .remote_uri
        .as_ref()
        .expect("remote_uri should be set");
    assert_eq!(remote_uri.origin.as_ref().unwrap().to_string(), "new.host");
    assert_eq!(remote_uri.bucket, "new-bucket");
    assert_eq!(remote_uri.hash, "", "hash should remain empty");

    Ok(())
}

#[test(tokio::test)]
async fn test_set_remote_recommits_existing_commit() -> Res {
    let (home, _temp_dir1) = Home::from_temp_dir()?;
    let (paths, _temp_dir2) = DomainPaths::from_temp_dir()?;

    let storage = LocalStorage::new();
    let remote = MockRemote::default();
    let namespace: Namespace = ("test", "recommit").into();

    paths
        .scaffold_for_installing(&storage, &home, &namespace)
        .await?;

    // Start with no remote and no commit
    let lineage_json = r#"{
        "packages": {
            "test/recommit": {
                "commit": null,
                "remote": null,
                "base_hash": "",
                "latest_hash": "",
                "paths": {}
            }
        },
        "home": "/tmp/working_dir"
    }"#;
    storage
        .write_byte_stream(&paths.lineage(), lineage_json.as_bytes().to_vec().into())
        .await?;

    // Write a file to package home so commit has something to pick up
    let package_home = home.join(namespace.to_string());
    storage.create_dir_all(&package_home).await?;
    storage
        .write_byte_stream(
            package_home.join("data.txt"),
            ByteStream::from_static(b"hello world"),
        )
        .await?;

    let domain_lineage_io = DomainLineageIo::new(paths.lineage());
    let package = InstalledPackage {
        lineage: PackageLineageIo::new(domain_lineage_io, namespace.clone()),
        paths,
        remote,
        storage,
        namespace: namespace.clone(),
    };

    // Commit the package (no remote yet, uses default HostConfig)
    let commit = package
        .commit(
            "Initial commit".to_string(),
            UserMeta::Set(serde_json::json!({"key": "value"})),
            None,
            None,
        )
        .await?;
    let hash_before = commit.hash.clone();

    // Now set_remote — this should trigger recommit.
    // MockRemote returns HostConfig::default() (SHA256 chunked), same as the
    // initial commit, so the row hashes stay the same. But the manifest is
    // rebuilt (e.g. workflow may change), and the lineage prev_hashes are updated.
    package
        .set_remote(
            "my-bucket".to_string(),
            Some("example.com".parse()?),
            WorkflowIntent::BucketDefault,
        )
        .await?;

    let lineage = package.lineage().await?;

    // Remote should be set
    let remote_uri = lineage
        .remote_uri
        .as_ref()
        .expect("remote_uri should be set");
    assert_eq!(
        remote_uri.origin.as_ref().unwrap().to_string(),
        "example.com"
    );
    assert_eq!(remote_uri.bucket, "my-bucket");

    // Recommit should have produced a new commit
    let new_commit = lineage.commit.as_ref().expect("commit should exist");
    assert_eq!(
        new_commit.prev_hashes.first(),
        Some(&hash_before),
        "Old hash should be in prev_hashes after recommit"
    );

    // The new manifest should be readable with preserved message and meta
    let manifest_path = package
        .paths
        .installed_manifest(&namespace, &new_commit.hash);
    let manifest = Manifest::from_path(&package.storage, &manifest_path).await?;
    assert_eq!(
        manifest.header.message,
        Some("Initial commit".to_string()),
        "Message should be preserved after recommit"
    );
    assert_eq!(
        manifest.header.user_meta,
        Some(serde_json::json!({"key": "value"})),
        "User meta should be preserved after recommit"
    );

    Ok(())
}

#[test(tokio::test)]
async fn test_resolve_workflow_without_remote_is_none_for_every_intent() -> Res {
    let (home, _temp_dir1) = Home::from_temp_dir()?;
    let (paths, _temp_dir2) = DomainPaths::from_temp_dir()?;

    let storage = LocalStorage::new();
    let remote = MockRemote::default();
    let namespace: Namespace = ("test", "noremote").into();

    paths
        .scaffold_for_installing(&storage, &home, &namespace)
        .await?;

    let lineage_json = r#"{
        "packages": {
            "test/noremote": {
                "commit": null,
                "remote": null,
                "base_hash": "",
                "latest_hash": "",
                "paths": {}
            }
        },
        "home": "/tmp/working_dir"
    }"#;
    storage
        .write_byte_stream(&paths.lineage(), lineage_json.as_bytes().to_vec().into())
        .await?;

    let domain_lineage_io = DomainLineageIo::new(paths.lineage());
    let package = InstalledPackage {
        lineage: PackageLineageIo::new(domain_lineage_io, namespace.clone()),
        paths,
        remote,
        storage,
        namespace,
    };

    for intent in [
        WorkflowIntent::BucketDefault,
        WorkflowIntent::NoWorkflow,
        WorkflowIntent::Named("foo".to_string()),
    ] {
        assert!(
            package.resolve_workflow(intent.clone()).await?.is_none(),
            "no-remote short-circuit should return None for {intent:?}"
        );
    }

    Ok(())
}

#[test(tokio::test)]
async fn test_set_remote_recommit_picks_up_bucket_default() -> Res {
    let (home, _temp_dir1) = Home::from_temp_dir()?;
    let (paths, _temp_dir2) = DomainPaths::from_temp_dir()?;

    let storage = LocalStorage::new();
    let remote = MockRemote::default();
    let namespace: Namespace = ("test", "bucketdefault").into();

    paths
        .scaffold_for_installing(&storage, &home, &namespace)
        .await?;

    // The target bucket declares a `default_workflow`.
    let config_uri: S3Uri = "s3://my-bucket/.quilt/workflows/config.yml".parse()?;
    let config = r"
default_workflow: foo
workflows:
  foo:
    metadata_schema: bar
schemas:
  bar:
    url: s3://my-bucket/schemas/test.json
";
    let schema_uri: S3Uri = "s3://my-bucket/schemas/test.json".parse()?;
    remote
        .put_object(&None, &config_uri, config.as_bytes().to_vec())
        .await?;
    remote
        .put_object(&None, &schema_uri, b"{}".to_vec())
        .await?;

    // Start with no remote and no commit
    let lineage_json = r#"{
        "packages": {
            "test/bucketdefault": {
                "commit": null,
                "remote": null,
                "base_hash": "",
                "latest_hash": "",
                "paths": {}
            }
        },
        "home": "/tmp/working_dir"
    }"#;
    storage
        .write_byte_stream(&paths.lineage(), lineage_json.as_bytes().to_vec().into())
        .await?;

    // Write a file to package home so commit has something to pick up
    let package_home = home.join(namespace.to_string());
    storage.create_dir_all(&package_home).await?;
    storage
        .write_byte_stream(
            package_home.join("data.txt"),
            ByteStream::from_static(b"hello world"),
        )
        .await?;

    let domain_lineage_io = DomainLineageIo::new(paths.lineage());
    let package = InstalledPackage {
        lineage: PackageLineageIo::new(domain_lineage_io, namespace.clone()),
        paths,
        remote,
        storage,
        namespace: namespace.clone(),
    };

    // Commit the package locally (no remote yet, so no workflow stamped)
    package
        .commit(
            "Initial commit".to_string(),
            UserMeta::Set(serde_json::json!({"key": "value"})),
            None,
            None,
        )
        .await?;

    // set_remote triggers recommit, which must stamp the bucket default.
    package
        .set_remote(
            "my-bucket".to_string(),
            Some("example.com".parse()?),
            WorkflowIntent::BucketDefault,
        )
        .await?;

    let lineage = package.lineage().await?;
    let new_commit = lineage.commit.as_ref().expect("commit should exist");
    let manifest_path = package
        .paths
        .installed_manifest(&namespace, &new_commit.hash);
    let manifest = Manifest::from_path(&package.storage, &manifest_path).await?;

    let workflow = manifest
        .header
        .workflow
        .expect("recommit should stamp a workflow from the bucket default");
    assert_eq!(
        workflow.id.expect("workflow id should be set").id,
        "foo",
        "recommit should pick up the bucket's default_workflow"
    );

    Ok(())
}

/// Set up a locally-committed package against a bucket whose config declares a
/// `foo` workflow but no `default_workflow`, run `set_remote` with `intent`, and
/// return the recommitted manifest's header. The absence of `default_workflow`
/// is what lets the assertions distinguish the caller's chosen intent from the
/// bucket-default fallback.
async fn recommit_manifest_for_intent(slug: &str, intent: WorkflowIntent) -> Res<ManifestHeader> {
    let (home, _temp_dir1) = Home::from_temp_dir()?;
    let (paths, _temp_dir2) = DomainPaths::from_temp_dir()?;

    let storage = LocalStorage::new();
    let remote = MockRemote::default();
    let namespace: Namespace = ("test", slug).into();

    paths
        .scaffold_for_installing(&storage, &home, &namespace)
        .await?;

    // The target bucket declares `foo` but no `default_workflow`, so the
    // no-gesture (BucketDefault) path would stamp an id-less record.
    let config_uri: S3Uri = "s3://my-bucket/.quilt/workflows/config.yml".parse()?;
    let config = r"
workflows:
  foo:
    metadata_schema: bar
schemas:
  bar:
    url: s3://my-bucket/schemas/test.json
";
    let schema_uri: S3Uri = "s3://my-bucket/schemas/test.json".parse()?;
    remote
        .put_object(&None, &config_uri, config.as_bytes().to_vec())
        .await?;
    remote
        .put_object(&None, &schema_uri, b"{}".to_vec())
        .await?;

    let lineage_json = format!(
        r#"{{
        "packages": {{
            "test/{slug}": {{
                "commit": null,
                "remote": null,
                "base_hash": "",
                "latest_hash": "",
                "paths": {{}}
            }}
        }},
        "home": "/tmp/working_dir"
    }}"#
    );
    storage
        .write_byte_stream(&paths.lineage(), lineage_json.as_bytes().to_vec().into())
        .await?;

    let package_home = home.join(namespace.to_string());
    storage.create_dir_all(&package_home).await?;
    storage
        .write_byte_stream(
            package_home.join("data.txt"),
            ByteStream::from_static(b"hello world"),
        )
        .await?;

    let domain_lineage_io = DomainLineageIo::new(paths.lineage());
    let package = InstalledPackage {
        lineage: PackageLineageIo::new(domain_lineage_io, namespace.clone()),
        paths,
        remote,
        storage,
        namespace: namespace.clone(),
    };

    package
        .commit(
            "Initial commit".to_string(),
            UserMeta::Set(serde_json::json!({"key": "value"})),
            None,
            None,
        )
        .await?;

    package
        .set_remote(
            "my-bucket".to_string(),
            Some("example.com".parse()?),
            intent,
        )
        .await?;

    let lineage = package.lineage().await?;
    let new_commit = lineage.commit.as_ref().expect("commit should exist");
    let manifest_path = package
        .paths
        .installed_manifest(&namespace, &new_commit.hash);
    let manifest = Manifest::from_path(&package.storage, &manifest_path).await?;
    Ok(manifest.header)
}

/// Build a locally-committed package against a bucket whose config declares a
/// `foo` workflow but no `default_workflow`, ready for a `set_remote` call. The
/// returned temp-dir guards must be kept alive for the package's storage to
/// remain valid.
async fn package_with_foo_config(
    slug: &str,
) -> Res<(
    InstalledPackage<LocalStorage, MockRemote>,
    tempfile::TempDir,
    tempfile::TempDir,
)> {
    let (home, temp_dir1) = Home::from_temp_dir()?;
    let (paths, temp_dir2) = DomainPaths::from_temp_dir()?;

    let storage = LocalStorage::new();
    let remote = MockRemote::default();
    let namespace: Namespace = ("test", slug).into();

    paths
        .scaffold_for_installing(&storage, &home, &namespace)
        .await?;

    // The target bucket declares `foo` but no `default_workflow`.
    let config_uri: S3Uri = "s3://my-bucket/.quilt/workflows/config.yml".parse()?;
    let config = r"
workflows:
  foo:
    metadata_schema: bar
schemas:
  bar:
    url: s3://my-bucket/schemas/test.json
";
    let schema_uri: S3Uri = "s3://my-bucket/schemas/test.json".parse()?;
    remote
        .put_object(&None, &config_uri, config.as_bytes().to_vec())
        .await?;
    remote
        .put_object(&None, &schema_uri, b"{}".to_vec())
        .await?;

    let lineage_json = format!(
        r#"{{
        "packages": {{
            "test/{slug}": {{
                "commit": null,
                "remote": null,
                "base_hash": "",
                "latest_hash": "",
                "paths": {{}}
            }}
        }},
        "home": "/tmp/working_dir"
    }}"#
    );
    storage
        .write_byte_stream(&paths.lineage(), lineage_json.as_bytes().to_vec().into())
        .await?;

    let package_home = home.join(namespace.to_string());
    storage.create_dir_all(&package_home).await?;
    storage
        .write_byte_stream(
            package_home.join("data.txt"),
            ByteStream::from_static(b"hello world"),
        )
        .await?;

    let domain_lineage_io = DomainLineageIo::new(paths.lineage());
    let package = InstalledPackage {
        lineage: PackageLineageIo::new(domain_lineage_io, namespace.clone()),
        paths,
        remote,
        storage,
        namespace,
    };

    package
        .commit(
            "Initial commit".to_string(),
            UserMeta::Set(serde_json::json!({"key": "value"})),
            None,
            None,
        )
        .await?;

    Ok((package, temp_dir1, temp_dir2))
}

#[test(tokio::test)]
async fn test_set_remote_propagates_named_workflow_error() -> Res {
    // An explicit `Named` gesture whose id isn't in the bucket config must make
    // `set_remote` fail loudly rather than silently swallowing the recommit
    // error (the user's workflow choice would otherwise be dropped).
    let (package, _t1, _t2) = package_with_foo_config("named-error").await?;

    let result = package
        .set_remote(
            "my-bucket".to_string(),
            Some("example.com".parse()?),
            WorkflowIntent::Named("nope".to_string()),
        )
        .await;

    assert!(
        result.is_err(),
        "an explicit Named intent with an unknown id must surface the recommit error"
    );
    assert!(
        result.unwrap_err().to_string().contains("Workflow nope"),
        "error should name the unresolved workflow"
    );

    Ok(())
}

#[test(tokio::test)]
async fn test_set_remote_swallows_bucket_default_recommit_error() -> Res {
    // The no-gesture `BucketDefault` path stays best-effort: even against the
    // same config (with no `default_workflow`), `set_remote` succeeds — the
    // remote is saved and any recommit hiccup is only logged.
    let (package, _t1, _t2) = package_with_foo_config("bucketdefault-ok").await?;

    package
        .set_remote(
            "my-bucket".to_string(),
            Some("example.com".parse()?),
            WorkflowIntent::BucketDefault,
        )
        .await?;

    let lineage = package.lineage().await?;
    assert!(
        lineage.remote_uri.is_some(),
        "remote should be persisted on the BucketDefault path"
    );

    Ok(())
}

#[test(tokio::test)]
async fn test_set_remote_stamps_named_workflow() -> Res {
    // `Named("foo")` must stamp `foo` even though the bucket declares no default.
    let header =
        recommit_manifest_for_intent("named", WorkflowIntent::Named("foo".to_string())).await?;

    let workflow = header
        .workflow
        .expect("recommit should stamp the named workflow");
    assert_eq!(
        workflow.id.expect("workflow id should be set").id,
        "foo",
        "recommit should stamp the caller's chosen workflow, not the bucket default"
    );

    Ok(())
}

#[test(tokio::test)]
async fn test_set_remote_stamps_no_workflow() -> Res {
    // `NoWorkflow` must produce an explicit id-less record when a config exists.
    let header = recommit_manifest_for_intent("noworkflow", WorkflowIntent::NoWorkflow).await?;

    let workflow = header
        .workflow
        .expect("recommit should stamp an id-less workflow when a config is present");
    assert!(
        workflow.id.is_none(),
        "NoWorkflow must not resolve any workflow id"
    );

    Ok(())
}
