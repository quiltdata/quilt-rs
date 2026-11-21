//! `agent_exec` on the dataspace engine: dispatched from the default
//! dataspace's `post-read` hook, which `dataspace::sync` fires once a read
//! moves a package's lock to a newer revision (25.13), instead of from the
//! ref engine's autopull tick.
//!
//! The dispatch reads the revision's `agent_exec` as [`super::check`] reads
//! the head's: only in a blessed bucket, and only with tasks on. What it finds
//! is handed to the caller's [`OnTask`]. The app's logs it: claiming, running
//! and recovering a task on the dataspace engine are not built, so a task is
//! run on the ref engine, with the dataspace engine opted out of.

use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use quilt_rs::dataspace::contract::Registry;
use quilt_rs::dataspace::hooks;
use quilt_rs::dataspace::hooks::HookCall;
use quilt_rs::dataspace::hooks::HookPoint;
use quilt_rs::dataspace::hooks::Registration;
use quilt_rs::io::remote::Remote;

use super::agent_exec;
use super::agent_exec::Verdict;
use super::settings;
use super::settings::TaskSettings;
use crate::engine::dataspace::Dataspace;
use crate::engine::telemetry::prelude::*;

/// What the dispatch does with a revision that asks for a task.
pub type OnTask = Arc<dyn Fn(&HookCall, Verdict) + Send + Sync>;

/// Dispatches `agent_exec` from the `post-read` hook of `ds`'s default
/// dataspace, until the returned registration drops. Each call is handed to
/// a task of its own on the current runtime, so the sync it follows goes on.
pub fn register<R: Remote + Send + Sync + 'static>(
    ds: &Arc<Dataspace<R>>,
    data_dir: PathBuf,
    on_task: OnTask,
) -> Registration {
    let runtime = tokio::runtime::Handle::current();
    let ds_for_hook = Arc::clone(ds);
    hooks::register(
        &ds.dataspace(),
        HookPoint::PostRead,
        Arc::new(move |call: &HookCall| {
            let ds = Arc::clone(&ds_for_hook);
            let data_dir = data_dir.clone();
            let on_task = Arc::clone(&on_task);
            let call = call.clone();
            runtime.spawn(async move {
                if let Some(verdict) = dispatch(&ds, &data_dir, &call).await {
                    on_task(&call, verdict);
                }
            });
        }),
    )
}

/// The app's [`OnTask`]: a revision that asks for a task is logged, as the
/// dataspace engine does not run one yet.
#[allow(
    clippy::needless_pass_by_value,
    reason = "an `OnTask`, which owns the verdict so a test can keep it"
)]
pub fn log_task(call: &HookCall, verdict: Verdict) {
    warn!(
        "tasks: {}: revision {} asks for a task ({verdict:?}); the dataspace engine does not run tasks yet, so opt out of it (\"dataspace_opt_out\": true in experimental_settings.json) to run it",
        call.entry.key(),
        call.revision
    );
}

/// What the revision `call` read asks for: `None` with tasks off, outside a
/// blessed bucket, with no `agent_exec`, or where it asks nothing of this
/// desktop.
pub async fn dispatch<R: Remote + Sync>(
    ds: &Dataspace<R>,
    data_dir: &Path,
    call: &HookCall,
) -> Option<Verdict> {
    let settings = match TaskSettings::load(data_dir).await {
        Ok(Some(settings)) => settings,
        Ok(None) => return None,
        Err(err) => {
            warn!("tasks: cannot read the task settings, tasks are off: {err}");
            return None;
        }
    };
    let Registry::S3(bucket) = &call.entry.registry else {
        return None;
    };
    if !settings.blesses(bucket) {
        return None;
    }
    let manifest = match ds.revision_manifest(&call.entry, &call.revision).await {
        Ok(manifest) => manifest,
        Err(err) => {
            warn!("tasks: {}: {err}", call.entry.key());
            return None;
        }
    };
    let user_meta = manifest.header.user_meta.as_ref()?;
    user_meta.get("agent_exec")?;
    let own = match settings::own_invocations(data_dir).await {
        Ok(own) => own,
        Err(err) => {
            warn!("tasks: {}: {err}", call.entry.key());
            return None;
        }
    };
    let verdict = agent_exec::classify(
        user_meta,
        |path| super::lists(&manifest, path),
        &settings.agents,
        &own,
    );
    (verdict != Verdict::LeaveAlone).then_some(verdict)
}

#[cfg(test)]
mod tests;
