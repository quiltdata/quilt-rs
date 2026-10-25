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
    pub agent_id: String,
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

struct Head {
    lineage: quilt::lineage::PackageLineage,
    user_meta: Value,
    workflow: Option<quilt::manifest::Workflow>,
    verdict: Verdict,
}

/// `None` for a package outside the blessed buckets or with no `agent_exec`.
async fn read_head<M: QuiltModel>(
    model: &M,
    data_dir: &std::path::Path,
    settings: &TaskSettings,
    pkg: &quilt::InstalledPackage,
) -> Result<Option<Head>, Error> {
    let lineage = model.get_installed_package_lineage(pkg).await?;
    let Some(remote) = lineage.remote_uri.as_ref() else {
        return Ok(None);
    };
    if !settings.blesses(&remote.bucket) {
        return Ok(None);
    }
    let manifest = model.get_installed_package_manifest(pkg, &lineage).await?;
    let Some(user_meta) = manifest.header.user_meta.as_ref() else {
        return Ok(None);
    };
    if user_meta.get("agent_exec").is_none() {
        return Ok(None);
    }
    let own = settings::own_invocations(data_dir).await?;
    let verdict = agent_exec::classify(
        user_meta,
        |path| lists(&manifest, path),
        &settings.agents,
        &own,
    );
    Ok(Some(Head {
        user_meta: user_meta.clone(),
        workflow: manifest.header.workflow,
        lineage,
        verdict,
    }))
}

/// Whether the sweep can publish over a head in `state`: its base is `latest`.
fn sweepable(state: UpstreamState) -> bool {
    matches!(state, UpstreamState::UpToDate | UpstreamState::Ahead)
}

impl Tasks {
    /// Whether only the sweep may publish `pkg`: its head is this desktop's own
    /// claim with no run alive, and a fresh read finds its base at `latest` or
    /// can't read `latest`. Autosync's publish could get there first, or
    /// replace `agent_exec`. The stored lineage's `latest` is only the last
    /// push's or pull's.
    pub async fn awaits_sweep<M: QuiltModel>(
        &self,
        model: &M,
        settings: &TaskSettings,
        pkg: &quilt::InstalledPackage,
    ) -> bool {
        let Some(data_dir) = self.data_dir.as_deref() else {
            return false;
        };
        if self.holds(&pkg.namespace) {
            return false;
        }
        let Ok(Some(Head {
            lineage,
            verdict: Verdict::Interrupted,
            ..
        })) = read_head(model, data_dir, settings, pkg).await
        else {
            return false;
        };
        let Ok(Some(locked)) = model.try_lock_package(pkg).await else {
            return true;
        };
        match fresh_at_head(model, pkg, &locked, &lineage).await {
            Ok(Some(tree)) => sweepable(tree.status.upstream_state),
            _ => true,
        }
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
    let Some(Head {
        lineage,
        user_meta,
        workflow,
        verdict,
    }) = read_head(model, data_dir, settings, pkg).await?
    else {
        return Ok(());
    };
    let user_meta = &user_meta;
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
    let Some(tree) = fresh_at_head(model, pkg, &locked, &lineage).await? else {
        return Ok(());
    };
    let workflow = carried_workflow(workflow.as_ref());
    if verdict == Verdict::Interrupted {
        return sweep(model, &locked, pkg, workflow, &lineage, user_meta, tree).await;
    }
    // A claim or `unrunnable` changes only the metadata. With a pending commit,
    // publishing pushes that commit and drops the new metadata.
    let Tree { status, commit } = tree;
    if status.upstream_state != UpstreamState::UpToDate
        || !status.changes.is_empty()
        || commit.is_some()
    {
        return Ok(());
    }

    let (meta, message) = match verdict {
        Verdict::LeaveAlone | Verdict::Interrupted => return Ok(()),
        Verdict::Unrunnable { reason } => (
            agent_exec::unrunnable(user_meta, &reason),
            format!("Task cannot run: {reason}"),
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
                reset_if_clean(model, pkg, &locked).await?;
                return Ok(());
            }
            (tasks.spawn_run)(Run {
                namespace: pkg.namespace.clone(),
                home,
                document,
                agent: settings.agents[&agent].clone(),
                agent_id: agent,
                claim,
                claimed_meta,
                claim_hash: push.manifest_uri.hash.clone(),
                workflow,
                _hold: hold,
            });
            return Ok(());
        }
    };
    let outcome = publish(model, &locked, pkg, workflow, meta, message, status).await?;
    if !outcome.push().certified_latest {
        reset_if_clean(model, pkg, &locked).await?;
    }
    Ok(())
}

/// After a metadata-only publish that didn't become `latest`: back to `latest`
/// while nothing would be lost (a clean tree, no later commit), else Diverged.
async fn reset_if_clean<M: QuiltModel>(
    model: &M,
    pkg: &quilt::InstalledPackage,
    locked: &M::Locked,
) -> Result<(), Error> {
    let (tree, _) = walk(model, pkg, locked).await?;
    if tree.status.changes.is_empty() && tree.commit.is_none() {
        model.locked_package_reset_local(locked).await?;
    } else {
        warn!(
            "tasks: {}: left Diverged, the tree changed after the publish",
            pkg.namespace
        );
    }
    Ok(())
}

/// A publish other than a metadata-only one is never reset when it doesn't
/// become `latest`: the package stays Diverged for the user.
fn note_not_latest(pkg: &Namespace, push: &quilt::PushOutcome) {
    if !push.certified_latest {
        warn!("tasks: {pkg}: what it pushed is not latest; the package is left Diverged");
    }
}

/// The tree walked under the package's lock, and the commit pending on it.
struct Tree {
    status: quilt::lineage::InstalledPackageStatus,
    commit: Option<quilt::lineage::CommitState>,
}

async fn walk<M: QuiltModel>(
    model: &M,
    pkg: &quilt::InstalledPackage,
    locked: &M::Locked,
) -> Result<(Tree, quilt::lineage::PackageLineage), Error> {
    let status = model.locked_package_status(locked, None).await?;
    let lineage = model.get_installed_package_lineage(pkg).await?;
    let tree = Tree {
        status,
        commit: lineage.commit.clone(),
    };
    Ok((tree, lineage))
}

/// `None` unless `head` is still the package's revision and `latest` was read
/// just now.
async fn fresh_at_head<M: QuiltModel>(
    model: &M,
    pkg: &quilt::InstalledPackage,
    locked: &M::Locked,
    head: &quilt::lineage::PackageLineage,
) -> Result<Option<Tree>, Error> {
    let (tree, lineage) = walk(model, pkg, locked).await?;
    let fresh = tree.status.latest_refreshed && lineage.current_hash() == head.current_hash();
    Ok(fresh.then_some(tree))
}

/// This desktop's interrupted claim, on a base at `latest`: published `failed`
/// with whatever the agent left in the tree if `latest` holds that claim too,
/// else reset as a lost claim.
async fn sweep<M: QuiltModel>(
    model: &M,
    locked: &M::Locked,
    pkg: &quilt::InstalledPackage,
    workflow: WorkflowIntent,
    head: &quilt::lineage::PackageLineage,
    user_meta: &Value,
    Tree { status, commit }: Tree,
) -> Result<(), Error> {
    if !sweepable(status.upstream_state) {
        return Ok(());
    }
    let latest_holds_it = head.current_hash() == Some(status.latest_hash.as_str())
        || read_latest(model, head, status.latest_hash.clone())
            .await?
            .header
            .user_meta
            .is_some_and(|latest| agent_exec::holds_same_claim(user_meta, &latest));
    if !latest_holds_it {
        // A claim that never left here is the one commit pending, and
        // changed only the metadata.
        let only_the_claim = commit.is_none_or(|commit| commit.prev_hashes.is_empty());
        if status.changes.is_empty() && only_the_claim {
            model.locked_package_reset_local(locked).await?;
        } else {
            warn!(
                "tasks: {}: the claim was lost; left Diverged",
                pkg.namespace
            );
        }
        return Ok(());
    }
    if commit.is_some() && status.changes.is_empty() {
        // Publishing would drop the new metadata: push the commit as it is,
        // and fail its claim next tick.
        let push = model.locked_package_push(locked).await?;
        note_not_latest(&pkg.namespace, &push);
        return Ok(());
    }
    let meta = agent_exec::interrupted(user_meta);
    let message = "Task failed: interrupted by restart".to_string();
    let outcome = publish(model, locked, pkg, workflow, meta, message, status).await?;
    note_not_latest(&pkg.namespace, outcome.push());
    Ok(())
}

/// Put `document` in the package's folder, which a sparse copy may lack. A
/// regular file already there stays: the walk compares it with the revision.
/// A symlink on the path stops the claim: the walk skips links, and a linked
/// folder leads outside the package.
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
    let outcome = acp::run(&run.agent, &run.home, async || {
        recheck_claim(model, &run).await.unwrap_or_else(|err| {
            Err(acp::RunOutcome::NotStarted {
                reason: format!("could not re-read the package: {err}"),
            })
        })
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

/// Re-read `latest` before the prompt: the document to prompt, or how the run
/// ends unprompted. A later revision that still holds the claim is pulled and
/// its task checked again.
async fn recheck_claim<M: QuiltModel>(
    model: &M,
    run: &Run,
) -> Result<Result<String, acp::RunOutcome>, Error> {
    let installed = installed(model, &run.namespace).await?;
    let status = model.get_installed_package_status(&installed, None).await?;
    if !status.latest_refreshed {
        return Err(Error::General(
            "the remote's latest could not be read".into(),
        ));
    }
    if status.latest_hash == run.claim_hash {
        return Ok(Ok(run.document.clone()));
    }
    let lineage = model.get_installed_package_lineage(&installed).await?;
    let held = |latest: &quilt::manifest::Manifest| {
        latest
            .header
            .user_meta
            .as_ref()
            .is_some_and(|meta| run.claim.is_held_in(meta))
    };
    let latest = read_latest(model, &lineage, status.latest_hash.clone()).await?;
    if !held(&latest) {
        return Ok(Err(acp::RunOutcome::Superseded));
    }
    let locked = model.lock_package(&installed).await?;
    let pulled = model
        .locked_package_pull(&locked, None, lineage.sync_scope)
        .await?
        .manifest_uri
        .hash;
    // The pull resolves `latest` again, and may land a revision later still.
    let latest = if pulled == status.latest_hash {
        latest
    } else {
        read_latest(model, &lineage, pulled).await?
    };
    let Some(meta) = latest.header.user_meta.as_ref().filter(|_| held(&latest)) else {
        return Ok(Err(acp::RunOutcome::Superseded));
    };
    let rechecked = agent_exec::recheck(meta, |path| lists(&latest, path), &run.agent_id);
    if let Ok(document) = &rechecked {
        let pulled = model.get_installed_package_lineage(&installed).await?;
        download(model, &installed, &locked, &pulled, document).await?;
    }
    Ok(rechecked)
}

fn lists(manifest: &quilt::manifest::Manifest, path: &std::path::Path) -> bool {
    manifest.rows.iter().any(|row| row.logical_key == path)
}

/// The manifest of `lineage`'s remote at `hash`.
async fn read_latest<M: QuiltModel>(
    model: &M,
    lineage: &quilt::lineage::PackageLineage,
    hash: String,
) -> Result<quilt::manifest::Manifest, Error> {
    let uri = quilt_uri::ManifestUri {
        hash,
        ..lineage.remote()?.clone()
    };
    model.browse_remote_manifest(&uri).await
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
    let installed = installed(model, &run.namespace).await?;
    let locked = model.lock_package(&installed).await?;
    let Some(claim) = run.claim.outcome(outcome) else {
        // The claim lost `latest` before the prompt: nothing to publish.
        return reset_if_clean(model, &installed, &locked).await;
    };
    let message = format!(
        "Task {} {}",
        run.document,
        claim["status"].as_str().unwrap_or_default()
    );
    let status = model.locked_package_status(&locked, None).await?;
    // A sign-in lost mid-turn is `not_started` too, after the agent wrote files.
    let metadata_only = matches!(
        outcome,
        acp::RunOutcome::NotStarted { .. } | acp::RunOutcome::Unrunnable { .. }
    ) && status.changes.is_empty();
    // The head the run ended on: the claim, or a later revision carrying it.
    let lineage = model.get_installed_package_lineage(&installed).await?;
    let head = model
        .get_installed_package_manifest(&installed, &lineage)
        .await?;
    let (base, workflow) = match head.header.user_meta {
        Some(meta) if run.claim.is_held_in(&meta) => {
            (meta, carried_workflow(head.header.workflow.as_ref()))
        }
        _ => (run.claimed_meta.clone(), run.workflow.clone()),
    };
    let meta = agent_exec::with_claim(&base, claim);
    let published = publish(model, &locked, &installed, workflow, meta, message, status).await?;
    if metadata_only {
        if !published.push().certified_latest {
            reset_if_clean(model, &installed, &locked).await?;
        }
    } else {
        note_not_latest(&run.namespace, published.push());
    }
    Ok(())
}

#[cfg(test)]
mod tests;
