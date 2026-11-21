//! `agent_exec` on the dataspace engine.
//!
//! A task is found from the default dataspace's `post-read` hook, which
//! `dataspace::sync` fires once a read moves a `sync` entry's lock to a
//! newer revision (25.13), and, on start, at each entry's lock base, which is
//! where a run the app's exit cut short is recovered. It runs as the ref
//! engine's does ([`super::check`]): only in a blessed bucket, with tasks on,
//! and with the same verdicts, claim, run and outcome. What differs is how
//! a revision is made: the claim and the outcome are written to the folder's
//! `metadata.toml`, committed and synced, and the run holds the package in
//! the Domain, so the autosync tick leaves its folder alone until it ends.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;

use futures_util::future::BoxFuture;
use quilt_rs::dataspace::contract::Access;
use quilt_rs::dataspace::contract::Entry;
use quilt_rs::dataspace::contract::PackageKey;
use quilt_rs::dataspace::contract::Registry;
use quilt_rs::dataspace::hooks;
use quilt_rs::dataspace::hooks::HookCall;
use quilt_rs::dataspace::hooks::HookPoint;
use quilt_rs::dataspace::hooks::Registration;
use quilt_rs::dataspace::status::EntryStatus;
use quilt_rs::io::remote::Remote;
use quilt_rs::io::remote::RemoteS3;
use quilt_rs::io::storage::Storage;
use quilt_rs::lineage::UpstreamState;
use serde_json::Value;

use super::acp;
use super::acp::RunOutcome;
use super::agent_exec;
use super::agent_exec::Claim;
use super::agent_exec::Verdict;
use super::agent_process;
use super::settings;
use super::settings::AgentSettings;
use super::settings::TaskSettings;
use crate::engine::Error;
use crate::engine::dataspace::Dataspace;
use crate::engine::telemetry::prelude::*;

/// One run of an agent: what it is given.
pub struct AgentRun {
    pub agent: AgentSettings,
    /// The package's working folder, which the agent runs in.
    pub folder: PathBuf,
    pub data_dir: PathBuf,
    pub invocation_id: String,
    /// Re-reads the task once the session is open: the document to prompt,
    /// or how the run ends unprompted.
    pub recheck: BoxFuture<'static, Result<String, RunOutcome>>,
}

/// Runs an agent to its outcome: [`acp_agent`] in the app.
pub type RunAgent = Arc<dyn Fn(AgentRun) -> BoxFuture<'static, RunOutcome> + Send + Sync>;

/// The app's agent: started from its settings and spoken to over ACP, its
/// process recorded so a restart can stop one left running.
pub fn acp_agent() -> RunAgent {
    Arc::new(|run: AgentRun| {
        Box::pin(async move {
            let AgentRun {
                agent,
                folder,
                data_dir,
                invocation_id,
                recheck,
            } = run;
            let started = async |pid| {
                let spawned = agent_process::Spawned::of(pid);
                if let Err(err) = settings::record_agent(&data_dir, &invocation_id, spawned).await {
                    warn!("tasks: cannot record the agent: {err}");
                }
            };
            let outcome = acp::run(&agent, &folder, started, async move || recheck.await).await;
            if let Err(err) = settings::record_agent(&data_dir, &invocation_id, None).await {
                warn!("tasks: cannot clear the agent's record: {err}");
            }
            outcome
        })
    })
}

/// The task runner of one dataspace engine.
pub struct DataspaceTasks<R: Remote = RemoteS3> {
    ds: Arc<Dataspace<R>>,
    data_dir: PathBuf,
    run_agent: RunAgent,
    /// The packages a run holds: no other check acts on them.
    in_flight: Arc<Mutex<BTreeSet<PackageKey>>>,
    runs: Mutex<Vec<tokio::task::JoinHandle<()>>>,
}

/// Lets go of a package a run held.
struct Hold {
    key: PackageKey,
    in_flight: Arc<Mutex<BTreeSet<PackageKey>>>,
}

impl Drop for Hold {
    fn drop(&mut self) {
        locked(&self.in_flight).remove(&self.key);
    }
}

fn locked<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn fail(err: impl std::fmt::Display) -> Error {
    Error::Dataspace(err.to_string())
}

/// What a revision asks of this desktop.
struct Found {
    settings: TaskSettings,
    user_meta: Value,
    verdict: Verdict,
}

/// A claimed task, for its run to take.
pub struct Claimed {
    entry: Entry,
    claim: Claim,
    claim_hash: String,
    document: String,
    agent_id: String,
    agent: AgentSettings,
    /// The package, held from autosync's checks in this process.
    hold: Hold,
    /// The package's lock in the Domain, which keeps the autosync tick out
    /// of the folder until the agent ends.
    guard: quilt_rs::io::storage::LockGuard,
}

/// A remote the runner's futures can be handed to tasks of their own with:
/// a generic one's futures are not known to be `Send`.
pub trait Spawn: Remote + Send + Sync + Sized + 'static {
    /// Runs [`DataspaceTasks::check`] on a task of its own.
    fn spawn_check(
        runtime: &tokio::runtime::Handle,
        tasks: Arc<DataspaceTasks<Self>>,
        entry: Entry,
        revision: String,
    );

    /// Runs a claimed task on a task of its own.
    fn spawn_run(tasks: Arc<DataspaceTasks<Self>>, claimed: Claimed)
    -> tokio::task::JoinHandle<()>;

    /// The run's re-read of its task, boxed for the agent.
    fn boxed_recheck(
        tasks: Arc<DataspaceTasks<Self>>,
        entry: Entry,
        claim: Claim,
        document: String,
        agent_id: String,
        claim_hash: String,
    ) -> BoxFuture<'static, Result<String, RunOutcome>>;
}

macro_rules! spawns {
    ($remote:ty) => {
        impl Spawn for $remote {
            fn spawn_check(
                runtime: &tokio::runtime::Handle,
                tasks: Arc<DataspaceTasks<Self>>,
                entry: Entry,
                revision: String,
            ) {
                runtime.spawn(async move { Box::pin(tasks.check(&entry, &revision)).await });
            }

            fn spawn_run(
                tasks: Arc<DataspaceTasks<Self>>,
                claimed: Claimed,
            ) -> tokio::task::JoinHandle<()> {
                tokio::spawn(async move { Box::pin(tasks.run(claimed)).await })
            }

            fn boxed_recheck(
                tasks: Arc<DataspaceTasks<Self>>,
                entry: Entry,
                claim: Claim,
                document: String,
                agent_id: String,
                claim_hash: String,
            ) -> BoxFuture<'static, Result<String, RunOutcome>> {
                Box::pin(tasks.recheck(entry, claim, document, agent_id, claim_hash))
            }
        }
    };
}

spawns!(RemoteS3);
#[cfg(test)]
spawns!(quilt_rs::io::remote::mocks::MockRemote);

impl<R: Spawn> DataspaceTasks<R> {
    pub fn new(ds: Arc<Dataspace<R>>, data_dir: PathBuf, run_agent: RunAgent) -> Arc<Self> {
        Arc::new(Self {
            ds,
            data_dir,
            run_agent,
            in_flight: Arc::default(),
            runs: Mutex::default(),
        })
    }

    /// Checks each revision a read brings to the default dataspace, from its
    /// `post-read` hook, until the registration drops. Each check runs on a
    /// task of its own, once the sync that read it has ended.
    pub fn register(self: &Arc<Self>) -> Registration {
        let runtime = tokio::runtime::Handle::current();
        let tasks = Arc::clone(self);
        hooks::register(
            &self.ds.dataspace(),
            HookPoint::PostRead,
            Arc::new(move |call: &HookCall| {
                R::spawn_check(
                    &runtime,
                    Arc::clone(&tasks),
                    call.entry.clone(),
                    call.revision.clone(),
                );
            }),
        )
    }

    /// Checks each `sync` entry at its lock's base, as the app starts: a run
    /// the app's exit cut short is recovered, and a task a check left
    /// waiting is looked at again.
    pub async fn scan(self: &Arc<Self>) {
        let contract = match self.ds.contract().await {
            Ok(contract) => contract,
            Err(err) => {
                warn!("tasks: cannot read the dataspace: {err}");
                return;
            }
        };
        for entry in contract.entries() {
            if entry.access != Access::Sync {
                continue;
            }
            let base = match self.ds.entry_status(&entry.package, false).await {
                Ok(status) => status.hashes.base,
                Err(err) => {
                    warn!("tasks: {}: {err}", entry.key());
                    continue;
                }
            };
            if let Some(base) = base {
                Box::pin(self.check(entry, &base)).await;
            }
        }
    }

    /// Stops every run, as the app's exit does: what each left is recovered
    /// on the next start.
    pub fn shutdown(&self) {
        for run in locked(&self.runs).drain(..) {
            run.abort();
        }
    }

    fn holds(&self, key: &PackageKey) -> bool {
        locked(&self.in_flight).contains(key)
    }

    fn hold(&self, key: PackageKey) -> Hold {
        locked(&self.in_flight).insert(key.clone());
        Hold {
            key,
            in_flight: Arc::clone(&self.in_flight),
        }
    }

    /// Acts on what `revision`, just read into `entry`, asks of this desktop.
    pub async fn check(self: &Arc<Self>, entry: &Entry, revision: &str) {
        if entry.access != Access::Sync || self.holds(&entry.key()) {
            return;
        }
        let Some(Found {
            settings,
            user_meta,
            verdict,
        }) = self.find(entry, revision).await
        else {
            return;
        };
        let checked = match verdict {
            Verdict::LeaveAlone => Ok(()),
            Verdict::Claim { document, agent } => {
                let agent = (agent.clone(), settings.agents[&agent].clone());
                Box::pin(self.claim(entry, revision, &user_meta, document, agent)).await
            }
            Verdict::Unrunnable { reason } => {
                Box::pin(self.unrunnable(entry, revision, &user_meta, &reason)).await
            }
            Verdict::Interrupted => Box::pin(self.recover(entry, revision, &user_meta)).await,
        };
        if let Err(err) = checked {
            warn!("tasks: {}: {err}", entry.key());
        }
    }

    /// What `revision` asks: `None` with tasks off, outside a blessed bucket,
    /// or with no `agent_exec`.
    async fn find(&self, entry: &Entry, revision: &str) -> Option<Found> {
        let settings = match TaskSettings::load(&self.data_dir).await {
            Ok(settings) => settings?,
            Err(err) => {
                warn!("tasks: cannot read the task settings, tasks are off: {err}");
                return None;
            }
        };
        let Registry::S3(bucket) = &entry.registry else {
            return None;
        };
        if !settings.blesses(bucket) {
            return None;
        }
        let manifest = self
            .ds
            .revision_manifest(entry, revision)
            .await
            .inspect_err(|err| warn!("tasks: {}: {err}", entry.key()))
            .ok()?;
        let user_meta = manifest.header.user_meta.clone()?;
        user_meta.get("agent_exec")?;
        let own = settings::own_invocations(&self.data_dir)
            .await
            .inspect_err(|err| warn!("tasks: {}: {err}", entry.key()))
            .ok()?;
        let verdict = agent_exec::classify(
            &user_meta,
            |path| super::lists(&manifest, path),
            &settings.agents,
            &own,
        );
        Some(Found {
            settings,
            user_meta,
            verdict,
        })
    }

    /// The entry's status with `latest` read just now: `None`, logged,
    /// unless the folder holds `revision` with nothing changed or unpushed
    /// and `latest` names it. Only then may a revision changing only the
    /// metadata be published over it.
    async fn clean_at(&self, entry: &Entry, revision: &str) -> Result<Option<EntryStatus>, Error> {
        let status = self.ds.entry_status(&entry.package, true).await?;
        let clean = status.hashes.base.as_deref() == Some(revision)
            && status.hashes.latest.as_deref() == Some(revision)
            && status.upstream_state == UpstreamState::UpToDate
            && status.changes.is_empty()
            && !status.metadata_changed
            && status.unpushed.is_empty()
            && status.compared;
        if !clean {
            info!(
                "tasks: {}: waits for a clean folder at latest, checked again on the next read or start",
                entry.key()
            );
        }
        Ok(clean.then_some(status))
    }

    /// Writes `user_meta` to the folder's metadata file, and commits and
    /// syncs it as `message`.
    async fn publish(&self, entry: &Entry, user_meta: &Value, message: &str) -> Result<(), Error> {
        quilt_rs::dataspace::set_package_metadata(self.ds.storage(), &entry.folder, user_meta)
            .await
            .map_err(fail)?;
        Box::pin(self.ds.commit_and_sync(&entry.package, message)).await?;
        Ok(())
    }

    /// After a revision that changes only the metadata: whether the lock's
    /// base is `latest` and its metadata passes `holds_it`. Where not, and the
    /// commit is all the folder holds, back to `latest`; else left, Diverged
    /// or blocked, for the user.
    async fn landed(
        &self,
        entry: &Entry,
        holds_it: impl Fn(&Value) -> bool,
    ) -> Result<Option<String>, Error> {
        let status = self.ds.entry_status(&entry.package, true).await?;
        if let Some(base) = status.hashes.base.clone()
            && status.unpushed.is_empty()
            && status.hashes.latest.as_ref() == Some(&base)
        {
            let manifest = self.ds.revision_manifest(entry, &base).await?;
            if manifest.header.user_meta.as_ref().is_some_and(&holds_it) {
                return Ok(Some(base));
            }
        }
        if status.changes.is_empty() && status.unpushed.len() <= 1 {
            let _writes = self.ds.hold_writes().await;
            Box::pin(self.ds.reset(&entry.package)).await?;
        } else {
            warn!(
                "tasks: {}: the revision did not become latest; left for the user",
                entry.key()
            );
        }
        Ok(None)
    }

    /// A task this desktop can run: claimed by a revision that changes only
    /// the metadata, then run in the folder, which the run holds in the
    /// Domain, and its outcome published.
    async fn claim(
        self: &Arc<Self>,
        entry: &Entry,
        revision: &str,
        user_meta: &Value,
        document: String,
        (agent_id, agent): (String, AgentSettings),
    ) -> Result<(), Error> {
        if self.clean_at(entry, revision).await?.is_none() {
            return Ok(());
        }
        // The document must be on disk: a sparse entry can lack it.
        let on_disk = tokio::fs::symlink_metadata(entry.folder.join(&document)).await;
        if !on_disk.is_ok_and(|meta| meta.is_file()) {
            warn!(
                "tasks: {}: {document} is not a file in the folder; keep it to run the task",
                entry.key()
            );
            return Ok(());
        }
        let hold = self.hold(entry.key());
        let claim = Claim {
            invocation_id: uuid::Uuid::new_v4().to_string(),
            claimed_at: agent_exec::claim_time(jiff::Timestamp::now()),
        };
        // Before the push, so a crash after it still knows the claim as ours.
        settings::remember_invocation(&self.data_dir, &claim.invocation_id).await?;
        let claimed = agent_exec::with_claim(user_meta, claim.claimed());
        self.publish(entry, &claimed, &format!("Task {document} claimed"))
            .await?;
        let Some(claim_hash) = self.landed(entry, |meta| claim.is_held_in(meta)).await? else {
            info!(
                "tasks: {}: another revision became latest first, not running",
                entry.key()
            );
            return Ok(());
        };
        let Registry::S3(bucket) = &entry.registry else {
            return Ok(());
        };
        let package_lock = self
            .ds
            .domain()
            .registry(bucket)
            .map_err(fail)?
            .package_lock(&entry.package);
        let guard = self
            .ds
            .storage()
            .lock_exclusive(package_lock)
            .await
            .map_err(fail)?;
        let claimed = Claimed {
            entry: entry.clone(),
            claim,
            claim_hash,
            document,
            agent_id,
            agent,
            hold,
            guard,
        };
        let run = R::spawn_run(Arc::clone(self), claimed);
        locked(&self.runs).push(run);
        Ok(())
    }

    /// Runs the agent on a claimed task, then lets the package go and
    /// publishes the outcome.
    async fn run(self: Arc<Self>, claimed: Claimed) {
        let Claimed {
            entry,
            claim,
            claim_hash,
            document,
            agent_id,
            agent,
            hold,
            guard,
        } = claimed;
        let _hold = hold;
        let recheck = R::boxed_recheck(
            Arc::clone(&self),
            entry.clone(),
            claim.clone(),
            document.clone(),
            agent_id,
            claim_hash,
        );
        let outcome = (self.run_agent)(AgentRun {
            agent,
            folder: entry.folder.clone(),
            data_dir: self.data_dir.clone(),
            invocation_id: claim.invocation_id.clone(),
            recheck,
        })
        .await;
        info!("tasks: {}: {outcome:?}", entry.key());
        drop(guard);
        if let Err(err) = Box::pin(self.finish(&entry, &claim, &document, &outcome)).await {
            warn!(
                "tasks: {}: publishing the outcome failed: {err}",
                entry.key()
            );
        }
    }

    /// Re-reads `latest` before the prompt. The claim still `latest` runs as
    /// it is; a later revision that still holds it is checked again and run
    /// in the folder as it stands, which the run holds; any other ends the
    /// run unprompted.
    async fn recheck(
        self: Arc<Self>,
        entry: Entry,
        claim: Claim,
        document: String,
        agent_id: String,
        claim_hash: String,
    ) -> Result<String, RunOutcome> {
        let not_started = |reason: String| RunOutcome::NotStarted {
            reason: format!("could not re-read the package: {reason}"),
        };
        let status = self
            .ds
            .entry_status(&entry.package, true)
            .await
            .map_err(|err| not_started(err.to_string()))?;
        let Some(latest) = status.hashes.latest else {
            return Err(not_started("its latest could not be read".to_string()));
        };
        if latest == claim_hash {
            return Ok(document);
        }
        let manifest = self
            .ds
            .revision_manifest(&entry, &latest)
            .await
            .map_err(|err| not_started(err.to_string()))?;
        let Some(meta) = manifest
            .header
            .user_meta
            .as_ref()
            .filter(|meta| claim.is_held_in(meta))
        else {
            return Err(RunOutcome::Superseded);
        };
        agent_exec::recheck(meta, |path| super::lists(&manifest, path), &agent_id)
    }

    /// The run's outcome, saved first, then published where the head still
    /// holds the claim. A run superseded before its prompt publishes nothing:
    /// the next sync reads `latest`.
    async fn finish(
        &self,
        entry: &Entry,
        claim: &Claim,
        document: &str,
        outcome: &RunOutcome,
    ) -> Result<(), Error> {
        let Some(out) = claim.outcome(outcome) else {
            return Ok(());
        };
        if let Err(err) =
            settings::remember_outcome(&self.data_dir, &claim.invocation_id, &out).await
        {
            warn!("tasks: {}: cannot save the outcome: {err}", entry.key());
        }
        let message = super::outcome_message(document, &out);
        self.publish_outcome(entry, out, message, |meta| claim.is_held_in(meta))
            .await
    }

    /// Publishes the folder, the run's work, with the claim replaced by
    /// `out`, while the head, the lock's last commit else its base, holds
    /// the claim; a head that no longer does wins, and the folder is left to
    /// autosync. Where `latest` moved, the sync merges the outcome onto it,
    /// or blocks on what collides, as for any change.
    async fn publish_outcome(
        &self,
        entry: &Entry,
        mut out: Value,
        mut message: String,
        holds_it: impl Fn(&Value) -> bool,
    ) -> Result<(), Error> {
        let status = self.ds.entry_status(&entry.package, false).await?;
        let Some(current) = status.hashes.current.clone() else {
            return Ok(());
        };
        let manifest = self.ds.revision_manifest(entry, &current).await?;
        let Some(base_meta) = manifest.header.user_meta.filter(|meta| holds_it(meta)) else {
            info!(
                "tasks: {}: the head no longer holds the claim; no outcome",
                entry.key()
            );
            return Ok(());
        };
        let counted =
            matches!(out["status"].as_str(), Some("done" | "failed")) && status.unpushed.is_empty();
        if counted {
            out["changes"] = super::count_changes(&status.changes);
            if status.changes.is_empty() {
                message.push_str(", no changes");
            }
        }
        let meta = agent_exec::with_claim(&base_meta, out);
        self.publish(entry, &meta, &message).await
    }

    /// A task that can't run as written: its claim says why, in a revision
    /// that changes only the metadata.
    async fn unrunnable(
        &self,
        entry: &Entry,
        revision: &str,
        user_meta: &Value,
        reason: &str,
    ) -> Result<(), Error> {
        if self.clean_at(entry, revision).await?.is_none() {
            return Ok(());
        }
        let meta = agent_exec::unrunnable(user_meta, reason);
        self.publish(entry, &meta, &format!("Task cannot run: {reason}"))
            .await?;
        let claim = meta["agent_exec"]["claim"].clone();
        self.landed(entry, |latest| latest["agent_exec"]["claim"] == claim)
            .await?;
        Ok(())
    }

    /// This desktop's own claim with no run alive, as after the app's exit:
    /// the agent left running is stopped, and, while `latest` holds the
    /// claim, the folder is published with the outcome the run saved, or
    /// `failed` where it saved none.
    async fn recover(&self, entry: &Entry, revision: &str, user_meta: &Value) -> Result<(), Error> {
        let namespace = &entry.package;
        if !super::stop_leftover_agent(&self.data_dir, user_meta, namespace).await {
            return Ok(());
        }
        let status = self.ds.entry_status(namespace, true).await?;
        if !matches!(
            status.upstream_state,
            UpstreamState::UpToDate | UpstreamState::Ahead
        ) {
            return Ok(());
        }
        let latest_holds_it = match status.hashes.latest.as_deref() {
            Some(latest) if latest == revision => true,
            Some(latest) => self
                .ds
                .revision_manifest(entry, latest)
                .await?
                .header
                .user_meta
                .is_some_and(|latest| agent_exec::holds_same_claim(user_meta, &latest)),
            None => false,
        };
        if !latest_holds_it {
            warn!(
                "tasks: {}: the claim was lost; the folder is left to autosync",
                entry.key()
            );
            return Ok(());
        }
        let invocation_id = user_meta["agent_exec"]["claim"]["invocation_id"]
            .as_str()
            .unwrap_or_default();
        let saved = settings::remembered_outcome(&self.data_dir, invocation_id)
            .await
            .unwrap_or_else(|err| {
                warn!(
                    "tasks: {}: cannot read the saved outcome: {err}",
                    entry.key()
                );
                None
            });
        let (out, message) = match saved {
            Some(out) => {
                let document = user_meta["agent_exec"]["document"]
                    .as_str()
                    .unwrap_or_default();
                let message = super::outcome_message(document, &out);
                (out, message)
            }
            None => (
                agent_exec::interrupted(user_meta),
                "Task failed: interrupted by restart".to_string(),
            ),
        };
        self.publish_outcome(entry, out, message, |meta| {
            agent_exec::holds_same_claim(user_meta, meta)
        })
        .await
    }
}

#[cfg(test)]
mod tests;
