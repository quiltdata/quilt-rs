//! Tests for the commit/push/pull/status lifecycle of an installed package.

use super::*;

use test_log::test;

use aws_sdk_s3::primitives::ByteStream;

use crate::io::remote::mocks::MockRemote;
use crate::io::storage::StorageExt;
use crate::lineage::DomainLineageIo;
use crate::lineage::Home;
use crate::lineage::PackageLineageIo;
use crate::paths::DomainPaths;

#[test(tokio::test)]
async fn test_spamming_commit_writes() -> Res {
    let (home, _temp_dir1) = Home::from_temp_dir()?;
    let (paths, _temp_dir2) = DomainPaths::from_temp_dir()?;

    let storage = LocalStorage::new();
    let remote = MockRemote::default();
    let namespace: Namespace = ("test", "history").into();
    let test_hash = "deadbeef".to_string();

    paths
        .scaffold_for_installing(&storage, &home, &namespace)
        .await?;
    // Initialize domain lineage file
    let lineage_json = format!(
        r#"{{
            "packages": {{
                "test/history": {{
                    "commit": null,
                    "remote": {{
                        "bucket": "bucket",
                        "namespace": "test/history",
                        "hash": "{}",
                        "catalog": "test.quilt.dev"
                    }},
                    "base_hash": "{}",
                    "latest_hash": "{}",
                    "paths": {{}}
                }}}},
            "home": "/tmp/working_dir"
            }}"#,
        test_hash, "foo", "bar"
    );
    storage
        .write_byte_stream(&paths.lineage(), lineage_json.as_bytes().to_vec().into())
        .await?;

    // Copy manifest to the expected path
    let test_manifest_path = paths.installed_manifest(&namespace, &test_hash);
    let test_manifest = r#"{"version": "v0"}"#;
    storage
        .write_byte_stream(
            &test_manifest_path,
            ByteStream::from_static(test_manifest.as_bytes()),
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

    // Make 10 commits with different content
    let mut expected_hashes = Vec::new();
    for i in 0..10 {
        let commit = package
            .commit(
                format!("Commit new1 {i}"),
                UserMeta::Set(serde_json::json!({ "count": i })),
                None,
                None,
            )
            .await?;
        expected_hashes.insert(i, commit.hash);
    }

    // Remove last, cause it's the "current" hash, not a part of `prev_hashes`
    expected_hashes.pop();

    let commit_state = package.lineage().await?.commit.unwrap();

    assert_eq!(commit_state.prev_hashes.len(), 9);
    // let hashes_to_assert: Vec<String> = expected_hashes.into_iter().rev().collect();
    assert_eq!(
        commit_state.prev_hashes,
        expected_hashes.into_iter().rev().collect::<Vec<String>>()
    );

    Ok(())
}

/// Scenario A: diverged with an unpushed local commit. The user lands on
/// the merge page because someone else moved `latest` past our install
/// base, and we have a local commit on top of that base. `certify_latest`
/// must push our commit and then tag the resulting remote hash as
/// `latest` — not roll the tag back to the install-time hash.
#[test(tokio::test)]
async fn test_certify_latest_pushes_pending_commit_then_tags() -> Res {
    let (home, _temp_dir1) = Home::from_temp_dir()?;
    let (paths, _temp_dir2) = DomainPaths::from_temp_dir()?;

    let storage = LocalStorage::new();
    let remote = MockRemote::default();
    let namespace: Namespace = ("test", "diverged").into();
    let bucket = "b";

    // L is the rebuilt-manifest hash that push will produce from an empty
    // manifest with header user_meta=null. Using a known fixture keeps
    // push's "rebuilt hash must equal commit hash" check happy without
    // wiring real objects.
    let local_hash = crate::fixtures::top_hash::EMPTY_NULL_TOP_HASH;
    let install_hash = "I_HASH";
    let other_hash = "N_HASH";

    paths
        .scaffold_for_installing(&storage, &home, &namespace)
        .await?;
    paths.scaffold_for_caching(&storage, bucket).await?;

    let lineage_json = format!(
        r#"{{
            "packages": {{
                "test/diverged": {{
                    "commit": {{
                        "timestamp": "2024-01-01T00:00:00Z",
                        "hash": "{local_hash}",
                        "prev_hashes": []
                    }},
                    "remote": {{
                        "bucket": "{bucket}",
                        "namespace": "test/diverged",
                        "hash": "{install_hash}"
                    }},
                    "base_hash": "{install_hash}",
                    "latest_hash": "{other_hash}",
                    "paths": {{}}
                }}
            }},
            "home": "/tmp/working_dir"
        }}"#
    );
    storage
        .write_byte_stream(&paths.lineage(), lineage_json.as_bytes().to_vec().into())
        .await?;

    // Local installed manifest at the commit hash — push reads it via
    // `self.manifest()`. These exact bytes are what make the rebuild
    // produce `EMPTY_NULL_TOP_HASH` (matching `local_hash` above); any
    // change to the manifest serialization or top-hash algorithm will
    // surface as a push-side "rebuilt hash != commit hash" failure
    // rather than at the final `latest_body` assertion.
    let local_manifest = b"{\"version\":\"v0\",\"message\":\"\",\"user_meta\":null}\n".to_vec();
    storage
        .write_byte_stream(
            &paths.installed_manifest(&namespace, local_hash),
            local_manifest.clone().into(),
        )
        .await?;

    // Pre-cache the install-time remote manifest so push's `flow::browse`
    // call for the previous remote_uri succeeds without a remote round-trip.
    let install_manifest_uri = ManifestUri {
        bucket: bucket.to_string(),
        namespace: namespace.clone(),
        hash: install_hash.to_string(),
        origin: None,
    };
    storage
        .write_byte_stream(
            paths.cached_manifest(&install_manifest_uri),
            local_manifest.into(),
        )
        .await?;

    // Remote `latest` tag points at someone else's hash — this is what
    // makes the state Diverged.
    remote
        .put_object(
            &None,
            &S3Uri::try_from(
                format!("s3://{bucket}/.quilt/named_packages/test/diverged/latest").as_str(),
            )?,
            other_hash.as_bytes().to_vec(),
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

    package.certify_latest().await?;

    // Remote `latest` now points at the user's revision (L), not the
    // teammate's (N) and not the install-time hash (I).
    let latest_uri = S3Uri::try_from(
        format!("s3://{bucket}/.quilt/named_packages/test/diverged/latest").as_str(),
    )?;
    let latest_body = package
        .remote
        .get_object_stream(&None, &latest_uri)
        .await?
        .body
        .collect()
        .await?
        .to_vec();
    assert_eq!(latest_body, local_hash.as_bytes());

    let lineage = package.lineage().await?;
    assert_eq!(lineage.base_hash, local_hash);
    assert_eq!(lineage.latest_hash, local_hash);
    assert!(lineage.commit.is_none(), "push should have consumed commit");

    Ok(())
}

/// Scenario B: diverged because our prior push uploaded the manifest but
/// `push_package` declined to certify (`latest` had moved between
/// `base_hash` and our push). With `commit = None`, `certify_latest`
/// must skip the push and tag the already-pushed hash as `latest`.
#[test(tokio::test)]
async fn test_certify_latest_skips_push_when_no_pending_commit() -> Res {
    let (home, _temp_dir1) = Home::from_temp_dir()?;
    let (paths, _temp_dir2) = DomainPaths::from_temp_dir()?;

    let storage = LocalStorage::new();
    let remote = MockRemote::default();
    let namespace: Namespace = ("test", "pushed").into();
    let bucket = "b";

    let pushed_hash = "X_HASH";
    let other_hash = "Y_HASH";

    paths
        .scaffold_for_installing(&storage, &home, &namespace)
        .await?;
    paths.scaffold_for_caching(&storage, bucket).await?;

    let lineage_json = format!(
        r#"{{
            "packages": {{
                "test/pushed": {{
                    "commit": null,
                    "remote": {{
                        "bucket": "{bucket}",
                        "namespace": "test/pushed",
                        "hash": "{pushed_hash}"
                    }},
                    "base_hash": "{pushed_hash}",
                    "latest_hash": "{other_hash}",
                    "paths": {{}}
                }}
            }},
            "home": "/tmp/working_dir"
        }}"#
    );
    storage
        .write_byte_stream(&paths.lineage(), lineage_json.as_bytes().to_vec().into())
        .await?;

    // Remote `latest` tag currently points at someone else's hash.
    remote
        .put_object(
            &None,
            &S3Uri::try_from(
                format!("s3://{bucket}/.quilt/named_packages/test/pushed/latest").as_str(),
            )?,
            other_hash.as_bytes().to_vec(),
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

    package.certify_latest().await?;

    // Remote `latest` now points at the previously-pushed revision (X).
    let latest_uri = S3Uri::try_from(
        format!("s3://{bucket}/.quilt/named_packages/test/pushed/latest").as_str(),
    )?;
    let latest_body = package
        .remote
        .get_object_stream(&None, &latest_uri)
        .await?
        .body
        .collect()
        .await?
        .to_vec();
    assert_eq!(latest_body, pushed_hash.as_bytes());

    // No manifest was uploaded as part of certification — push was skipped.
    assert!(
        !package
            .remote
            .exists(
                &None,
                &S3Uri::try_from(format!("s3://{bucket}/.quilt/packages/{pushed_hash}").as_str(),)?,
            )
            .await?,
        "push should be skipped when there is no pending commit",
    );

    let lineage = package.lineage().await?;
    assert_eq!(lineage.base_hash, pushed_hash);
    assert_eq!(lineage.latest_hash, pushed_hash);
    assert!(lineage.commit.is_none());

    Ok(())
}

#[test(tokio::test)]
async fn test_manifest_recovery_from_corruption() -> Res {
    let (home, _temp_dir1) = Home::from_temp_dir()?;
    let (paths, _temp_dir2) = DomainPaths::from_temp_dir()?;

    let storage = LocalStorage::new();
    let remote = MockRemote::default();
    let namespace: Namespace = ("test", "recovery").into();
    let test_hash = "deadbeef".to_string();

    paths
        .scaffold_for_installing(&storage, &home, &namespace)
        .await?;
    paths.scaffold_for_caching(&storage, "test-bucket").await?;

    // Initialize domain lineage file
    let lineage_json = format!(
        r#"{{
            "packages": {{
                "test/recovery": {{
                    "commit": null,
                    "remote": {{
                        "bucket": "test-bucket",
                        "namespace": "test/recovery",
                        "hash": "{}",
                        "catalog": null
                    }},
                    "base_hash": "{}",
                    "latest_hash": "{}",
                    "paths": {{}}
                }}}},
            "home": "/tmp/working_dir"
            }}"#,
        test_hash, "foo", "bar"
    );
    storage
        .write_byte_stream(&paths.lineage(), lineage_json.as_bytes().to_vec().into())
        .await?;

    // Set up a valid cached manifest
    let reference_manifest = crate::fixtures::manifest::path();
    let manifest_uri = ManifestUri {
        bucket: "test-bucket".to_string(),
        namespace: namespace.clone(),
        hash: test_hash.clone(),
        origin: None,
    };
    let cached_manifest = paths.cached_manifest(&manifest_uri);
    storage.copy(reference_manifest?, cached_manifest).await?;

    // Create a corrupted installed manifest
    let installed_manifest = paths.installed_manifest(&namespace, &test_hash);
    storage
        .write_byte_stream(
            &installed_manifest,
            ByteStream::from_static(b"corrupted data"),
        )
        .await?;

    let domain_lineage_io = DomainLineageIo::new(paths.lineage());
    let package = InstalledPackage {
        lineage: PackageLineageIo::new(domain_lineage_io, namespace.clone()),
        paths,
        remote,
        storage: storage.clone(),
        namespace,
    };

    // This should succeed by recovering from cache despite corrupted installed manifest
    let result = package.manifest().await;
    assert!(
        result.is_ok(),
        "Should recover from cache when installed is corrupted"
    );

    // Verify the corrupted file was replaced with good data
    let fixed_manifest_content = storage.read_bytes(&installed_manifest).await?;
    assert!(
        fixed_manifest_content.len() > 10,
        "Installed manifest should be fixed"
    );
    assert!(
        !fixed_manifest_content.starts_with(b"corrupted"),
        "Should no longer be corrupted"
    );

    Ok(())
}

/// A remote that always returns `LoginRequired`, simulating a logged-out user.
struct LoggedOutRemote;

impl crate::io::remote::Remote for LoggedOutRemote {
    async fn exists(&self, _host: &Option<Host>, _s3_uri: &S3Uri) -> Res<bool> {
        Err(Error::Login(LoginError::Required(None)))
    }
    async fn get_object_stream(
        &self,
        _host: &Option<Host>,
        _s3_uri: &S3Uri,
    ) -> Res<crate::io::remote::RemoteObjectStream> {
        Err(Error::Login(LoginError::Required(None)))
    }
    async fn resolve_url(&self, _host: &Option<Host>, _s3_uri: &S3Uri) -> Res<S3Uri> {
        Err(Error::Login(LoginError::Required(None)))
    }
    async fn put_object(
        &self,
        _host: &Option<Host>,
        _s3_uri: &S3Uri,
        _contents: impl Into<aws_sdk_s3::primitives::ByteStream>,
    ) -> Res {
        Err(Error::Login(LoginError::Required(None)))
    }
    async fn upload_file(
        &self,
        _host_config: &crate::io::remote::HostConfig,
        _source_path: impl AsRef<std::path::Path>,
        _dest_uri: &S3Uri,
        _size: u64,
    ) -> Res<(S3Uri, crate::checksum::ObjectHash)> {
        Err(Error::Login(LoginError::Required(None)))
    }
    async fn host_config(&self, _host: &Option<Host>) -> Res<crate::io::remote::HostConfig> {
        Ok(crate::io::remote::HostConfig::default())
    }
    async fn verify_bucket(&self, _bucket: &str) -> Res {
        Ok(())
    }
}

#[test(tokio::test)]
async fn test_status_propagates_login_required() -> Res {
    let (home, _temp_dir1) = Home::from_temp_dir()?;
    let (paths, _temp_dir2) = DomainPaths::from_temp_dir()?;

    let storage = LocalStorage::new();
    let namespace: Namespace = ("test", "needslogin").into();

    paths
        .scaffold_for_installing(&storage, &home, &namespace)
        .await?;

    // Package with remote configured but never pushed (empty hash)
    let lineage_json = r#"{
        "packages": {
            "test/needslogin": {
                "commit": null,
                "remote": {
                    "bucket": "my-bucket",
                    "namespace": "test/needslogin",
                    "hash": "",
                    "origin": "nightly.quilttest.com"
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
        remote: LoggedOutRemote,
        storage,
        namespace,
    };

    // status() should propagate LoginRequired so the UI can show a Login button
    let result = package.status(None).await;
    assert!(
        matches!(result, Err(Error::Login(LoginError::Required(_)))),
        "Expected LoginRequired error, got: {result:?}"
    );

    Ok(())
}

/// Pull must refresh `latest_hash` from the remote before evaluating
/// `flow::pull`'s `base_hash == latest_hash` guard. Before the
/// "Stop writing lineage from `InstalledPackage::status`" refactor, a
/// prior `status` call would persist the refreshed `latest_hash`, so
/// disk was reliably fresh when `pull` ran. Without that persist,
/// the disk-stale `latest_hash` always equalled `base_hash` and
/// pull short-circuited with "already up-to-date" — both the
/// autosync watcher's pull branch and the manual Pull button were
/// affected.
#[test(tokio::test)]
async fn test_pull_refreshes_latest_hash_when_remote_moved() -> Res {
    let (home, _temp_dir1) = Home::from_temp_dir()?;
    let (paths, _temp_dir2) = DomainPaths::from_temp_dir()?;
    let storage = LocalStorage::new();
    let remote = MockRemote::default();
    let namespace: Namespace = ("test", "pull_refresh").into();
    let bucket = "bkt";
    let install_hash = "INSTALL_HASH";
    let new_hash = "NEW_HASH";

    paths
        .scaffold_for_installing(&storage, &home, &namespace)
        .await?;

    // Disk lineage at the install-time hash: latest_hash == base_hash
    // == remote.hash. Without the refresh inside `pull` this state
    // alone short-circuits the up-to-date guard.
    let lineage_json = format!(
        r#"{{
            "packages": {{
                "test/pull_refresh": {{
                    "commit": null,
                    "remote": {{
                        "bucket": "{bucket}",
                        "namespace": "test/pull_refresh",
                        "hash": "{install_hash}",
                        "catalog": "test.quilt.dev"
                    }},
                    "base_hash": "{install_hash}",
                    "latest_hash": "{install_hash}",
                    "paths": {{}}
                }}
            }},
            "home": "{}"
        }}"#,
        home.as_ref().display(),
    );
    storage
        .write_byte_stream(&paths.lineage(), lineage_json.as_bytes().to_vec().into())
        .await?;

    // Installed manifest at the install-time hash so `package.manifest()`
    // can resolve.
    storage
        .write_byte_stream(
            paths.installed_manifest(&namespace, install_hash),
            ByteStream::from_static(br#"{"version": "v0"}"#),
        )
        .await?;

    // Remote `latest` tag has moved past the install — this is the
    // exact state that broke after the read-only-status refactor.
    remote
        .put_object(
            &None,
            &S3Uri::try_from(
                format!("s3://{bucket}/.quilt/named_packages/test/pull_refresh/latest").as_str(),
            )?,
            new_hash.as_bytes().to_vec(),
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

    // Pull will eventually fail downstream — we have not staged the
    // manifest at `new_hash`, so `cache_remote_manifest` will return a
    // NotFound — but the *specific* error we are guarding against is
    // "package is already up-to-date". Any other failure mode proves
    // the refresh-then-check path ran.
    let err = package
        .pull(None)
        .await
        .expect_err("pull should fail downstream on the missing NEW_HASH manifest");
    let msg = err.to_string();
    assert!(
        !msg.contains("already up-to-date"),
        "pull must refresh latest_hash before the up-to-date guard; got: {msg}"
    );

    Ok(())
}
