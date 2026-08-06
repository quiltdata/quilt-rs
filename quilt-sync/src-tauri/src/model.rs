use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use mockall::automock;
use mockall::predicate::str;

use tokio::sync;

use tokio_stream::StreamExt;

use crate::error::Error;
use crate::quilt;
use crate::telemetry::prelude::*;

use quilt_rs::RoleInfo;
use quilt_rs::flow::PullOutcome;
use quilt_rs::flow::UserMeta;
use quilt_rs::io::remote::HostConfig;
use quilt_rs::io::remote::WorkflowIntent;
use quilt_rs::io::remote::WorkflowsConfig;
use quilt_rs::io::remote::fetch_workflows_config_for_bucket;
use quilt_rs::workflow::WorkflowRules;

use quilt_uri::Host;

/// Result of checking whether a package is already installed.
#[derive(Debug)]
pub enum InstallCheck {
    /// Exact same hash — already up to date
    AlreadyInstalled,
    /// Same namespace, different hash — needs pull.
    /// Contains the hash of the currently installed version.
    DifferentVersion(String),
    /// Installed locally without a remote origin
    LocalOnly,
    /// Not installed at all
    NotInstalled,
}

/// Result of attempting to install a package.
#[derive(Debug, PartialEq)]
pub enum InstallOutcome {
    /// Package was installed (or was already installed with the same hash).
    Installed,
    /// A different version is already installed.
    DifferentVersion {
        requested_hash: String,
        installed_hash: String,
    },
    /// Installed locally without a remote origin.
    LocalOnly,
}

pub struct Model {
    quilt: sync::Mutex<quilt::LocalDomain>,
}

#[automock]
pub trait QuiltModel {
    fn get_quilt(&self) -> &sync::Mutex<quilt::LocalDomain>;

    async fn browse_remote_manifest(
        &self,
        remote_manifest: &quilt_uri::ManifestUri,
    ) -> Result<quilt::manifest::Manifest, Error> {
        Ok(self
            .get_quilt()
            .lock()
            .await
            .browse_remote_manifest(remote_manifest)
            .await?)
    }

    async fn get_installed_packages_list(&self) -> Result<Vec<quilt::InstalledPackage>, Error> {
        Ok(self
            .get_quilt()
            .lock()
            .await
            .list_installed_packages()
            .await?)
    }

    async fn get_installed_package(
        &self,
        namespace: &quilt_uri::Namespace,
    ) -> Result<Option<quilt::InstalledPackage>, Error> {
        Ok(self
            .get_quilt()
            .lock()
            .await
            .get_installed_package(namespace)
            .await?)
    }

    async fn get_installed_package_lineage(
        &self,
        package: &quilt::InstalledPackage,
    ) -> Result<quilt::lineage::PackageLineage, Error> {
        Ok(package.lineage().await?)
    }

    async fn get_installed_package_records(
        &self,
        package: &quilt::InstalledPackage,
    ) -> Result<BTreeMap<PathBuf, quilt::manifest::ManifestRow>, Error> {
        let manifest = package.manifest().await?;
        let mut stream = manifest.records_stream().await;
        let mut records = BTreeMap::new();
        while let Some(page) = stream.next().await {
            if let Ok(rows) = page {
                for row in rows.into_iter().flatten() {
                    records.insert(row.logical_key.clone(), row);
                }
            }
        }
        Ok(records)
    }

    async fn get_installed_package_status(
        &self,
        package: &quilt::InstalledPackage,
        host_config: Option<HostConfig>,
    ) -> Result<quilt::lineage::InstalledPackageStatus, Error> {
        Ok(package.status(host_config).await?)
    }

    async fn recompute_local_status(
        &self,
        package: &quilt::InstalledPackage,
        host_config: Option<HostConfig>,
    ) -> Result<quilt::lineage::InstalledPackageStatus, Error> {
        Ok(package.recompute_local_status(host_config).await?)
    }

    async fn package_commit(
        &self,
        package: &quilt::InstalledPackage,
        message: String,
        metadata: UserMeta,
        workflow: Option<quilt::manifest::Workflow>,
        host_config: Option<HostConfig>,
    ) -> Result<quilt::lineage::CommitState, Error> {
        Ok(package
            .commit(message, metadata, workflow, host_config)
            .await?)
    }

    async fn package_install_paths(
        &self,
        package: &quilt::InstalledPackage,
        paths: &[PathBuf],
    ) -> Result<quilt::lineage::LineagePaths, Error> {
        Ok(package.install_paths(paths).await?)
    }

    /// TODO(sync-entire-package/u-desktop-plumbing): the scope passed here must
    /// become the package's stored `SyncScope` combined (logical and) with the
    /// experimental gate, on this path and on the autopull tick. Until it lands the
    /// app asks for the narrow scope, so behaviour is exactly as before.
    async fn package_pull(
        &self,
        package: &quilt::InstalledPackage,
        host_config: Option<HostConfig>,
    ) -> Result<quilt_uri::ManifestUri, Error> {
        Ok(package
            .pull(host_config, quilt::lineage::SyncScope::IndividualFiles)
            .await?)
    }

    /// Dry-run classifier: what would `package_pull` do right now? Delegates
    /// to the engine's [`InstalledPackage::pull_outcome`]; wrapping it on the
    /// trait lets the autosync tick route on the [`PullOutcome`] through a
    /// `MockQuiltModel` in unit tests without hitting real storage.
    async fn package_pull_outcome(
        &self,
        package: &quilt::InstalledPackage,
    ) -> Result<PullOutcome, Error> {
        Ok(package.pull_outcome(None).await?)
    }

    async fn is_package_installed(
        &self,
        manifest_uri: &quilt_uri::ManifestUri,
    ) -> Result<InstallCheck, Error> {
        match self.get_installed_package(&manifest_uri.namespace).await? {
            Some(installed_package) => {
                let package_lineage = self
                    .get_installed_package_lineage(&installed_package)
                    .await?;
                let Some(installed_manifest_uri) = package_lineage.remote_uri.as_ref() else {
                    return Ok(InstallCheck::LocalOnly);
                };
                if manifest_uri.hash == installed_manifest_uri.hash {
                    Ok(InstallCheck::AlreadyInstalled)
                } else {
                    Ok(InstallCheck::DifferentVersion(
                        installed_manifest_uri.hash.clone(),
                    ))
                }
            }
            None => Ok(InstallCheck::NotInstalled),
        }
    }

    async fn is_path_installed(
        &self,
        package: &quilt::InstalledPackage,
        path: &PathBuf,
    ) -> Result<bool, Error> {
        let package_lineage = self.get_installed_package_lineage(package).await?;
        Ok(package_lineage.paths.contains_key(path))
    }

    async fn package_push(
        &self,
        package: &quilt::InstalledPackage,
        host_config: Option<HostConfig>,
    ) -> Result<quilt::PushOutcome, Error> {
        Ok(package.push(host_config).await?)
    }

    async fn package_publish(
        &self,
        package: &quilt::InstalledPackage,
        message: String,
        metadata: UserMeta,
        workflow: Option<quilt::manifest::Workflow>,
        host_config: Option<HostConfig>,
        status: Option<quilt::lineage::InstalledPackageStatus>,
    ) -> Result<quilt::PublishOutcome, Error> {
        Ok(package
            .publish(message, metadata, workflow, host_config, status)
            .await?)
    }

    /// Resolve a [`WorkflowIntent`] into the materialised
    /// [`quilt::manifest::Workflow`] the remote enforces. Wrapping the
    /// `InstalledPackage` method on the trait lets the autosync tick
    /// path go through `model::package_publish` (free function) without
    /// hitting real storage in mock-based unit tests.
    async fn resolve_workflow(
        &self,
        package: &quilt::InstalledPackage,
        workflow: WorkflowIntent,
    ) -> Result<Option<quilt::manifest::Workflow>, Error> {
        Ok(package.resolve_workflow(workflow).await?)
    }

    /// Fetch and parse the bucket's declared workflows for this package's remote.
    /// Returns `Ok(None)` for a package with no remote or a bucket with no
    /// config, letting the commit dialog degrade to today's control.
    async fn get_workflows_config(
        &self,
        package: &quilt::InstalledPackage,
    ) -> Result<Option<WorkflowsConfig>, Error> {
        Ok(package.workflows_config().await?)
    }

    /// Fetch and compile the pure-validator [`WorkflowRules`] for a named
    /// workflow in this package's bucket config, for live commit-dialog
    /// validation. Wrapping the `InstalledPackage` method on the trait lets the
    /// workflow-rules cache go through a `MockQuiltModel` in unit tests (so a
    /// second, cache-hitting load can be asserted to skip the fetch) without
    /// hitting real storage. Returns `Ok(None)` for an ungoverned package.
    async fn get_workflow_rules(
        &self,
        package: &quilt::InstalledPackage,
        workflow_id: &str,
    ) -> Result<Option<WorkflowRules>, Error> {
        Ok(package.workflow_rules(workflow_id).await?)
    }

    /// Fetch and parse a bucket's declared workflows directly from its
    /// `.quilt/workflows/config.yml`, independent of any package's remote.
    /// Lets the set-remote popup preview a bucket's governance before the
    /// choice is committed. Returns `Ok(None)` when the bucket has no config.
    async fn get_bucket_workflows_config(
        &self,
        host: Option<quilt_uri::Host>,
        bucket: &str,
    ) -> Result<Option<WorkflowsConfig>, Error> {
        let quilt = self.get_quilt().lock().await;
        Ok(fetch_workflows_config_for_bucket(quilt.get_remote(), host.as_ref(), bucket).await?)
    }

    async fn package_revision_certify_latest(
        &self,
        package: &quilt::InstalledPackage,
    ) -> Result<quilt_uri::ManifestUri, Error> {
        Ok(package.certify_latest().await?)
    }

    async fn package_revision_reset_local(
        &self,
        package: &quilt::InstalledPackage,
    ) -> Result<quilt_uri::ManifestUri, Error> {
        Ok(package.reset_to_latest().await?)
    }

    /// Set the package's remote, returning `Some(reason)` when the bucket's
    /// default workflow could not be resolved on the best-effort path (the
    /// remote is still set; the caller surfaces the reason), else `None`.
    async fn set_remote(
        &self,
        package: &quilt::InstalledPackage,
        origin: quilt_uri::Host,
        bucket: String,
        workflow: WorkflowIntent,
    ) -> Result<Option<String>, Error> {
        Ok(package
            .set_remote(bucket, Some(origin), workflow)
            .await?
            .resolution_warning)
    }

    async fn package_create(
        &self,
        namespace: quilt_uri::Namespace,
        source: Option<PathBuf>,
        message: Option<String>,
    ) -> Result<quilt::InstalledPackage, Error> {
        Ok(self
            .get_quilt()
            .lock()
            .await
            .create_package(namespace, source, message)
            .await?)
    }

    async fn package_install(
        &self,
        remote_manifest: &quilt_uri::ManifestUri,
    ) -> Result<quilt::InstalledPackage, Error> {
        Ok(self
            .get_quilt()
            .lock()
            .await
            .install_package(remote_manifest)
            .await?)
    }

    async fn package_uninstall(&self, namespace: quilt_uri::Namespace) -> Result<(), Error> {
        Ok(self
            .get_quilt()
            .lock()
            .await
            .uninstall_package(namespace)
            .await?)
    }

    async fn package_home(&self, namespace: &quilt_uri::Namespace) -> Result<PathBuf, Error> {
        let installed_package = self
            .get_installed_package(namespace)
            .await?
            .ok_or_else(|| {
                Error::from(quilt::InstallPackageError::NotInstalled(namespace.clone()))
            })?;
        let working_folder_path = installed_package.package_home().await?;
        if !working_folder_path.exists() {
            return Err(Error::FsOpen(crate::error::FsOpenError::PathNotFound(
                working_folder_path,
            )));
        }

        Ok(working_folder_path)
    }

    async fn file_path(
        &self,
        namespace: &quilt_uri::Namespace,
        relative_path: &PathBuf,
    ) -> Result<PathBuf, Error> {
        let package_home = self.package_home(namespace).await?;
        let file_path = package_home.join(relative_path);
        if !file_path.exists() {
            return Err(Error::FsOpen(crate::error::FsOpenError::PathNotFound(
                file_path,
            )));
        }
        Ok(file_path)
    }

    async fn open_in_file_browser(
        &self,
        namespace: &quilt_uri::Namespace,
    ) -> Result<PathBuf, Error> {
        let dir_path = self.package_home(namespace).await?;
        opener::open_browser(&dir_path)?;
        Ok(dir_path)
    }

    async fn reveal_in_file_browser(
        &self,
        namespace: &quilt_uri::Namespace,
        path: &PathBuf,
    ) -> Result<PathBuf, Error> {
        let file_path = self.file_path(namespace, path).await?;
        opener::reveal(&file_path)?;
        Ok(file_path)
    }

    async fn open_in_default_application(
        &self,
        namespace: &quilt_uri::Namespace,
        path: &PathBuf,
    ) -> Result<PathBuf, Error> {
        let file_path = self.file_path(namespace, path).await?;
        opener::open(&file_path)?;
        Ok(file_path)
    }

    async fn resolve_manifest_uri(
        &self,
        uri: &quilt_uri::S3PackageUri,
    ) -> Result<quilt_uri::ManifestUri, Error> {
        Ok(quilt::io::manifest::resolve_manifest_uri(
            self.get_quilt().lock().await.get_remote(),
            uri.catalog.as_ref(),
            uri,
        )
        .await?)
    }

    /// Drop the remote's cached S3 clients (and their in-memory
    /// credentials) for `host`, or for all hosts when `None`. Invoked on
    /// logout and on a role switch so credentials stop working immediately
    /// rather than lingering in a cached client until STS expiry. It lives
    /// on the trait so a `MockQuiltModel` can record the flush — the one
    /// step that makes either operation take effect in-process.
    ///
    /// The host is owned, not `Option<&Host>`, because `#[automock]` cannot
    /// build an expectation for a reference nested inside another type
    /// (same reason as [`QuiltModel::get_bucket_workflows_config`]).
    async fn clear_remote_client_cache(&self, host: Option<Host>) {
        self.get_quilt()
            .lock()
            .await
            .get_remote()
            .clear_client_cache(host.as_ref());
    }

    /// Buckets the active role can read on `host`.
    ///
    /// An optimistic hint, not an authoritative answer: it over-reports for
    /// unmanaged roles and anonymous-access stacks (the registry returns
    /// every in-stack bucket when it cannot introspect the role) and is
    /// read-presence only — it never distinguishes read from write. A
    /// listed bucket can still deny, so callers must keep treating an
    /// `AccessDenied` from the real call as the authority.
    ///
    /// Takes a [`LocalDomain::remote_handle`] and releases the domain lock
    /// before the round trip. This one runs on the roster's first paint, so
    /// holding the lock across it would stall every other Tauri command and
    /// the autopull tick behind a registry call.
    async fn readable_buckets(&self, host: &Host) -> Result<Vec<String>, Error> {
        let remote = self.get_quilt().lock().await.remote_handle();
        Ok(remote.readable_buckets(host).await?)
    }

    /// Read the active role for `host`. Releases the domain lock before the
    /// round trip, for the same reason as
    /// [`QuiltModel::readable_buckets`] — the roster reaches this one too.
    async fn refresh_roles(&self, host: &Host) -> Result<RoleInfo, Error> {
        let remote = self.get_quilt().lock().await.remote_handle();
        Ok(remote.refresh_roles(host).await?)
    }

    /// Switch the primary role. The caller **must** follow this with
    /// [`QuiltModel::clear_remote_client_cache`] for the same host —
    /// expiring the stored credentials does not touch a cached client's own
    /// in-memory copy, so without it the old role keeps signing.
    ///
    /// Releases the domain lock before the round trip, for the same reason as
    /// [`QuiltModel::readable_buckets`]. This one is worse than the reads:
    /// it resolves the registry URL, runs a GraphQL mutation and may refresh
    /// the access token in between, so holding the lock would park every
    /// other Tauri command and the autopull tick behind a slow registry for
    /// as long as the switch takes.
    async fn switch_role(&self, host: &Host, role_name: &str) -> Result<RoleInfo, Error> {
        let remote = self.get_quilt().lock().await.remote_handle();
        Ok(remote.switch_role(host, role_name).await?)
    }
}

impl QuiltModel for Model {
    fn get_quilt(&self) -> &sync::Mutex<quilt::LocalDomain> {
        &self.quilt
    }
}

impl Model {
    pub fn create(data_dir: impl AsRef<Path>) -> Self {
        debug!("Root directory is {:?}", data_dir.as_ref());
        let quilt = quilt::LocalDomain::new(data_dir);
        Model {
            quilt: sync::Mutex::new(quilt),
        }
    }

    pub async fn set_home(
        &self,
        directory: impl AsRef<Path>,
    ) -> Result<quilt::lineage::Home, Error> {
        Ok(self.get_quilt().lock().await.set_home(directory).await?)
    }

    /// Expire `host`'s cached STS credentials without logging out. Pair it
    /// with [`QuiltModel::clear_remote_client_cache`] — see
    /// [`QuiltModel::switch_role`].
    #[expect(
        dead_code,
        reason = "plumbing for the role UI; drop this attribute with the first caller"
    )]
    pub async fn expire_credentials(&self, host: &Host) -> Result<(), Error> {
        // Takes a handle and drops the domain lock first: the expiry waits on
        // this host's refresh lock, which an in-flight vend can hold across a
        // network round trip. Same rule as [`QuiltModel::switch_role`].
        let remote = self.quilt.lock().await.remote_handle();
        Ok(remote.expire_credentials(host).await?)
    }
}

mod ops;
pub use ops::*;

#[cfg(test)]
pub mod mocks;

/// Does a role call keep the domain mutex while it works?
///
/// `lock().await.get_remote().call().await` keeps the guard alive to the end
/// of the *statement*, so the domain stays locked for the whole round trip and
/// every other Tauri command — and the autopull tick — parks behind it.
/// Taking a [`quilt::LocalDomain::remote_handle`] and letting the guard drop
/// first is the shape these tests pin.
///
/// The suspension observed here is the token-file read rather than the
/// registry call, because a test cannot make a registry call hang on demand.
/// It stands in for it: the question is only whether the lock is still held
/// while the call is waiting on something, and the first thing every one of
/// these calls waits on is that file.
///
/// Making the suspension *certain* is the reason for the hand-built runtime.
/// `tokio::fs` dispatches to the blocking pool, and a pool with a free thread
/// can finish the read before the future is first polled — which would make
/// the assertion vanish rather than fail. These tests run on a pool of
/// exactly one thread and occupy it, so the read is guaranteed to queue and
/// the call is guaranteed to suspend.
#[cfg(test)]
mod domain_lock_tests {
    use std::future::Future;
    use std::future::poll_fn;
    use std::pin::pin;
    use std::str::FromStr;
    use std::task::Poll;

    use tempfile::TempDir;
    use tokio::runtime::Runtime;
    use tokio::sync::oneshot;

    use super::Model;
    use super::QuiltModel;
    use quilt_uri::Host;

    /// A runtime whose blocking pool is a single thread, so a test can hold
    /// it and starve every file read the call under test issues.
    fn runtime_with_one_blocking_thread() -> Runtime {
        tokio::runtime::Builder::new_current_thread()
            .max_blocking_threads(1)
            .enable_all()
            .build()
            .expect("runtime")
    }

    /// Take the pool's only thread. Dropping the returned sender gives it
    /// back, letting the starved read — and the call waiting on it — finish.
    fn occupy_the_blocking_pool() -> oneshot::Sender<()> {
        let (release, wait) = oneshot::channel::<()>();
        tokio::task::spawn_blocking(move || {
            let _ = wait.blocking_recv();
        });
        release
    }

    /// Drive `call` to its first suspension and assert the domain mutex is
    /// free at that moment, then let it finish.
    async fn assert_domain_lock_is_free_while_awaiting<F: Future>(model: &Model, call: F) {
        let release = occupy_the_blocking_pool();
        let mut call = pin!(call);

        let first = poll_fn(|cx| Poll::Ready(call.as_mut().poll(cx))).await;
        assert!(
            first.is_pending(),
            "the call must suspend on the starved file read, or this proves nothing"
        );
        assert!(
            model.get_quilt().try_lock().is_ok(),
            "the domain lock must be free while the call is awaiting, \
             or every other command blocks behind it"
        );

        drop(release);
        let _ = call.await;
    }

    /// A switch resolves the registry URL, runs a GraphQL mutation and may
    /// refresh the access token in between. Holding the domain mutex across
    /// that means picking a role freezes the app for as long as the registry
    /// takes to answer.
    #[test]
    fn switch_role_releases_the_domain_lock_before_it_awaits() {
        runtime_with_one_blocking_thread().block_on(async {
            let temp = TempDir::new().expect("temp dir");
            let model = Model::create(temp.path());
            let host = Host::from_str("test.quilt.dev").expect("host");

            assert_domain_lock_is_free_while_awaiting(
                &model,
                model.switch_role(&host, "ReadWrite"),
            )
            .await;
        });
    }

    /// The same for the two reads, so an edit that folds either back into a
    /// single locked statement fails here too.
    #[test]
    fn the_role_reads_release_the_domain_lock_before_they_await() {
        runtime_with_one_blocking_thread().block_on(async {
            let temp = TempDir::new().expect("temp dir");
            let model = Model::create(temp.path());
            let host = Host::from_str("test.quilt.dev").expect("host");

            assert_domain_lock_is_free_while_awaiting(&model, model.refresh_roles(&host)).await;
            assert_domain_lock_is_free_while_awaiting(&model, model.readable_buckets(&host)).await;
        });
    }
}
