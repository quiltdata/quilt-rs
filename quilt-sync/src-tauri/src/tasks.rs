//! Tasks a package revision asks for: detected on the autopull tick, claimed by
//! publishing a revision, run by a local agent over ACP in the package's folder,
//! and closed by publishing what the agent left as the next revision.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;

use quilt_rs::flow::UserMeta;
use quilt_rs::io::remote::WorkflowIntent;
use quilt_uri::Namespace;
use serde_json::Value;

use crate::Error;
use crate::model::QuiltModel;
use crate::quilt;
use crate::quilt::lineage::UpstreamState;
use crate::telemetry::prelude::*;

pub mod acp;
pub mod agent_exec;
pub mod settings;

use agent_exec::Claim;
use agent_exec::Verdict;
use settings::AgentSettings;
use settings::TaskSettings;

pub type SpawnRun = Arc<dyn Fn(Run) + Send + Sync>;

pub struct Tasks {
    /// `None` keeps tasks off.
    data_dir: Option<PathBuf>,
    in_flight: Arc<Mutex<BTreeSet<Namespace>>>,
    spawn_run: SpawnRun,
}

impl Default for Tasks {
    fn default() -> Self {
        Self {
            data_dir: None,
            in_flight: Arc::default(),
            spawn_run: Arc::new(|_| {}),
        }
    }
}

impl Tasks {
    pub fn new(data_dir: PathBuf, spawn_run: SpawnRun) -> Self {
        Self {
            data_dir: Some(data_dir),
            spawn_run,
            ..Self::default()
        }
    }

    /// Whether a run holds `namespace`: autosync neither pulls nor publishes it
    /// from just before its claim until its last revision.
    pub fn holds(&self, namespace: &Namespace) -> bool {
        lock(&self.in_flight).contains(namespace)
    }

    /// The settings as the file says now; `None` while tasks are off.
    pub async fn settings(&self) -> Option<TaskSettings> {
        let data_dir = self.data_dir.as_ref()?;
        TaskSettings::load(data_dir).await.unwrap_or_else(|err| {
            warn!("tasks: cannot read the task settings, tasks are off: {err}");
            None
        })
    }

    pub fn hold(&self, namespace: &Namespace) -> Hold {
        lock(&self.in_flight).insert(namespace.clone());
        Hold {
            namespace: namespace.clone(),
            in_flight: Arc::clone(&self.in_flight),
        }
    }
}

fn lock(in_flight: &Mutex<BTreeSet<Namespace>>) -> std::sync::MutexGuard<'_, BTreeSet<Namespace>> {
    in_flight
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// A package held for a run; dropping it lets autosync back in.
pub struct Hold {
    namespace: Namespace,
    in_flight: Arc<Mutex<BTreeSet<Namespace>>>,
}

impl Drop for Hold {
    fn drop(&mut self) {
        lock(&self.in_flight).remove(&self.namespace);
    }
}

pub struct Run {
    pub namespace: Namespace,
    pub home: PathBuf,
    pub document: String,
    pub agent: AgentSettings,
    pub claim: Claim,
    /// The claim revision's metadata, which the last revision starts from.
    pub claimed_meta: Value,
    pub claim_hash: String,
    pub workflow: WorkflowIntent,
    /// Released when the run ends, by being dropped with it.
    _hold: Hold,
}

pub async fn check<M: QuiltModel>(
    model: &M,
    tasks: &Tasks,
    settings: &TaskSettings,
    pkg: &quilt::InstalledPackage,
) {
    if let Err(err) = try_check(model, tasks, settings, pkg).await {
        warn!("tasks: {}: {err}", pkg.namespace);
    }
}

async fn try_check<M: QuiltModel>(
    model: &M,
    tasks: &Tasks,
    settings: &TaskSettings,
    pkg: &quilt::InstalledPackage,
) -> Result<(), Error> {
    let Some(data_dir) = tasks.data_dir.as_deref() else {
        return Ok(());
    };
    // A held package's run is alive, so its claim is not an interrupted one.
    if tasks.holds(&pkg.namespace) {
        return Ok(());
    }
    let lineage = model.get_installed_package_lineage(pkg).await?;
    let Some(remote) = lineage.remote_uri.as_ref() else {
        return Ok(());
    };
    if !settings.blesses(&remote.bucket) {
        return Ok(());
    }
    let manifest = model.get_installed_package_manifest(pkg, &lineage).await?;
    let Some(user_meta) = manifest.header.user_meta.as_ref() else {
        return Ok(());
    };
    if user_meta.get("agent_exec").is_none() {
        return Ok(());
    }
    let own = settings::own_invocations(data_dir).await?;
    let verdict = agent_exec::classify(
        user_meta,
        |path| manifest.rows.iter().any(|row| row.logical_key == path),
        &settings.agents,
        &own,
    );
    if verdict == Verdict::LeaveAlone {
        return Ok(());
    }

    let Some(locked) = model.try_lock_package(pkg).await? else {
        return Ok(());
    };
    // Before the walk, so the claim publishes a status that includes it.
    if let Verdict::Claim { document, .. } = &verdict {
        download(model, pkg, &locked, &lineage, document).await?;
    }
    let Some(status) = clean_at_head(model, pkg, &locked, &lineage).await? else {
        return Ok(());
    };
    let workflow = carried_workflow(manifest.header.workflow.as_ref());

    let (meta, message) = match verdict {
        Verdict::LeaveAlone => return Ok(()),
        Verdict::Unrunnable { reason } => (
            agent_exec::unrunnable(user_meta, &reason),
            format!("Task cannot run: {reason}"),
        ),
        Verdict::Interrupted => (
            agent_exec::interrupted(user_meta),
            "Task failed: interrupted by restart".to_string(),
        ),
        Verdict::Claim { document, agent } => {
            let home = model.package_home(&pkg.namespace).await?;
            let hold = tasks.hold(&pkg.namespace);
            let claim = Claim {
                invocation_id: uuid::Uuid::new_v4().to_string(),
                claimed_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            };
            // Before the push, so a crash after it still knows the claim as ours.
            settings::remember_invocation(data_dir, &claim.invocation_id).await?;
            let claimed_meta = agent_exec::with_claim(user_meta, claim.claimed());
            let message = format!("Task {document} claimed");
            let outcome = publish(
                model,
                &locked,
                pkg,
                workflow.clone(),
                claimed_meta.clone(),
                message,
                status,
            )
            .await?;
            let push = outcome.push();
            if !push.certified_latest {
                info!(
                    "tasks: {}: another revision became latest first, not running",
                    pkg.namespace
                );
                return Ok(());
            }
            (tasks.spawn_run)(Run {
                namespace: pkg.namespace.clone(),
                home,
                document,
                agent: settings.agents[&agent].clone(),
                claim,
                claimed_meta,
                claim_hash: push.manifest_uri.hash.clone(),
                workflow,
                _hold: hold,
            });
            return Ok(());
        }
    };
    publish(model, &locked, pkg, workflow, meta, message, status).await?;
    Ok(())
}

/// The tree's status, walked under `locked`, when a revision can change only
/// the metadata: a clean tree, no pending commit, and `head` still the
/// package's revision and at a `latest` read just now. With a pending commit,
/// publishing pushes that commit and drops the new metadata.
async fn clean_at_head<M: QuiltModel>(
    model: &M,
    pkg: &quilt::InstalledPackage,
    locked: &M::Locked,
    head: &quilt::lineage::PackageLineage,
) -> Result<Option<quilt::lineage::InstalledPackageStatus>, Error> {
    let status = model.locked_package_status(locked, None).await?;
    let lineage = model.get_installed_package_lineage(pkg).await?;
    let clean = status.latest_refreshed
        && status.upstream_state == UpstreamState::UpToDate
        && status.changes.is_empty()
        && lineage.commit.is_none()
        && lineage.current_hash() == head.current_hash();
    Ok(clean.then_some(status))
}

/// Put `document` in the package's folder: a sparse copy lists it without
/// having it on disk. A file already there, tracked or not, is left alone; one
/// that differs from the revision fails the clean-tree walk. Anything else at
/// that path is not a file the walk compares, so the download refuses it. The
/// walk skips symlinks, and a download through a linked folder would land
/// outside the package, so a link anywhere on the path stops the claim.
async fn download<M: QuiltModel>(
    model: &M,
    pkg: &quilt::InstalledPackage,
    locked: &M::Locked,
    lineage: &quilt::lineage::PackageLineage,
    document: &str,
) -> Result<(), Error> {
    let path = PathBuf::from(document);
    if lineage.paths.contains_key(&path) {
        return Ok(());
    }
    let home = model.package_home(&pkg.namespace).await?;
    let target = home.join(&path);
    let mut on_disk = home;
    for part in path.components() {
        on_disk.push(part);
        let Ok(meta) = tokio::fs::symlink_metadata(&on_disk).await else {
            break;
        };
        if meta.is_symlink() {
            return Err(Error::General(format!(
                "{document} is reached through a symlink"
            )));
        }
        if on_disk == target && meta.is_file() {
            return Ok(());
        }
    }
    let report = model.locked_package_install_paths(locked, &[path]).await?;
    if report.skipped.is_empty() {
        Ok(())
    } else {
        Err(Error::General(format!(
            "{document} could not be downloaded"
        )))
    }
}

/// The workflow the head was published under, so a governed bucket accepts the
/// next revision too.
fn carried_workflow(workflow: Option<&quilt::manifest::Workflow>) -> WorkflowIntent {
    match workflow {
        Some(quilt::manifest::Workflow { id: Some(id), .. }) => {
            WorkflowIntent::Named(id.id.clone())
        }
        Some(_) => WorkflowIntent::NoWorkflow,
        None => WorkflowIntent::BucketDefault,
    }
}

async fn publish<M: QuiltModel>(
    model: &M,
    locked: &M::Locked,
    pkg: &quilt::InstalledPackage,
    workflow: WorkflowIntent,
    meta: Value,
    message: String,
    status: quilt::lineage::InstalledPackageStatus,
) -> Result<quilt::PublishOutcome, Error> {
    let workflow = model.resolve_workflow(pkg, workflow).await?;
    model
        .locked_package_publish(
            locked,
            message,
            UserMeta::Set(meta),
            workflow,
            None,
            Some(status),
        )
        .await
}

pub async fn execute<M: QuiltModel>(model: &M, run: Run) {
    let outcome = acp::run(&run.agent, &run.home, &run.document, async || {
        claim_is_latest(model, &run.namespace, &run.claim_hash)
            .await
            .map_err(|err| format!("could not re-read the package: {err}"))
    })
    .await;
    info!("tasks: {}: {outcome:?}", run.namespace);
    if let Err(err) = finish(model, &run, &outcome).await {
        warn!(
            "tasks: {}: publishing the outcome failed: {err}",
            run.namespace
        );
    }
}

async fn claim_is_latest<M: QuiltModel>(
    model: &M,
    namespace: &Namespace,
    claim_hash: &str,
) -> Result<bool, Error> {
    let installed = installed(model, namespace).await?;
    let status = model.get_installed_package_status(&installed, None).await?;
    if !status.latest_refreshed {
        return Err(Error::General(
            "the remote's latest could not be read".into(),
        ));
    }
    Ok(status.latest_hash == claim_hash)
}

async fn installed<M: QuiltModel>(
    model: &M,
    namespace: &Namespace,
) -> Result<quilt::InstalledPackage, Error> {
    model
        .get_installed_package(namespace)
        .await?
        .ok_or_else(|| Error::from(quilt::InstallPackageError::NotInstalled(namespace.clone())))
}

async fn finish<M: QuiltModel>(
    model: &M,
    run: &Run,
    outcome: &acp::RunOutcome,
) -> Result<(), Error> {
    let Some(claim) = run.claim.outcome(outcome) else {
        return Ok(());
    };
    let message = format!(
        "Task {} {}",
        run.document,
        claim["status"].as_str().unwrap_or_default()
    );
    let installed = installed(model, &run.namespace).await?;
    let locked = model.lock_package(&installed).await?;
    let status = model.locked_package_status(&locked, None).await?;
    let meta = agent_exec::with_claim(&run.claimed_meta, claim);
    publish(
        model,
        &locked,
        &installed,
        run.workflow.clone(),
        meta,
        message,
        status,
    )
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests;
