use std::collections::BTreeMap;
use std::path::PathBuf;

use tracing::log;

use crate::Error;
use crate::Res;
use crate::error::LoginError;
use crate::error::PackageOpError;
use crate::flow;
use crate::flow::UserMeta;
use crate::flow::cache_remote_manifest;
use crate::io::remote::HostConfig;
use crate::io::remote::Remote;
use crate::io::remote::RemoteS3;
use crate::io::remote::WorkflowIntent;
use crate::io::remote::resolve_workflow;
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
        write!(f, r#"Installed package "{}""#, self.namespace)
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
        let Some(hash) = lineage.current_hash() else {
            return Ok(Manifest::default());
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
                log::info!("Attempting to recover from cache at {remote_uri}");
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

    /// Recompute working-tree status against the cached manifest without
    /// contacting the remote. Caller accepts that `upstream_state` reflects
    /// the last-known `latest_hash` rather than a freshly-resolved one;
    /// pair with `status` (which calls `refresh_latest_hash`) when remote
    /// freshness matters.
    pub async fn recompute_local_status(
        &self,
        host_config_opt: Option<HostConfig>,
    ) -> Res<InstalledPackageStatus> {
        let (package_home, lineage) = self.lineage.read(&self.storage).await?;
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

        let (_, status) = flow::status(
            lineage,
            &self.storage,
            &manifest,
            &package_home,
            host_config,
        )
        .await?;
        Ok(status)
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
                    )));
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

        let (_, status) = flow::status(
            lineage,
            &self.storage,
            &manifest,
            &package_home,
            host_config,
        )
        .await?;
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

    /// Commit the package's pending changes as a new revision.
    ///
    /// See [`UserMeta`] for the metadata contract: `Keep` inherits the
    /// previous revision's package-level metadata, `Clear` removes it,
    /// `Set` replaces it.
    pub async fn commit(
        &self,
        message: String,
        user_meta: UserMeta,
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
        user_meta: UserMeta,
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
                )));
            }
            None => {
                return Err(Error::PackageOp(PackageOpError::Publish(
                    "No remote configured. Use set_remote first.".to_string(),
                )));
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
                )));
            }
            None => {
                return Err(Error::PackageOp(PackageOpError::Push(
                    "No remote configured. Use set_remote first.".to_string(),
                )));
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
        // `flow::pull`'s `base_hash == latest_hash` guard reads
        // `latest_hash` from the lineage we pass in. Before the
        // `Stop writing lineage from InstalledPackage::status` refactor,
        // a prior `status` call would have refreshed-and-persisted
        // `latest_hash`, so disk was reliably fresh when `pull` ran.
        // That implicit persist is gone now, so `pull` must refresh
        // on its own — otherwise a moved remote always trips the
        // "already up-to-date" branch and the user sees no pull.
        let lineage = flow::refresh_latest_hash(lineage, &self.remote).await?;
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

    /// Pushes any pending local commit, then promotes the resulting remote
    /// hash to `latest`. Last-writer-wins: any concurrent move of the
    /// `latest` tag between push and tag is overwritten. Invoked from the
    /// merge page when the user resolves a `Diverged` state in favor of
    /// their own revision.
    pub async fn certify_latest(&self) -> Res<ManifestUri> {
        let (_, lineage) = self.lineage.read(&self.storage).await?;

        // Push first so the hash we tag exists on remote. Push mutates
        // lineage on disk, so re-read to pick up the new remote hash.
        let lineage = if lineage.commit.is_some() {
            self.push(None).await?;
            self.lineage.read(&self.storage).await?.1
        } else {
            lineage
        };

        let pushed_manifest_uri = lineage.remote()?.clone();
        let lineage = flow::certify_latest(lineage, &self.remote, pushed_manifest_uri).await?;
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
        if let Some(existing) = &lineage.remote_uri
            && !existing.hash.is_empty()
        {
            let same_remote = existing.bucket == bucket && existing.origin == origin;
            if same_remote {
                return Ok(());
            }
            return Err(Error::PackageOp(PackageOpError::Push(
                "Cannot change remote on a package that has already been pushed".to_string(),
            )));
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
        if let Some(origin) = origin
            && lineage.commit.is_some()
            && let Err(err) = self.recommit_for_remote(lineage, origin, bucket).await
        {
            log::warn!("Remote saved but recommit failed (will retry on push): {err}");
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
            version: None,
        };
        // Set Remote is a no-gesture path: the user expresses no workflow
        // choice here, and publish later pushes this pending recommit
        // *without* re-resolving the workflow. So recommit must stamp the
        // bucket default now — otherwise a locally-created package's first
        // publish would silently miss the bucket's `default_workflow`.
        let workflow = resolve_workflow(
            &self.remote,
            &Some(origin),
            WorkflowIntent::BucketDefault,
            &workflows_config_uri,
        )
        .await?;
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

    pub async fn resolve_workflow(&self, intent: WorkflowIntent) -> Res<Option<Workflow>> {
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
            intent,
            &workflows_config_uri,
        )
        .await
    }
}

#[cfg(test)]
mod set_remote_tests;
#[cfg(test)]
mod sync_flow_tests;
