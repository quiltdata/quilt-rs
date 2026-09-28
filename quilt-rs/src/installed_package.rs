use std::collections::BTreeMap;
use std::collections::HashSet;
use std::path::Path;
use std::path::PathBuf;

use multihash::Multihash;

use tracing::log;

use crate::Error;
use crate::Res;
use crate::error::LoginError;
use crate::error::PackageOpError;
use crate::flow;
use crate::flow::UserMeta;
use std::sync::Arc;

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
use crate::lineage::SyncScope;
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

mod write_step;
use write_step::Verdict;

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

/// Every early return from the dry run means the same thing — nothing for a
/// pull to do, and so no incoming paths to name.
fn nothing_to_pull() -> flow::PullPreview {
    flow::PullPreview {
        outcome: flow::PullOutcome::UpToDate,
        added: Vec::new(),
    }
}

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

/// What a pull's redo did to the working tree, kept across its rounds.
#[derive(Default)]
struct Redone {
    /// Crossed paths latest lacks: their files go once the write lands.
    retired: BTreeMap<PathBuf, Multihash<256>>,
    /// Crossed files the redo replaced: what it put, and what was there. Put
    /// back if the pull refuses.
    overwritten: BTreeMap<PathBuf, (Multihash<256>, Multihash<256>)>,
}

/// Similar to `LocalDomain` because it has access to the same lineage file and remote/storage
/// traits.
/// But it only manages one particular installed package.
/// It can be instantiated from `LocalDomain` by installing new or listing existing packages.
#[derive(Debug)]
pub struct InstalledPackage<S: Storage = LocalStorage, R: Remote = RemoteS3> {
    pub lineage: lineage::PackageLineageIo,
    pub paths: paths::DomainPaths,
    pub remote: Arc<R>,
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

    /// The revisions this copy has, newest first.
    ///
    /// See [`flow::list_revisions`] for what "newest" means — it is acquisition
    /// order, not the order the revisions were made in.
    pub async fn revisions(&self) -> Res<Vec<flow::Revision>> {
        flow::list_revisions(&self.paths, &self.storage, &self.namespace).await
    }

    /// How many revisions this copy has, without parsing any of them.
    ///
    /// Equal to `self.revisions().await?.len()` when every manifest parses; a
    /// damaged one is still counted, where [`Self::revisions`] fails on it.
    pub async fn revision_count(&self) -> Res<usize> {
        flow::count_revisions(&self.paths, &self.storage, &self.namespace).await
    }

    /// [`Self::revisions`], each marked by whether the registry of
    /// `lineage`'s remote lists it — a timestamped pointer to it exists.
    /// One listing for the whole package, never inferred from `CommitState`.
    /// No remote, or a remote with no catalog host (so no registry to ask):
    /// no remote call, and every entry is unpublished.
    pub async fn revision_history(
        &self,
        lineage: &lineage::PackageLineage,
    ) -> Res<Vec<flow::HistoryEntry>> {
        let revisions = self.revisions().await?;
        let published = self.registry_listing(lineage).await?;
        Ok(revisions
            .into_iter()
            .map(|revision| flow::HistoryEntry {
                published: published.contains(&revision.hash),
                revision,
            })
            .collect())
    }

    /// The revisions the registry of `lineage`'s remote lists. No remote,
    /// or no catalog host (so no registry to ask): no request, and none.
    async fn registry_listing(&self, lineage: &lineage::PackageLineage) -> Res<HashSet<String>> {
        Ok(match lineage.remote_uri.as_ref() {
            Some(ManifestUri {
                origin: Some(host),
                bucket,
                namespace,
                ..
            }) => self
                .remote
                .published_revisions(host, bucket, namespace)
                .await?
                .into_iter()
                .collect(),
            _ => HashSet::new(),
        })
    }

    /// The revision selected by one lineage snapshot.
    ///
    /// Unlike [`Self::revisions`], this reads only the manifest selected by
    /// [`lineage::PackageLineage::current_hash`]. That distinction keeps an
    /// unrelated damaged historical manifest from breaking a read of the
    /// current package state. The caller supplies the snapshot so adjacent
    /// facts such as the bucket cannot come from a different revision if the
    /// lineage changes while a page read is in flight.
    pub async fn current_revision(
        &self,
        lineage: &lineage::PackageLineage,
    ) -> Res<Option<flow::Revision>> {
        let Some(hash) = lineage.current_hash().map(str::to_owned) else {
            return Ok(None);
        };

        let manifest = self.manifest_from_lineage(lineage).await?;
        let installed_path = self.paths.installed_manifest(&self.namespace, &hash);
        let obtained = self.storage.modified_timestamp(&installed_path).await?;

        Ok(Some(flow::Revision {
            hash,
            obtained,
            message: manifest.header.message,
        }))
    }

    /// [`flow::compare_for_resolve`] over the manifest `lineage` selects and the
    /// published `latest`, resolved now rather than read from `lineage`, whose
    /// `latest_hash` is only as fresh as the last write. The published manifest
    /// is cached by hash. The listing follows [`Self::revision_history`]'s rule:
    /// no catalog host, no request, and the whole chain is unpublished.
    ///
    /// # Errors
    /// No remote; the tag read, the manifest fetch or the listing failing.
    pub async fn resolve_comparison(
        &self,
        lineage: &lineage::PackageLineage,
    ) -> Res<flow::ResolveComparison> {
        let remote_uri = lineage.remote()?.clone();
        let fresh = flow::refresh_latest_hash(lineage.clone(), &*self.remote).await?;
        let published_uri = ManifestUri {
            hash: fresh.latest_hash,
            ..remote_uri.clone()
        };
        self.scaffold_paths_for_caching(&remote_uri.bucket).await?;
        let published =
            cache_remote_manifest(&self.paths, &self.storage, &*self.remote, &published_uri)
                .await?;
        let current = self.manifest_from_lineage(lineage).await?;
        let listed = self.registry_listing(lineage).await?;
        Ok(flow::compare_for_resolve(
            &current,
            &published,
            lineage.commit.as_ref(),
            &listed,
        ))
    }

    pub async fn manifest(&self) -> Res<Manifest> {
        let (_, lineage) = self.lineage.read(&self.storage).await?;
        self.manifest_from_lineage(&lineage).await
    }

    /// Read the manifest selected by an already-read lineage snapshot.
    ///
    /// Keeping selection and recovery on that same value prevents a concurrent
    /// lineage edit from changing which manifest supplies the message halfway
    /// through a larger read.
    pub async fn manifest_from_lineage(&self, lineage: &lineage::PackageLineage) -> Res<Manifest> {
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
                    cache_remote_manifest(&self.paths, &self.storage, &*self.remote, remote_uri)
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
        let manifest = self.manifest_from_lineage(&lineage).await?;

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
            Some(_) => match flow::refresh_latest_hash(lineage.clone(), &*self.remote).await {
                Ok(lineage) => lineage,
                Err(Error::Login(LoginError::NoSession(_))) => {
                    return Err(Error::Login(LoginError::NoSession(
                        lineage.remote_uri.as_ref().and_then(|r| r.origin.clone()),
                    )));
                }
                // A denial is not a degradable failure. Every other error
                // here means "the remote is unreachable right now", and
                // continuing on stale lineage is exactly right — it is what
                // lets the app work offline. A denial means the request
                // arrived and was refused: the answer will not change until
                // the role does, and swallowing it leaves the caller unable
                // to tell the user why. Callers distinguish it with
                // [`Error::is_access_denied`].
                Err(err) if err.is_access_denied() => return Err(err),
                // Nor is a rejected credential: the session is dead, and
                // stale lineage would report the package as fine.
                Err(err) if err.is_session_absent() => return Err(err),
                Err(err) => {
                    log::warn!("Failed to refresh latest hash: {err}");
                    lineage
                }
            },
            None => lineage,
        };
        let manifest = self.manifest_from_lineage(&lineage).await?;

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

    /// Downloads `paths` and starts tracking them. A path whose bytes the
    /// remote no longer holds is skipped, not an error: see
    /// [`InstallPathsReport::skipped`](flow::InstallPathsReport::skipped).
    ///
    /// Installs from the revision the entry named when it started. If a pull
    /// or a reset moved the revision meanwhile, it installs the same paths
    /// again from the new one before it writes.
    pub async fn install_paths(&self, paths: &[PathBuf]) -> Res<flow::InstallPathsReport> {
        if paths.is_empty() {
            return Ok(flow::InstallPathsReport::default());
        }

        self.scaffold_paths().await?;

        let (package_home, mut started) = self.lineage.read(&self.storage).await?;
        let remote_uri = started.remote()?;

        self.scaffold_paths_for_caching(&remote_uri.bucket).await?;

        let mut manifest = self.manifest_from_lineage(&started).await?;
        let (mut next, mut skipped) = flow::install_paths(
            started.clone(),
            &mut manifest,
            &self.paths,
            package_home.clone(),
            self.namespace.clone(),
            &self.storage,
            &*self.remote,
            &paths.iter().collect::<Vec<&PathBuf>>(),
        )
        .await?;

        for round in 0..=write_step::MAX_REDO {
            let verdict = self
                .lineage
                .update(&self.storage, |entry| {
                    let Some(current) = entry.as_mut() else {
                        return Ok(Verdict::Gone);
                    };
                    if !write_step::same_revision(current, &started) {
                        return Ok(Verdict::Redo(Box::new(current.clone())));
                    }
                    *current = write_step::own_change(&started, &next, current);
                    Ok(Verdict::Written(current.paths.clone()))
                })
                .await?;
            match verdict {
                Verdict::Written(paths) => {
                    return Ok(flow::InstallPathsReport { paths, skipped });
                }
                Verdict::Gone => {
                    let placed = write_step::placed(&started, &next);
                    write_step::remove_placed(&self.storage, &package_home, &placed).await;
                    return Err(write_step::not_installed(&self.namespace));
                }
                Verdict::Redo(current) if round < write_step::MAX_REDO => {
                    let placed = write_step::placed(&started, &next);
                    let (redone, also_skipped) =
                        match self.install_again(&current, &placed, &package_home).await {
                            Ok(redone) => redone,
                            // The redo's check before each rename found a
                            // file another writer replaced meanwhile. Theirs
                            // stands; what this one placed goes.
                            Err(Error::PackageOp(PackageOpError::PullConflict(_))) => {
                                self.remove_placed_untracked(&placed, &package_home).await?;
                                return Err(write_step::changed_underneath(
                                    &self.namespace,
                                    "downloading",
                                ));
                            }
                            Err(err) => return Err(err),
                        };
                    next = redone;
                    skipped.extend(also_skipped);
                    started = *current;
                }
                // Refused: what it placed goes, as when the package is gone,
                // so the files are not left untracked in the way of a retry.
                // A path the entry now tracks is someone else's.
                Verdict::Redo(current) => {
                    let placed: LineagePaths = write_step::placed(&started, &next)
                        .into_iter()
                        .filter(|(path, _)| !current.paths.contains_key(path))
                        .collect();
                    write_step::remove_placed(&self.storage, &package_home, &placed).await;
                    break;
                }
                Verdict::Refused => break,
            }
        }
        Err(write_step::changed_underneath(
            &self.namespace,
            "downloading",
        ))
    }

    /// Removes the files in `placed` that still hold what this writer placed
    /// and that the entry, read now, does not track: a path it tracks is
    /// another writer's.
    async fn remove_placed_untracked(&self, placed: &LineagePaths, package_home: &Path) -> Res {
        let (_, now) = self.lineage.read(&self.storage).await?;
        let untracked: LineagePaths = placed
            .iter()
            .filter(|(path, _)| !now.paths.contains_key(*path))
            .map(|(path, state)| (path.clone(), state.clone()))
            .collect();
        write_step::remove_placed(&self.storage, package_home, &untracked).await;
        Ok(())
    }

    /// The download's redo: the revision moved to `current`'s while it placed
    /// `placed`, so it installs the same paths again from there. A file that
    /// no longer holds what it placed is someone's edit, and is left alone; a
    /// path the new revision lacks is removed and reported as skipped; one
    /// someone else now tracks is theirs.
    async fn install_again(
        &self,
        current: &lineage::PackageLineage,
        placed: &LineagePaths,
        package_home: &Path,
    ) -> Res<(lineage::PackageLineage, Vec<PathBuf>)> {
        let mut manifest = self.manifest_from_lineage(current).await?;
        let mut next = current.clone();
        let mut skipped = Vec::new();
        let mut gone = BTreeMap::new();
        let mut install = Vec::new();
        let mut protect = BTreeMap::new();
        for (path, state) in placed {
            if current.paths.contains_key(path) {
                continue;
            }
            let file = package_home.join(path);
            let Some(row) = manifest.get_record(path) else {
                gone.insert(path.clone(), state.hash);
                skipped.push(path.clone());
                continue;
            };
            if Multihash::from(row.hash.clone()) == state.hash {
                next.paths.insert(path.clone(), state.clone());
            } else if write_step::holds(&self.storage, &file, &state.hash).await
                && let Some(placed_row) = write_step::row_of(path, &state.hash)
            {
                install.push(path.clone());
                protect.insert(path.clone(), placed_row);
            }
        }
        let (next, also_skipped) = flow::install_paths_over(
            next,
            &mut manifest,
            &self.paths,
            package_home.to_path_buf(),
            self.namespace.clone(),
            &self.storage,
            &*self.remote,
            &install.iter().collect::<Vec<&PathBuf>>(),
            &flow::Protect::BaseContent(&protect),
            flow::OnMismatch::Skip,
        )
        .await?;
        for path in also_skipped {
            gone.insert(path.clone(), placed[&path].hash);
            skipped.push(path);
        }
        write_step::remove_if_unchanged(&self.storage, package_home, &gone).await;
        Ok((next, skipped))
    }

    /// Stops tracking `paths` and removes their files. The uninstall wins
    /// over a pull that crossed it: a path the pull re-placed meanwhile is
    /// removed too, and its file deleted if it still holds what the pull put
    /// there.
    pub async fn uninstall_paths(&self, paths: &Vec<PathBuf>) -> Res<LineagePaths> {
        let (package_home, started) = self.lineage.read(&self.storage).await?;
        let next =
            flow::uninstall_paths(started.clone(), package_home.clone(), &self.storage, paths)
                .await?;
        let (tracked, replaced) = self
            .lineage
            .update(&self.storage, |entry| {
                let Some(current) = entry.as_mut() else {
                    return Err(write_step::not_installed(&self.namespace));
                };
                let replaced: BTreeMap<PathBuf, Multihash<256>> = paths
                    .iter()
                    .filter_map(|path| {
                        let now = current.paths.get(path)?;
                        let was = started.paths.get(path);
                        was.is_none_or(|was| was.hash != now.hash)
                            .then(|| (path.clone(), now.hash))
                    })
                    .collect();
                *current = write_step::own_change(&started, &next, current);
                for path in paths {
                    current.paths.remove(path);
                }
                Ok((current.paths.clone(), replaced))
            })
            .await?;
        write_step::remove_if_unchanged(&self.storage, &package_home, &replaced).await;
        Ok(tracked)
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

        // A commit is local work only, so one that finds the entry moved
        // under it does it again from the entry it found.
        for _ in 0..=write_step::MAX_REDO {
            let (package_home, started) = self.lineage.read(&self.storage).await?;
            let mut manifest = self.manifest_from_lineage(&started).await?;

            let host_config = match host_config_opt.clone() {
                Some(hc) => hc,
                None => match started.remote_uri.as_ref() {
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
                started.clone(),
                &self.storage,
                &manifest,
                &package_home,
                host_config,
            )
            .await?;

            let (next, commit) = flow::commit(
                lineage,
                &mut manifest,
                &self.paths,
                &self.storage,
                &*self.remote,
                host.as_ref(),
                package_home,
                status,
                self.namespace.clone(),
                message.clone(),
                user_meta.clone(),
                workflow.clone(),
            )
            .await?;
            let verdict = self
                .lineage
                .update(&self.storage, |entry| {
                    let Some(current) = entry.as_mut() else {
                        return Ok(Verdict::Gone);
                    };
                    if !write_step::same_revision(current, &started)
                        || !write_step::same_rows(&current.paths, &started.paths)
                    {
                        return Ok(Verdict::Redo(Box::new(current.clone())));
                    }
                    *current = write_step::own_change(&started, &next, current);
                    Ok(Verdict::Written(()))
                })
                .await?;
            match verdict {
                Verdict::Written(()) => return Ok(commit),
                Verdict::Gone => return Err(write_step::not_installed(&self.namespace)),
                Verdict::Redo(_) | Verdict::Refused => {}
            }
        }
        Err(write_step::changed_underneath(
            &self.namespace,
            "committing",
        ))
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

        let (package_home, started) = self.lineage.read(&self.storage).await?;
        let lineage = started.clone();
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

        let mut manifest = self.manifest_from_lineage(&started).await?;
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

        // Boxed so the write step after it does not push every caller's
        // future over the workspace's size budget.
        let outcome = Box::pin(flow::publish(
            lineage,
            &mut manifest,
            &self.paths,
            &self.storage,
            &*self.remote,
            package_home,
            status,
            self.namespace.clone(),
            host_config,
            flow::CommitOptions {
                message,
                user_meta,
                workflow,
            },
        ))
        .await?;

        let (committed, push_result) = match outcome {
            flow::PublishOutcome::CommittedAndPushed(p) => (true, p),
            flow::PublishOutcome::PushedOnly(p) => (false, p),
        };
        let certified_latest = push_result.certified_latest;
        let lineage = self.write_pushed(&started, &push_result.lineage).await?;
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
        let started = lineage.clone();
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

        let manifest = self.manifest_from_lineage(&started).await?;

        let host_config =
            host_config_opt.unwrap_or(self.remote.host_config(remote_uri.origin.as_ref()).await?);

        let result = flow::push(
            lineage,
            manifest,
            &self.paths,
            &self.storage,
            &*self.remote,
            Some(self.namespace.clone()),
            host_config,
        )
        .await?;
        let certified_latest = result.certified_latest;
        let lineage = self.write_pushed(&started, &result.lineage).await?;
        Ok(PushOutcome {
            manifest_uri: lineage.remote()?.clone(),
            certified_latest,
        })
    }

    /// The write step of a push, and of publish's push. By now the upload and
    /// the tag have happened, so the remote fields are facts and always land.
    /// A commit made on top of the pushed one while the push ran stays
    /// pending; its chain now starts at the pushed hash.
    async fn write_pushed(
        &self,
        started: &lineage::PackageLineage,
        next: &lineage::PackageLineage,
    ) -> Res<lineage::PackageLineage> {
        self.lineage
            .update(&self.storage, |entry| {
                let current = entry
                    .as_mut()
                    .ok_or_else(|| write_step::not_installed(&self.namespace))?;
                let newer_commit = (current.commit != started.commit)
                    .then(|| current.commit.clone())
                    .flatten();
                *current = write_step::own_change(started, next, current);
                if newer_commit.is_some() {
                    current.commit = newer_commit;
                }
                Ok(current.clone())
            })
            .await
    }

    /// Record this package's standing [`SyncScope`].
    ///
    /// Storage only, and deliberately separate from [`Self::pull`]: writing the
    /// choice and acting on it are different decisions, made by different
    /// callers. Nothing in this crate reads the stored value back.
    ///
    /// Goes through
    /// [`PackageLineageIo::edit`](lineage::PackageLineageIo::edit)
    /// rather than read-then-write, because this writer is user-triggered and
    /// can land at any moment — including mid-pull on the autosync tick, which
    /// is doing its own read-modify-write of the same entry. Reading here and
    /// writing later would clobber whatever that pull had recorded.
    pub async fn set_sync_scope(&self, scope: SyncScope) -> Res<()> {
        self.lineage
            .edit(&self.storage, |l| l.sync_scope = scope)
            .await?;
        Ok(())
    }

    /// `scope` comes from the caller, not from the package's stored
    /// [`SyncScope`]. Both faces of this engine —
    /// the desktop app and the `quilt` CLI — reach pull through here, and only
    /// one of them has a setting; reading the field at this level would let
    /// state the app wrote change what the CLI does.
    pub async fn pull(
        &self,
        host_config_opt: Option<HostConfig>,
        scope: SyncScope,
    ) -> Res<flow::PullReport> {
        self.scaffold_paths().await?;

        let (package_home, started) = self.lineage.read(&self.storage).await?;
        let lineage = started.clone();
        let remote_uri = lineage.remote()?.clone();

        self.scaffold_paths_for_caching(&remote_uri.bucket).await?;

        let mut manifest = self.manifest_from_lineage(&started).await?;

        let host_config =
            host_config_opt.unwrap_or(self.remote.host_config(remote_uri.origin.as_ref()).await?);

        // All network (tag resolve + manifest fetch) happens here, before the
        // status walk — the snapshot is the freshest classification input.
        let (lineage, snapshot) = flow::snapshot_for_pull(
            lineage,
            &manifest,
            &self.paths,
            &self.storage,
            &*self.remote,
            &package_home,
            host_config,
        )
        .await?;
        let (next, report) = flow::pull(
            lineage,
            &mut manifest,
            &self.paths,
            &self.storage,
            &*self.remote,
            package_home.clone(),
            snapshot,
            self.namespace.clone(),
            scope,
        )
        .await?;
        self.write_latest(started, next, &package_home, "pulling")
            .await?;
        Ok(report)
    }

    /// The write step of a pull or a reset, which moved the package from
    /// `started` to latest as `next`.
    ///
    /// - The entry is already where `next` is (another pull got there first):
    ///   nothing to write.
    /// - Its revision moved anywhere else (a commit, push or reset landed):
    ///   refuse and write nothing. The files this one placed stay, as local
    ///   changes against the revision that landed.
    /// - Paths were added (a download landed): reconcile them to latest the
    ///   way a pull treats any tracked path, then try again, once.
    /// - A path was uninstalled: the uninstall wins. The path stays out, and
    ///   its file goes if it still holds what this writer placed.
    ///
    /// A refusal after a redo puts back the download's files the redo
    /// overwrote, so it never leaves another writer's paths changed.
    async fn write_latest(
        &self,
        mut started: lineage::PackageLineage,
        mut next: lineage::PackageLineage,
        package_home: &Path,
        verb: &'static str,
    ) -> Res<lineage::PackageLineage> {
        // Uninstalled while this worked: their files go whether it writes or
        // refuses, if they still hold what it placed.
        let mut uninstalled = BTreeMap::new();
        let mut redo = Redone::default();
        for round in 0..=write_step::MAX_REDO {
            let verdict = self
                .lineage
                .update(&self.storage, |entry| {
                    let Some(current) = entry.as_mut() else {
                        return Ok(Verdict::Gone);
                    };
                    if !write_step::same_revision(current, &started) {
                        if !write_step::same_revision(current, &next) {
                            return Ok(Verdict::Refused);
                        }
                        // Another pull got there first, so there is nothing
                        // to write. A file this one placed that the entry no
                        // longer tracks was uninstalled since: it goes.
                        for (path, state) in write_step::placed(&started, &next) {
                            if !current.paths.contains_key(&path) {
                                uninstalled.insert(path, state.hash);
                            }
                        }
                        return Ok(Verdict::Written(current.clone()));
                    }
                    // Before the redo check, so a redo carries it: `next`
                    // no longer tracks it, whatever `started` becomes.
                    for path in write_step::paths_removed(&started, current) {
                        if let Some(state) = next.paths.remove(&path) {
                            uninstalled.insert(path, state.hash);
                        }
                    }
                    if !write_step::paths_added(&started, current).is_empty() {
                        return Ok(Verdict::Redo(Box::new(current.clone())));
                    }
                    *current = write_step::own_change(&started, &next, current);
                    Ok(Verdict::Written(current.clone()))
                })
                .await?;
            match verdict {
                Verdict::Written(lineage) => {
                    write_step::remove_if_unchanged(&self.storage, package_home, &uninstalled)
                        .await;
                    write_step::remove_if_unchanged(&self.storage, package_home, &redo.retired)
                        .await;
                    return Ok(lineage);
                }
                Verdict::Gone => {
                    let placed = write_step::placed(&started, &next);
                    write_step::remove_placed(&self.storage, package_home, &placed).await;
                    return Err(write_step::not_installed(&self.namespace));
                }
                Verdict::Redo(current) if round < write_step::MAX_REDO => {
                    match self
                        .reconcile_to_latest(
                            &started,
                            &current,
                            next,
                            package_home,
                            verb,
                            &mut redo,
                        )
                        .await
                    {
                        Ok(reconciled) => next = reconciled,
                        Err(err) => {
                            self.put_back(package_home, &redo.overwritten).await;
                            return Err(err);
                        }
                    }
                    started = *current;
                }
                Verdict::Redo(_) | Verdict::Refused => break,
            }
        }
        write_step::remove_if_unchanged(&self.storage, package_home, &uninstalled).await;
        self.put_back(package_home, &redo.overwritten).await;
        Err(write_step::changed_underneath(&self.namespace, verb))
    }

    /// The pull's redo: `current` tracks paths `started` did not, placed by a
    /// download at the revision the pull started from. Each goes to latest's
    /// row in `next`: kept where latest has the same bytes, replaced where it
    /// has others, and dropped where latest lacks it (its file goes once the
    /// write lands). A file edited since the download, where latest changes
    /// it too, is a conflict: the redo refuses, as a pull would, before it
    /// overwrites anything.
    async fn reconcile_to_latest(
        &self,
        started: &lineage::PackageLineage,
        current: &lineage::PackageLineage,
        mut next: lineage::PackageLineage,
        package_home: &Path,
        verb: &'static str,
        redo: &mut Redone,
    ) -> Res<lineage::PackageLineage> {
        let mut latest = self.manifest_from_lineage(&next).await?;
        let mut install = Vec::new();
        let mut protect = BTreeMap::new();
        for path in write_step::paths_added(started, current) {
            let state = &current.paths[&path];
            let Some(row) = latest.get_record(&path) else {
                next.paths.remove(&path);
                redo.retired.insert(path, state.hash);
                continue;
            };
            let latest_hash = Multihash::from(row.hash.clone());
            let file = package_home.join(&path);
            if latest_hash != state.hash {
                // This pull may have placed latest's bytes there itself,
                // over the download's: then the path is already at latest.
                if write_step::holds(&self.storage, &file, &latest_hash).await {
                    let at_latest = match next.paths.get(&path) {
                        Some(placed) if placed.hash == latest_hash => placed.clone(),
                        _ => lineage::PathState {
                            timestamp: self.storage.modified_timestamp(&file).await?,
                            hash: latest_hash,
                        },
                    };
                    next.paths.insert(path, at_latest);
                    continue;
                }
                if !write_step::holds(&self.storage, &file, &state.hash).await {
                    return Err(write_step::changed_underneath(&self.namespace, verb));
                }
                if let Some(placed_row) = write_step::row_of(&path, &state.hash) {
                    install.push(path.clone());
                    protect.insert(path.clone(), placed_row);
                    redo.overwritten
                        .insert(path.clone(), (latest_hash, state.hash));
                }
            }
            next.paths.insert(path, state.clone());
        }
        let (next, _) = flow::install_paths_over(
            next,
            &mut latest,
            &self.paths,
            package_home.to_path_buf(),
            self.namespace.clone(),
            &self.storage,
            &*self.remote,
            &install.iter().collect::<Vec<&PathBuf>>(),
            &flow::Protect::BaseContent(&protect),
            flow::OnMismatch::Skip,
        )
        .await?;
        Ok(next)
    }

    /// Puts back the bytes another writer placed, over each file a refused
    /// redo overwrote: from the object store, where they are by hash, and by
    /// rename, so the file holds one or the other. A file that no longer holds
    /// what the redo put there is someone's edit since, and stays.
    async fn put_back(
        &self,
        package_home: &Path,
        overwritten: &BTreeMap<PathBuf, (Multihash<256>, Multihash<256>)>,
    ) {
        for (path, (put, original)) in overwritten {
            let file = package_home.join(path);
            if !write_step::holds(&self.storage, &file, put).await {
                continue;
            }
            let object = self.paths.object(original.digest());
            let staged = self
                .paths
                .staging_dir()
                .join(uuid::Uuid::new_v4().to_string());
            let restored = async {
                self.storage.copy(&object, &staged).await?;
                self.storage.rename(&staged, &file).await
            }
            .await;
            if let Err(err) = restored {
                log::warn!("Could not put back {}: {err}", file.display());
                let _ = self.storage.remove_file(&staged).await;
            }
        }
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
    pub async fn pull_outcome(
        &self,
        host_config_opt: Option<HostConfig>,
    ) -> Res<flow::PullPreview> {
        let (package_home, lineage) = self.lineage.read(&self.storage).await?;

        // A local-only package has no `latest` tag to resolve: `snapshot_for_pull`
        // would either error at `remote()?` (no `remote_uri`) or 404 on the
        // never-created `latest` tag. Mirror `UpstreamState::from`'s Local shape
        // and report `UpToDate` without any network, restoring the pre-reorder
        // contract. A remote whose local hash is empty but whose `latest` tag has
        // moved classifies as `Diverged` (not `Local`), so it still fetches below.
        if UpstreamState::from(lineage.clone()) == UpstreamState::Local {
            return Ok(nothing_to_pull());
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
            return Ok(nothing_to_pull());
        }

        let remote_uri = lineage.remote()?.clone();
        let base = self.manifest_from_lineage(&lineage).await?;
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
            &*self.remote,
            &package_home,
            host_config,
        )
        .await
        {
            Ok((_, snapshot)) => snapshot,
            Err(Error::PackageOp(PackageOpError::AlreadyUpToDate)) => {
                return Ok(nothing_to_pull());
            }
            Err(err) => return Err(err),
        };

        // `upstream_state` is computed from the ctor-refreshed lineage, so
        // `Diverged`/`Ahead` still report `UpToDate` (nothing to pull), matching
        // the previous contract.
        if snapshot.status.upstream_state != UpstreamState::Behind {
            return Ok(nothing_to_pull());
        }
        // Same reconciling pass the pull itself runs: a preview that called a
        // byte-identical edit a conflict would send the user to resolve one the
        // pull would not raise.
        let identical = flow::identical_to_latest(
            &self.storage,
            &package_home,
            &snapshot.status,
            &base,
            &snapshot.latest_manifest,
        )
        .await?;
        Ok(flow::PullPreview {
            added: flow::remote_additions(&base, &snapshot.latest_manifest),
            outcome: flow::classify_pull(
                &snapshot.status,
                &base,
                &snapshot.latest_manifest,
                &identical,
            ),
        })
    }

    /// Pushes any pending local commit, then promotes the resulting remote
    /// hash to `latest`. Last-writer-wins: any concurrent move of the
    /// `latest` tag between push and tag is overwritten. Invoked from the
    /// merge page when the user resolves a `Diverged` state in favor of
    /// their own revision.
    pub async fn certify_latest(&self) -> Res<ManifestUri> {
        let (_, lineage) = self.lineage.read(&self.storage).await?;

        // Push first so the hash we tag exists on remote. The push writes the
        // entry, so certify starts afresh from what it wrote.
        if lineage.commit.is_some() {
            self.push(None).await?;
        }
        let (_, started) = self.lineage.read(&self.storage).await?;

        let pushed_manifest_uri = started.remote()?.clone();
        let next =
            flow::certify_latest(started.clone(), &*self.remote, pushed_manifest_uri.clone())
                .await?;
        // The tag moved, so `latest_hash` is a fact and always lands;
        // `base_hash` only while the entry still names the hash it tagged.
        self.lineage
            .update(&self.storage, |entry| {
                let current = entry
                    .as_mut()
                    .ok_or_else(|| write_step::not_installed(&self.namespace))?;
                current.latest_hash.clone_from(&next.latest_hash);
                if current.remote_uri.as_ref() == Some(&pushed_manifest_uri) {
                    current.base_hash.clone_from(&next.base_hash);
                }
                Ok(current.remote()?.clone())
            })
            .await
    }

    pub async fn reset_to_latest(&self) -> Res<ManifestUri> {
        self.scaffold_paths().await?;

        let (package_home, lineage) = self.lineage.read(&self.storage).await?;
        let remote_uri = lineage.remote()?.clone();

        self.scaffold_paths_for_caching(&remote_uri.bucket).await?;

        let mut manifest = self.manifest_from_lineage(&lineage).await?;
        let started = lineage.clone();
        let next = flow::reset_to_latest(
            lineage,
            &mut manifest,
            &self.paths,
            &self.storage,
            &*self.remote,
            package_home.clone(),
            self.namespace.clone(),
        )
        .await?;
        let lineage = self
            .write_latest(started, next, &package_home, "resetting")
            .await?;
        Ok(lineage.remote()?.clone())
    }

    /// Discard this package's newest local commit and restore the revision
    /// before it. See [`flow::undo_commit`].
    ///
    /// The remote check below is narrower than the chain rule: a remote-backed
    /// package with unpushed commits still has a chain, but undoing there would
    /// leave a pending commit equal to its own base.
    pub async fn undo_commit(&self) -> Res<CommitState> {
        self.scaffold_paths().await?;

        let (package_home, lineage) = self.lineage.read(&self.storage).await?;
        if lineage.remote_uri.is_some() {
            return Err(Error::PackageOp(crate::PackageOpError::Undo(
                "this package has a remote, so its commit chain has been consumed by pushing; \
                 undo is only available before the first push"
                    .to_string(),
            )));
        }

        let started = lineage.clone();
        let (next, commit) = flow::undo_commit(
            lineage,
            &self.paths,
            &self.storage,
            package_home,
            self.namespace.clone(),
        )
        .await?;
        self.write_local(&started, &next, "undoing the commit")
            .await?;
        Ok(commit)
    }

    /// The write step of a writer that changed only local state: undo commit,
    /// set remote and its recommit. If the revision moved meanwhile it
    /// refuses: nothing remote happened, and the user can press again.
    async fn write_local(
        &self,
        started: &lineage::PackageLineage,
        next: &lineage::PackageLineage,
        verb: &'static str,
    ) -> Res {
        self.lineage
            .update(&self.storage, |entry| {
                let current = entry
                    .as_mut()
                    .ok_or_else(|| write_step::not_installed(&self.namespace))?;
                if !write_step::same_revision(current, started) {
                    return Err(write_step::changed_underneath(&self.namespace, verb));
                }
                *current = write_step::own_change(started, next, current);
                Ok(())
            })
            .await
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
        let started = lineage.clone();
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
                .recommit_for_remote(&started, lineage.clone(), origin, bucket, workflow)
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
                    self.write_local(&started, &lineage, "setting the remote")
                        .await?;
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
        self.write_local(&started, &lineage, "setting the remote")
            .await?;

        Ok(SetRemoteOutcome::default())
    }

    async fn recommit_for_remote(
        &self,
        started: &lineage::PackageLineage,
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
            fetch_workflows_config(&*self.remote, host.as_ref(), &workflows_config_uri).await?;
        // Publish later pushes this pending recommit *without* re-resolving the
        // workflow, so recommit must stamp the caller's chosen workflow now.
        // With `WorkflowIntent::BucketDefault` (the no-gesture path) this picks
        // up the bucket's `default_workflow`, so a locally-created package's
        // first publish is governed even when the user expresses no choice.
        let workflow = resolve_workflow_from_config(
            &*self.remote,
            host.as_ref(),
            workflow,
            config_uri,
            workflows_config.as_ref(),
        )
        .await?;
        let manifest = self.manifest_from_lineage(started).await?;
        let next = flow::recommit(
            lineage,
            &manifest,
            &self.paths,
            &self.storage,
            &*self.remote,
            host.as_ref(),
            self.namespace.clone(),
            host_config,
            workflow,
            workflows_config.as_ref(),
        )
        .await?;
        self.write_local(started, &next, "setting the remote").await
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
        resolve_workflow(&*self.remote, origin.as_ref(), intent, &config_uri).await
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
            fetch_workflows_config(&*self.remote, origin.as_ref(), &config_uri).await?;
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
            fetch_workflows_config(&*self.remote, origin.as_ref(), &config_uri).await?;
        let Some(config) = config else {
            return Ok(None);
        };
        Ok(Some(
            fetch_workflow_rules(&*self.remote, origin.as_ref(), &config, workflow_id).await?,
        ))
    }
}

#[cfg(test)]
mod current_revision_tests;
#[cfg(test)]
mod older_revision_tests;
#[cfg(test)]
mod ordered_writers_tests;
#[cfg(test)]
mod resolve_comparison_tests;
#[cfg(test)]
mod revision_history_tests;
#[cfg(test)]
mod set_remote_tests;
#[cfg(test)]
mod sync_flow_tests;
#[cfg(test)]
mod write_step_tests;
