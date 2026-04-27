use std::collections::BTreeMap;
use std::path::PathBuf;

use tracing::log;

use crate::error::LoginError;
use crate::error::PackageOpError;
use crate::flow;
use crate::flow::cache_remote_manifest;
use crate::io::remote::resolve_workflow;
use crate::io::remote::HostConfig;
use crate::io::remote::Remote;
use crate::io::remote::RemoteS3;
use crate::io::storage::LocalStorage;
use crate::io::storage::Storage;
use crate::lineage;
use crate::lineage::CommitState;
use crate::lineage::InstalledPackageStatus;
use crate::lineage::LineagePaths;
use crate::manifest::Manifest;
use crate::manifest::Workflow;
use crate::paths;
use crate::paths::copy_cached_to_installed;
use crate::Error;
use crate::Res;
use quilt_uri::Host;
use quilt_uri::ManifestUri;
use quilt_uri::Namespace;
use quilt_uri::S3Uri;
use quilt_uri::UriError;

/// Result of a push operation visible to callers outside `quilt-rs`.
pub struct PushOutcome {
    pub manifest_uri: ManifestUri,
    /// Whether the pushed revision was certified as "latest".
    /// `false` when the remote's latest tag moved since we last checked
    /// (i.e. someone else pushed in the meantime).
    pub certified_latest: bool,
}

/// Result of a publish operation visible to callers outside `quilt-rs`.
/// Alias of [`flow::PublishOutcome`] parameterized over the public
/// [`PushOutcome`], so external callers see a non-generic type name.
pub type PublishOutcome = flow::PublishOutcome<PushOutcome>;

/// Similar to `LocalDomain` because it has access to the same lineage file and remote/storage
/// traits.
/// But it only manages one particular installed package.
/// It can be instantiated from `LocalDomain` by installing new or listing existing packages.
#[derive(Debug)]
pub struct InstalledPackage<S: Storage = LocalStorage, R: Remote = RemoteS3> {
    pub lineage: lineage::PackageLineageIo,
    pub paths: paths::DomainPaths,
    pub remote: R,
    pub storage: S,
    pub namespace: Namespace,
}

impl std::fmt::Display for InstalledPackage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, r##"Installed package "{}""##, self.namespace)
    }
}

impl<S: Storage + Sync, R: Remote> InstalledPackage<S, R> {
    pub async fn scaffold_paths(&self) -> Res {
        let home = self.lineage.domain_home(&self.storage).await?;
        self.paths
            .scaffold_for_installing(&self.storage, &home, &self.namespace)
            .await
    }

    pub async fn scaffold_paths_for_caching(&self, bucket: &str) -> Res {
        self.paths.scaffold_for_caching(&self.storage, bucket).await
    }

    pub async fn manifest(&self) -> Res<Manifest> {
        let (_, lineage) = self.lineage.read(&self.storage).await?;
        let hash = match lineage.current_hash() {
            Some(h) => h,
            None => return Ok(Manifest::default()),
        };
        let installed_path = self.paths.installed_manifest(&self.namespace, hash);
        match Manifest::from_path(&self.storage, &installed_path).await {
            Ok(manifest) => return Ok(manifest),

            Err(e) => {
                log::warn!(
                    "Failed to read installed manifest at {}: {}",
                    installed_path.display(),
                    e
                );
            }
        }

        // If installed failed, try to recover from cache (only if we have a remote)
        match lineage.remote_uri.as_ref() {
            Some(remote_uri) => {
                log::info!("Attempting to recover from cache at {}", remote_uri);
                let cached_manifest =
                    cache_remote_manifest(&self.paths, &self.storage, &self.remote, remote_uri)
                        .await?;
                copy_cached_to_installed(&self.paths, &self.storage, remote_uri).await?;
                Ok(cached_manifest)
            }
            None => Err(Error::Uri(UriError::ManifestPath(
                "No installed manifest and no remote to recover from".to_string(),
            ))),
        }
    }

    pub async fn lineage(&self) -> Res<lineage::PackageLineage> {
        let (_, lineage) = self.lineage.read(&self.storage).await?;
        Ok(lineage)
    }

    pub async fn package_home(&self) -> Res<PathBuf> {
        self.lineage.package_home(&self.storage).await
    }

    pub async fn status(&self, host_config_opt: Option<HostConfig>) -> Res<InstalledPackageStatus> {
        let (package_home, lineage) = self.lineage.read(&self.storage).await?;

        // Only refresh latest hash if we have a remote
        let lineage = match lineage.remote_uri.as_ref() {
            Some(_) => match flow::refresh_latest_hash(lineage.clone(), &self.remote).await {
                Ok(lineage) => lineage,
                Err(Error::Login(LoginError::Required(_))) => {
                    return Err(Error::Login(LoginError::Required(
                        lineage.remote_uri.as_ref().and_then(|r| r.origin.clone()),
                    )))
                }
                Err(err) => {
                    log::warn!("Failed to refresh latest hash: {err}");
                    lineage
                }
            },
            None => lineage,
        };
        let manifest = self.manifest().await?;

        let host_config = match host_config_opt {
            Some(hc) => hc,
            None => match lineage.remote_uri.as_ref() {
                Some(remote_uri) if !remote_uri.bucket.is_empty() => {
                    self.remote.host_config(&remote_uri.origin).await?
                }
                _ => HostConfig::default(),
            },
        };

        let (lineage, status) = flow::status(
            lineage,
            &self.storage,
            &manifest,
            &package_home,
            host_config,
        )
        .await?;
        self.lineage.write(&self.storage, lineage).await?;
        Ok(status)
    }

    pub async fn install_paths(&self, paths: &[PathBuf]) -> Res<LineagePaths> {
        if paths.is_empty() {
            return Ok(BTreeMap::new());
        }

        self.scaffold_paths().await?;

        let (package_home, lineage) = self.lineage.read(&self.storage).await?;
        let remote_uri = lineage.remote()?;

        self.scaffold_paths_for_caching(&remote_uri.bucket).await?;

        let mut manifest = self.manifest().await?;
        let lineage = flow::install_paths(
            lineage,
            &mut manifest,
            &self.paths,
            package_home,
            self.namespace.clone(),
            &self.storage,
            &self.remote,
            &paths.iter().collect::<Vec<&PathBuf>>(),
        )
        .await?;
        let lineage = self.lineage.write(&self.storage, lineage).await?;
        Ok(lineage.paths)
    }

    pub async fn uninstall_paths(&self, paths: &Vec<PathBuf>) -> Res<LineagePaths> {
        let (package_home, lineage) = self.lineage.read(&self.storage).await?;
        let lineage = flow::uninstall_paths(lineage, package_home, &self.storage, paths).await?;
        let lineage = self.lineage.write(&self.storage, lineage).await?;
        Ok(lineage.paths)
    }

    pub async fn revert_paths(&self, paths: &Vec<String>) -> Res {
        log::debug!("revert_paths: {paths:?}");
        unimplemented!()
    }

    pub async fn commit(
        &self,
        message: String,
        user_meta: Option<serde_json::Value>,
        workflow: Option<Workflow>,
        host_config_opt: Option<HostConfig>,
    ) -> Res<CommitState> {
        self.scaffold_paths().await?;

        let (package_home, lineage) = self.lineage.read(&self.storage).await?;
        let mut manifest = self.manifest().await?;

        let host_config = match host_config_opt {
            Some(hc) => hc,
            None => match lineage.remote_uri.as_ref() {
                Some(remote_uri) if !remote_uri.bucket.is_empty() => {
                    self.remote.host_config(&remote_uri.origin).await?
                }
                _ => HostConfig::default(),
            },
        };

        let (lineage, status) = flow::status(
            lineage,
            &self.storage,
            &manifest,
            &package_home,
            host_config,
        )
        .await?;

        let (lineage, commit) = flow::commit(
            lineage,
            &mut manifest,
            &self.paths,
            &self.storage,
            package_home,
            status,
            self.namespace.clone(),
            message,
            user_meta,
            workflow,
        )
        .await?;
        self.lineage.write(&self.storage, lineage).await?;
        Ok(commit)
    }

    /// Commit any working-directory changes (if any) and push the revision to
    /// the remote in one step. Errors if the package has no remote or nothing
    /// to publish.
    ///
    /// `status_opt` is a caller-provided cache of `flow::status`: when
    /// `Some`, this method reuses it verbatim instead of re-scanning the
    /// working tree. The caller must ensure the status was computed from the
    /// same on-disk lineage and manifest that `publish` will re-read — i.e.
    /// nothing else should have mutated this package between the two calls.
    /// Passing `None` is always safe and falls back to an internal
    /// `flow::status` call.
    pub async fn publish(
        &self,
        message: String,
        user_meta: Option<serde_json::Value>,
        workflow: Option<Workflow>,
        host_config_opt: Option<HostConfig>,
        status_opt: Option<InstalledPackageStatus>,
    ) -> Res<PublishOutcome> {
        self.scaffold_paths().await?;

        let (package_home, lineage) = self.lineage.read(&self.storage).await?;
        let remote_uri = match lineage.remote_uri.as_ref() {
            Some(uri) if !uri.bucket.is_empty() => uri.clone(),
            Some(_) => {
                return Err(Error::PackageOp(PackageOpError::Publish(
                    "Remote bucket not set. Use set_remote first.".to_string(),
                )))
            }
            None => {
                return Err(Error::PackageOp(PackageOpError::Publish(
                    "No remote configured. Use set_remote first.".to_string(),
                )))
            }
        };

        self.scaffold_paths_for_caching(&remote_uri.bucket).await?;

        let mut manifest = self.manifest().await?;
        let host_config =
            host_config_opt.unwrap_or(self.remote.host_config(&remote_uri.origin).await?);

        let (lineage, status) = match status_opt {
            Some(status) => (lineage, status),
            None => {
                flow::status(
                    lineage,
                    &self.storage,
                    &manifest,
                    &package_home,
                    host_config.clone(),
                )
                .await?
            }
        };

        let outcome = flow::publish(
            lineage,
            &mut manifest,
            &self.paths,
            &self.storage,
            &self.remote,
            package_home,
            status,
            self.namespace.clone(),
            host_config,
            flow::CommitOptions {
                message,
                user_meta,
                workflow,
            },
        )
        .await?;

        let (committed, push_result) = match outcome {
            flow::PublishOutcome::CommittedAndPushed(p) => (true, p),
            flow::PublishOutcome::PushedOnly(p) => (false, p),
        };
        let certified_latest = push_result.certified_latest;
        let lineage = self
            .lineage
            .write(&self.storage, push_result.lineage)
            .await?;
        let push = PushOutcome {
            manifest_uri: lineage.remote()?.clone(),
            certified_latest,
        };
        Ok(if committed {
            PublishOutcome::CommittedAndPushed(push)
        } else {
            PublishOutcome::PushedOnly(push)
        })
    }

    /// Push the local revision to the remote.
    pub async fn push(&self, host_config_opt: Option<HostConfig>) -> Res<PushOutcome> {
        self.scaffold_paths().await?;

        let (_, lineage) = self.lineage.read(&self.storage).await?;
        let remote_uri = match lineage.remote_uri.as_ref() {
            Some(uri) if !uri.bucket.is_empty() => uri.clone(),
            Some(_) => {
                return Err(Error::PackageOp(PackageOpError::Push(
                    "Remote bucket not set. Use set_remote first.".to_string(),
                )))
            }
            None => {
                return Err(Error::PackageOp(PackageOpError::Push(
                    "No remote configured. Use set_remote first.".to_string(),
                )))
            }
        };

        if lineage.commit.is_none() {
            return Err(Error::PackageOp(PackageOpError::Push(
                "No commits to push".to_string(),
            )));
        }

        self.scaffold_paths_for_caching(&remote_uri.bucket).await?;

        let manifest = self.manifest().await?;

        let host_config =
            host_config_opt.unwrap_or(self.remote.host_config(&remote_uri.origin).await?);

        let result = flow::push(
            lineage,
            manifest,
            &self.paths,
            &self.storage,
            &self.remote,
            Some(self.namespace.clone()),
            host_config,
        )
        .await?;
        let certified_latest = result.certified_latest;
        let lineage = self.lineage.write(&self.storage, result.lineage).await?;
        Ok(PushOutcome {
            manifest_uri: lineage.remote()?.clone(),
            certified_latest,
        })
    }

    pub async fn pull(&self, host_config_opt: Option<HostConfig>) -> Res<ManifestUri> {
        self.scaffold_paths().await?;

        let (package_home, lineage) = self.lineage.read(&self.storage).await?;
        let remote_uri = lineage.remote()?.clone();

        self.scaffold_paths_for_caching(&remote_uri.bucket).await?;

        let mut manifest = self.manifest().await?;

        let host_config =
            host_config_opt.unwrap_or(self.remote.host_config(&remote_uri.origin).await?);

        let (lineage, status) = flow::status(
            lineage,
            &self.storage,
            &manifest,
            &package_home,
            host_config,
        )
        .await?;
        let lineage = flow::pull(
            lineage,
            &mut manifest,
            &self.paths,
            &self.storage,
            &self.remote,
            package_home,
            status,
            self.namespace.clone(),
        )
        .await?;
        let lineage = self.lineage.write(&self.storage, lineage).await?;
        Ok(lineage.remote()?.clone())
    }

    pub async fn certify_latest(&self) -> Res<ManifestUri> {
        let (_, lineage) = self.lineage.read(&self.storage).await?;
        let latest_manifest_uri = lineage.remote()?.clone();
        let lineage = flow::certify_latest(lineage, &self.remote, latest_manifest_uri).await?;
        let lineage = self.lineage.write(&self.storage, lineage).await?;
        Ok(lineage.remote()?.clone())
    }

    pub async fn reset_to_latest(&self) -> Res<ManifestUri> {
        self.scaffold_paths().await?;

        let (package_home, lineage) = self.lineage.read(&self.storage).await?;
        let remote_uri = lineage.remote()?.clone();

        self.scaffold_paths_for_caching(&remote_uri.bucket).await?;

        let mut manifest = self.manifest().await?;
        let lineage = flow::reset_to_latest(
            lineage,
            &mut manifest,
            &self.paths,
            &self.storage,
            &self.remote,
            package_home,
            self.namespace.clone(),
        )
        .await?;
        let lineage = self.lineage.write(&self.storage, lineage).await?;
        Ok(lineage.remote()?.clone())
    }

    pub async fn set_remote(&self, bucket: String, origin: Option<Host>) -> Res {
        if bucket.is_empty() {
            return Err(Error::PackageOp(PackageOpError::Push(
                "Bucket cannot be empty".to_string(),
            )));
        }
        let (_, mut lineage) = self.lineage.read(&self.storage).await?;
        if let Some(existing) = &lineage.remote_uri {
            if !existing.hash.is_empty() {
                let same_remote = existing.bucket == bucket && existing.origin == origin;
                if same_remote {
                    return Ok(());
                }
                return Err(Error::PackageOp(PackageOpError::Push(
                    "Cannot change remote on a package that has already been pushed".to_string(),
                )));
            }
        }
        // Validate the bucket up front so a typo surfaces here instead of
        // later at push time as an opaque S3 routing error. This is an
        // unauthenticated HEAD against s3.amazonaws.com — works even when
        // the user hasn't logged into the catalog yet.
        self.remote.verify_bucket(&bucket).await?;
        lineage.remote_uri = Some(ManifestUri {
            origin: origin.clone(),
            bucket: bucket.clone(),
            namespace: self.namespace.clone(),
            hash: String::new(),
        });
        // Persist remote_uri first — if recommit fails (e.g. network error),
        // the remote is still saved and the user can retry.
        self.lineage.write(&self.storage, lineage.clone()).await?;

        // Re-commit with the remote's host_config and workflow so push
        // works immediately without a manual re-commit.
        // This can fail (e.g. not logged in yet) — the remote is already saved,
        // so we log a warning and let the user push after logging in.
        if let Some(origin) = origin {
            if lineage.commit.is_some() {
                if let Err(err) = self.recommit_for_remote(lineage, origin, bucket).await {
                    log::warn!("Remote saved but recommit failed (will retry on push): {err}");
                }
            }
        }

        Ok(())
    }

    async fn recommit_for_remote(
        &self,
        lineage: lineage::PackageLineage,
        origin: Host,
        bucket: String,
    ) -> Res {
        let host_config = self.remote.host_config(&Some(origin.clone())).await?;
        let workflows_config_uri = S3Uri {
            key: ".quilt/workflows/config.yml".to_string(),
            bucket,
            ..S3Uri::default()
        };
        let workflow =
            resolve_workflow(&self.remote, &Some(origin), None, &workflows_config_uri).await?;
        let manifest = self.manifest().await?;
        let lineage = flow::recommit(
            lineage,
            &manifest,
            &self.paths,
            &self.storage,
            self.namespace.clone(),
            host_config,
            workflow,
        )
        .await?;
        self.lineage.write(&self.storage, lineage).await?;
        Ok(())
    }

    pub async fn resolve_workflow(&self, workflow_id: Option<String>) -> Res<Option<Workflow>> {
        let (_, lineage) = self.lineage.read(&self.storage).await?;
        let remote_uri = match lineage.remote_uri.as_ref() {
            Some(uri) if !uri.bucket.is_empty() => uri.clone(),
            _ => return Ok(None),
        };
        let workflows_config_uri = S3Uri {
            key: ".quilt/workflows/config.yml".to_string(),
            ..S3Uri::from(remote_uri.clone())
        };
        resolve_workflow(
            &self.remote,
            &remote_uri.origin,
            workflow_id,
            &workflows_config_uri,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
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
                    Some(serde_json::json!({ "count": i })),
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
            .set_remote("my-bucket".to_string(), Some("example.com".parse()?))
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
            .set_remote("".to_string(), Some("example.com".parse()?))
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

        /// Remote that rejects any verify_bucket call — models the case
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
            async fn host_config(
                &self,
                _host: &Option<Host>,
            ) -> Res<crate::io::remote::HostConfig> {
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
            .set_remote("typo-bucket".to_string(), Some("example.com".parse()?))
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
            .set_remote("new-bucket".to_string(), Some("new.host".parse()?))
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
            .set_remote("my-bucket".to_string(), Some("my.host".parse()?))
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
            .set_remote("new-bucket".to_string(), Some("new.host".parse()?))
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
                Some(serde_json::json!({"key": "value"})),
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
            .set_remote("my-bucket".to_string(), Some("example.com".parse()?))
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

    /// A remote that always returns LoginRequired, simulating a logged-out user.
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
}
