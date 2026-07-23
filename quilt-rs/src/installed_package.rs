use std::collections::BTreeMap;
use std::path::PathBuf;

use tracing::log;

use crate::Error;
use crate::Res;
use crate::error::LoginError;
use crate::error::PackageOpError;
use crate::flow;
use crate::flow::PullOutcome;
use crate::flow::UserMeta;
use crate::flow::cache_remote_manifest;
use crate::io::remote::HostConfig;
use crate::io::remote::Remote;
use crate::io::remote::RemoteS3;
use crate::io::remote::WORKFLOWS_CONFIG_KEY;
use crate::io::remote::WorkflowIntent;
use crate::io::remote::WorkflowsConfig;
use crate::io::remote::fetch_workflow_rules;
use crate::io::remote::fetch_workflows_config;
use crate::io::remote::resolve_workflow;
use crate::io::remote::resolve_workflow_from_config;
use crate::io::storage::LocalStorage;
use crate::io::storage::Storage;
use crate::lineage;
use crate::lineage::CommitState;
use crate::lineage::InstalledPackageStatus;
use crate::lineage::LineagePaths;
use crate::lineage::UpstreamState;
use crate::manifest::Manifest;
use crate::manifest::Workflow;
use crate::paths;
use crate::paths::copy_cached_to_installed;
use crate::workflow::WorkflowRules;
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

/// Result of [`InstalledPackage::set_remote`].
///
/// The remote was set (and, on the first-push recommit path, a workflow may
/// have been stamped). `resolution_warning` is `Some(reason)` only on the
/// best-effort `BucketDefault` path where the remote was persisted but the
/// bucket's default workflow could **not** be resolved — the operation still
/// succeeds and no workflow is stamped, but the caller should surface the
/// reason so the user is not silently left ungoverned until push time. Every
/// other success path leaves it `None`.
#[derive(Debug, Default)]
pub struct SetRemoteOutcome {
    pub resolution_warning: Option<String>,
}

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
                    self.remote.host_config(remote_uri.origin.as_ref()).await?
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
                    self.remote.host_config(remote_uri.origin.as_ref()).await?
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
                    self.remote.host_config(remote_uri.origin.as_ref()).await?
                }
                _ => HostConfig::default(),
            },
        };

        // Captured before `host_config` moves into `flow::status`: the commit
        // gate fetches the workflow's config + schemas from the same origin the
        // workflow was resolved against.
        let host = host_config.host.clone();

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
            &self.remote,
            host.as_ref(),
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
            host_config_opt.unwrap_or(self.remote.host_config(remote_uri.origin.as_ref()).await?);

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
            host_config_opt.unwrap_or(self.remote.host_config(remote_uri.origin.as_ref()).await?);

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
            host_config_opt.unwrap_or(self.remote.host_config(remote_uri.origin.as_ref()).await?);

        // All network (tag resolve + manifest fetch) happens here, before the
        // status walk — the snapshot is the freshest classification input.
        let (lineage, snapshot) = flow::snapshot_for_pull(
            lineage,
            &manifest,
            &self.paths,
            &self.storage,
            &self.remote,
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
            snapshot,
            self.namespace.clone(),
        )
        .await?;
        let lineage = self.lineage.write(&self.storage, lineage).await?;
        Ok(lineage.remote()?.clone())
    }

    /// Dry-run: what would `pull` do right now, without mutating anything?
    ///
    /// Sequence:
    /// - A **Local** package (no usable remote, per [`UpstreamState::Local`]:
    ///   `remote_uri` is `None`, a bucket-less remote, or a bucket that has
    ///   never been pushed) → [`PullOutcome::UpToDate`] with no network. There
    ///   is no `latest` tag to resolve for these shapes, so the tag read is
    ///   skipped rather than failing on the missing remote / absent tag.
    /// - Otherwise the `latest` tag is resolved once; if the resolved tip
    ///   already equals `base_hash` → `UpToDate` (that single tag read is the
    ///   only network paid for).
    /// - Otherwise the `latest` manifest is fetched + cached, the working tree
    ///   is walked, and the outcome is classified. Non-`Behind` upstream states
    ///   (`Ahead`/`Diverged`) report `UpToDate` — there is nothing to pull.
    ///
    /// [`PullOutcome::UpToDate`] here means "nothing for pull to do" and is
    /// returned for ALL non-`Behind` states (`Ahead`/`Local`/`Diverged`), not
    /// only when the package is genuinely current.
    ///
    /// Network-light — the caller (watcher / UI) uses it for two-phase render
    /// and routing.
    ///
    /// # Errors
    /// For a package with a real remote, propagates tag-resolution, manifest
    /// read, and remote fetch errors. The Local early return never touches the
    /// network, so those shapes cannot produce those errors.
    pub async fn pull_outcome(&self, host_config_opt: Option<HostConfig>) -> Res<PullOutcome> {
        let (package_home, lineage) = self.lineage.read(&self.storage).await?;

        // A local-only package has no `latest` tag to resolve: `snapshot_for_pull`
        // would either error at `remote()?` (no `remote_uri`) or 404 on the
        // never-created `latest` tag. Mirror `UpstreamState::from`'s Local shape
        // and report `UpToDate` without any network, restoring the pre-reorder
        // contract. A remote whose local hash is empty but whose `latest` tag has
        // moved classifies as `Diverged` (not `Local`), so it still fetches below.
        if UpstreamState::from(lineage.clone()) == UpstreamState::Local {
            return Ok(PullOutcome::UpToDate);
        }

        // Divergence-by-hash is a purely lineage-local fact: `UpstreamState::from`
        // reports `Diverged` from on-disk state when the local side is BOTH ahead
        // (`base != current_hash`) and behind (`base != latest_hash`). The
        // "ahead" component involves only the local commit/remote hash — a moved
        // `latest` tag can neither cause nor cure it — so no network is needed to
        // decide it, and this can never mask a genuine `Behind` (which is
        // ahead-free). Short-circuit before the snapshot constructor, symmetric
        // with the `Local` return above.
        //
        // The OTHER `Diverged` shape — a pending local commit atop a base whose
        // `latest_hash` is still stale (equal to `base`) on disk — reads as
        // `Ahead` here, not `Diverged`; it only becomes `Diverged` once the tag
        // read refreshes `latest_hash`, so it correctly falls through to the
        // post-walk `!= Behind` check below rather than being caught here.
        if UpstreamState::from(lineage.clone()) == UpstreamState::Diverged {
            return Ok(PullOutcome::UpToDate);
        }

        let remote_uri = lineage.remote()?.clone();
        let base = self.manifest().await?;
        let host_config =
            host_config_opt.unwrap_or(self.remote.host_config(remote_uri.origin.as_ref()).await?);

        // Build the classification snapshot with the same ctor `pull` uses: one
        // tag resolution, then the manifest fetch, then the walk. The ctor
        // short-circuits `base == latest` before any fetch, so `Ahead` (where
        // `latest == base`) costs no network here.
        let snapshot = match flow::snapshot_for_pull(
            lineage,
            &base,
            &self.paths,
            &self.storage,
            &self.remote,
            &package_home,
            host_config,
        )
        .await
        {
            Ok((_, snapshot)) => snapshot,
            Err(Error::PackageOp(PackageOpError::AlreadyUpToDate)) => {
                return Ok(PullOutcome::UpToDate);
            }
            Err(err) => return Err(err),
        };

        // `upstream_state` is computed from the ctor-refreshed lineage, so
        // `Diverged`/`Ahead` still report `UpToDate` (nothing to pull), matching
        // the previous contract.
        if snapshot.status.upstream_state != UpstreamState::Behind {
            return Ok(PullOutcome::UpToDate);
        }
        Ok(flow::classify_pull(
            &snapshot.status,
            &base,
            &snapshot.latest_manifest,
        ))
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

    pub async fn set_remote(
        &self,
        bucket: String,
        origin: Option<Host>,
        workflow: WorkflowIntent,
    ) -> Res<SetRemoteOutcome> {
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
                return Ok(SetRemoteOutcome::default());
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

        // An explicit workflow gesture (`Named`/`NoWorkflow`) must not be
        // silently dropped: if the recommit that stamps it fails, surface the
        // error. The no-gesture `BucketDefault` path stays best-effort for
        // *resolution* failures only — validity is never best-effort.
        let explicit_workflow = !matches!(&workflow, WorkflowIntent::BucketDefault);

        // Re-commit with the remote's host_config and workflow so push works
        // immediately without a manual re-commit. Nothing has been persisted
        // yet: on success the recommit itself writes the lineage (carrying the
        // remote set above) and the new manifest; on failure the kind of error
        // decides what, if anything, is saved.
        if let Some(origin) = origin
            && lineage.commit.is_some()
        {
            return match self
                .recommit_for_remote(lineage.clone(), origin, bucket, workflow)
                .await
            {
                Ok(()) => Ok(SetRemoteOutcome::default()),
                // The workflow gate rejected the committed revision. The
                // package's previous state must stay fully intact, so the
                // remote is NOT saved either — set_remote fails as a whole
                // and nothing is persisted.
                Err(err @ Error::WorkflowValidation(_)) => Err(err),
                Err(err) => {
                    // A resolution or transient failure (e.g. not logged in
                    // yet, or an unknown workflow id): persist the remote so
                    // the user can fix the problem and retry.
                    self.lineage.write(&self.storage, lineage).await?;
                    if explicit_workflow {
                        // The remote is persisted, but the chosen workflow
                        // could not be applied. Fail loudly so the user can
                        // fix the workflow id or log in and re-run Set Remote
                        // instead of pushing with the wrong workflow.
                        return Err(err);
                    }
                    // Best-effort BucketDefault path: the remote is saved but
                    // the bucket's default workflow could not be resolved. The
                    // operation succeeds without a workflow stamp; carry the
                    // reason back so the caller can surface it rather than
                    // leaving the user silently ungoverned until push time.
                    // Unwrap to the inner message so user-facing callers (the
                    // CLI's stderr warning, the Set-remote popup) don't show
                    // the "Remote catalog error: …" wrapper chain, matching
                    // how the selector's Invalid notice surfaces these.
                    let reason = match &err {
                        Error::RemoteCatalog(inner) => inner.to_string(),
                        _ => err.to_string(),
                    };
                    log::warn!(
                        "Remote saved but recommit failed ({reason}); re-run Set Remote (e.g. after logging in) to complete it before pushing."
                    );
                    Ok(SetRemoteOutcome {
                        resolution_warning: Some(reason),
                    })
                }
            };
        }

        // No origin or no local commit — nothing to recommit or validate.
        self.lineage.write(&self.storage, lineage).await?;

        Ok(SetRemoteOutcome::default())
    }

    async fn recommit_for_remote(
        &self,
        lineage: lineage::PackageLineage,
        origin: Host,
        bucket: String,
        workflow: WorkflowIntent,
    ) -> Res {
        let host = Some(origin);
        let host_config = self.remote.host_config(host.as_ref()).await?;
        let workflows_config_uri = S3Uri {
            key: WORKFLOWS_CONFIG_KEY.to_string(),
            bucket,
            version: None,
        };
        // Fetch the bucket's workflows config exactly once, then reuse the
        // parsed value for both resolution and the recommit gate below — the
        // gate would otherwise re-download the same config via the header's
        // pinned URI.
        let (config_uri, workflows_config) =
            fetch_workflows_config(&self.remote, host.as_ref(), &workflows_config_uri).await?;
        // Publish later pushes this pending recommit *without* re-resolving the
        // workflow, so recommit must stamp the caller's chosen workflow now.
        // With `WorkflowIntent::BucketDefault` (the no-gesture path) this picks
        // up the bucket's `default_workflow`, so a locally-created package's
        // first publish is governed even when the user expresses no choice.
        let workflow = resolve_workflow_from_config(
            &self.remote,
            host.as_ref(),
            workflow,
            config_uri,
            workflows_config.as_ref(),
        )
        .await?;
        let manifest = self.manifest().await?;
        let lineage = flow::recommit(
            lineage,
            &manifest,
            &self.paths,
            &self.storage,
            &self.remote,
            host.as_ref(),
            self.namespace.clone(),
            host_config,
            workflow,
            workflows_config.as_ref(),
        )
        .await?;
        self.lineage.write(&self.storage, lineage).await?;
        Ok(())
    }

    /// The remote host and the `.quilt/workflows/config.yml` address for this
    /// package's bucket, or `None` when there is no usable remote. Builds the
    /// address from the package's own remote for the two read paths that need
    /// it ([`Self::resolve_workflow`] and [`Self::workflows_config`]); the key
    /// itself is the shared [`WORKFLOWS_CONFIG_KEY`].
    async fn workflows_config_location(&self) -> Res<Option<(Option<Host>, S3Uri)>> {
        let (_, lineage) = self.lineage.read(&self.storage).await?;
        let remote_uri = match lineage.remote_uri.as_ref() {
            Some(uri) if !uri.bucket.is_empty() => uri.clone(),
            _ => return Ok(None),
        };
        let config_uri = S3Uri {
            key: WORKFLOWS_CONFIG_KEY.to_string(),
            ..S3Uri::from(remote_uri.clone())
        };
        Ok(Some((remote_uri.origin, config_uri)))
    }

    pub async fn resolve_workflow(&self, intent: WorkflowIntent) -> Res<Option<Workflow>> {
        let Some((origin, config_uri)) = self.workflows_config_location().await? else {
            return Ok(None);
        };
        resolve_workflow(&self.remote, origin.as_ref(), intent, &config_uri).await
    }

    /// Fetch and parse the bucket's `.quilt/workflows/config.yml` for this
    /// package's remote, returning the typed [`WorkflowsConfig`].
    ///
    /// Returns `Ok(None)` when the package has no remote or the bucket has no
    /// config, so callers building UI can degrade gracefully. This is the same
    /// fetch [`Self::resolve_workflow`] performs, exposed as a read-only view of
    /// the declared workflows rather than a resolution outcome.
    pub async fn workflows_config(&self) -> Res<Option<WorkflowsConfig>> {
        let Some((origin, config_uri)) = self.workflows_config_location().await? else {
            return Ok(None);
        };
        let (_, config) =
            fetch_workflows_config(&self.remote, origin.as_ref(), &config_uri).await?;
        Ok(config)
    }

    /// Fetch and compile the pure-validator [`WorkflowRules`] for a named
    /// workflow declared in this package's bucket config, for live commit-dialog
    /// validation.
    ///
    /// Returns `Ok(None)` when the package has no remote or the bucket has no
    /// config — an ungoverned package has no rules to validate against. This is
    /// the same config fetch [`Self::resolve_workflow`] and
    /// [`Self::workflows_config`] perform, followed by [`fetch_workflow_rules`]
    /// to load the workflow's schema documents; the resulting rules feed
    /// [`crate::workflow::validate_candidate_fields`], mirroring how the commit
    /// gate calls [`fetch_workflow_rules`] + `validate_package`. A schema fetch
    /// failure or an unknown `workflow_id` surfaces as the underlying error, so
    /// the advisory caller can decide to skip validation rather than block.
    pub async fn workflow_rules(&self, workflow_id: &str) -> Res<Option<WorkflowRules>> {
        let Some((origin, config_uri)) = self.workflows_config_location().await? else {
            return Ok(None);
        };
        let (_, config) =
            fetch_workflows_config(&self.remote, origin.as_ref(), &config_uri).await?;
        let Some(config) = config else {
            return Ok(None);
        };
        Ok(Some(
            fetch_workflow_rules(&self.remote, origin.as_ref(), &config, workflow_id).await?,
        ))
    }
}

#[cfg(test)]
mod set_remote_tests;
#[cfg(test)]
mod sync_flow_tests;
