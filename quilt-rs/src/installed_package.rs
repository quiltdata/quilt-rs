use std::collections::BTreeMap;
use std::path::PathBuf;

use tracing::log;

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
use crate::uri::Host;
use crate::uri::ManifestUri;
use crate::uri::Namespace;
use crate::uri::S3Uri;
use crate::Error;
use crate::Res;

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
            None => Err(Error::ManifestPath(
                "No installed manifest and no remote to recover from".to_string(),
            )),
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

        let lineage = flow::commit(
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
        let lineage = self.lineage.write(&self.storage, lineage).await?;
        match lineage.commit {
            Some(commit) => Ok(commit),
            None => Err(Error::Commit("Nothing committed".to_string())),
        }
    }

    pub async fn push(&self, host_config_opt: Option<HostConfig>) -> Res<ManifestUri> {
        self.scaffold_paths().await?;

        let (_, lineage) = self.lineage.read(&self.storage).await?;
        let remote_uri = match lineage.remote_uri.as_ref() {
            Some(uri) if !uri.bucket.is_empty() => uri.clone(),
            Some(_) => {
                return Err(Error::Push(
                    "Remote bucket not set. Use set_remote first.".to_string(),
                ))
            }
            None => {
                return Err(Error::Push(
                    "No remote configured. Use set_remote first.".to_string(),
                ))
            }
        };

        if lineage.commit.is_none() {
            return Err(Error::Push("No commits to push".to_string()));
        }

        self.scaffold_paths_for_caching(&remote_uri.bucket).await?;

        let manifest = self.manifest().await?;

        let host_config =
            host_config_opt.unwrap_or(self.remote.host_config(&remote_uri.origin).await?);

        let lineage = flow::push(
            lineage,
            manifest,
            &self.paths,
            &self.storage,
            &self.remote,
            Some(self.namespace.clone()),
            host_config,
        )
        .await?;
        let lineage = self.lineage.write(&self.storage, lineage).await?;
        Ok(lineage.remote()?.clone())
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

    pub async fn set_remote(&self, origin: Host, bucket: String) -> Res {
        if bucket.is_empty() {
            return Err(Error::Push("Bucket cannot be empty".to_string()));
        }
        let (_, mut lineage) = self.lineage.read(&self.storage).await?;
        lineage.remote_uri = Some(ManifestUri {
            origin: Some(origin),
            bucket,
            namespace: self.namespace.clone(),
            hash: String::new(),
        });
        self.lineage.write(&self.storage, lineage).await?;
        Ok(())
    }

    pub async fn set_origin(&self, origin: Host) -> Res {
        let (_, mut lineage) = self.lineage.read(&self.storage).await?;
        match lineage.remote_uri.as_mut() {
            Some(remote_uri) => remote_uri.origin = Some(origin),
            None => {
                lineage.remote_uri = Some(ManifestUri {
                    origin: Some(origin),
                    namespace: self.namespace.clone(),
                    ..ManifestUri::default()
                });
            }
        }
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
            .set_remote("example.com".parse()?, "my-bucket".to_string())
            .await?;

        let lineage = package.lineage().await?;
        let remote_uri = lineage.remote_uri.as_ref().expect("remote_uri should be set");
        assert_eq!(remote_uri.origin.as_ref().unwrap().to_string(), "example.com");
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
            .set_remote("example.com".parse()?, "".to_string())
            .await;

        assert!(result.is_err());
        assert!(
            result.unwrap_err().to_string().contains("Bucket cannot be empty"),
            "Error should mention empty bucket"
        );

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_set_remote_overwrites_existing() -> Res {
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
                        "catalog": "old.host"
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

        package
            .set_remote("new.host".parse()?, "new-bucket".to_string())
            .await?;

        let lineage = package.lineage().await?;
        let remote_uri = lineage.remote_uri.as_ref().expect("remote_uri should be set");
        assert_eq!(remote_uri.origin.as_ref().unwrap().to_string(), "new.host");
        assert_eq!(remote_uri.bucket, "new-bucket");
        assert_eq!(remote_uri.hash, "", "hash should be reset to empty");

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
        let cached_manifest = paths.cached_manifest("test-bucket", &test_hash);
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
}
