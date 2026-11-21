//! A dataspace's hooks: one script for each of four points around reading and
//! writing, declared in `data.toml` under `[hooks.<point>]` and run as the
//! user who runs the flow.
//!
//! A failing `pre-` script stops its entry; a failing `post-` script is only
//! logged, since what it follows is done.

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;
use std::path::PathBuf;
use std::process::ExitStatus;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

use serde::Deserialize;
use thiserror::Error;
use tracing::warn;

use crate::dataspace::contract::Contract;
use crate::dataspace::contract::Entry;

/// Where in a flow a hook runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum HookPoint {
    PreRead,
    PostRead,
    PreWrite,
    PostWrite,
}

/// The point's key under `[hooks]`.
impl fmt::Display for HookPoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::PreRead => "pre-read",
            Self::PostRead => "post-read",
            Self::PreWrite => "pre-write",
            Self::PostWrite => "post-write",
        })
    }
}

/// One script: a program, found on `PATH` where it is a bare name, run from
/// the dataspace's folder with `args`, and with `env` added to the
/// environment the flow runs in.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hook {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

/// The `[hooks]` table: at most one script for each point. Any other key is
/// refused; there is no hook for `sync` as a whole.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hooks {
    #[serde(rename = "pre-read")]
    pre_read: Option<Hook>,
    #[serde(rename = "post-read")]
    post_read: Option<Hook>,
    #[serde(rename = "pre-write")]
    pre_write: Option<Hook>,
    #[serde(rename = "post-write")]
    post_write: Option<Hook>,
}

impl Hooks {
    /// The script declared at `point`.
    #[must_use]
    pub fn get(&self, point: HookPoint) -> Option<&Hook> {
        match point {
            HookPoint::PreRead => self.pre_read.as_ref(),
            HookPoint::PostRead => self.post_read.as_ref(),
            HookPoint::PreWrite => self.pre_write.as_ref(),
            HookPoint::PostWrite => self.post_write.as_ref(),
        }
    }
}

/// Why a hook stopped its entry.
#[derive(Debug, Error)]
#[error("the {point} hook {command:?} {failure}")]
pub struct HookError {
    pub point: HookPoint,
    pub command: String,
    pub failure: HookFailure,
}

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum HookFailure {
    #[error("could not start: {0}")]
    Start(std::io::Error),

    #[error("failed: {0}")]
    Exit(ExitStatus),
}

/// The point the hook runs at: `pre-read`, `post-read`, `pre-write` or
/// `post-write`.
pub const HOOK_VAR: &str = "QUILT_HOOK";
/// The dataspace's folder, which the hook also runs in.
pub const DATASPACE_VAR: &str = "QUILT_DATASPACE";
/// The entry's package, as its canonical URI.
pub const PACKAGE_VAR: &str = "QUILT_PACKAGE";
/// The entry's working folder.
pub const FOLDER_VAR: &str = "QUILT_FOLDER";
/// After a read or a write, the revision read or stored, as a whole top hash.
pub const REVISION_VAR: &str = "QUILT_REVISION";

/// Runs the script `contract` declares at `point`, if any, for `entry`, and
/// waits for it to end. `revision` is the one just read or stored, after a
/// read or a write. Its output goes to the flow's standard error, so it
/// never mixes with what the flow prints.
pub(crate) async fn run(
    contract: &Contract,
    point: HookPoint,
    entry: &Entry,
    revision: Option<&str>,
) -> Result<(), HookError> {
    let Some(hook) = contract.hooks().get(point) else {
        return Ok(());
    };
    let failed = |failure| HookError {
        point,
        command: hook.command.clone(),
        failure,
    };
    let mut command = tokio::process::Command::new(&hook.command);
    command
        .args(&hook.args)
        .envs(&hook.env)
        .env(HOOK_VAR, point.to_string())
        .env(DATASPACE_VAR, contract.dataspace())
        .env(PACKAGE_VAR, entry.key().to_string())
        .env(FOLDER_VAR, &entry.folder)
        .env_remove(REVISION_VAR)
        .current_dir(contract.dataspace())
        .stdin(Stdio::null())
        .stdout(std::io::stderr())
        .stderr(std::io::stderr());
    if let Some(revision) = revision {
        command.env(REVISION_VAR, revision);
    }
    let status = command
        .status()
        .await
        .map_err(|err| failed(HookFailure::Start(err)))?;
    if status.success() {
        Ok(())
    } else {
        Err(failed(HookFailure::Exit(status)))
    }
}

/// [`run`] for a point after a read or write, which is done whatever the
/// script does: its failure is logged. Then each application hook
/// [`register`]ed for the dataspace at `point` is called.
pub(crate) async fn after(contract: &Contract, point: HookPoint, entry: &Entry, revision: &str) {
    if let Err(err) = run(contract, point, entry, Some(revision)).await {
        warn!("{}: {err}", entry.key());
    }
    call_registered(&HookCall {
        point,
        dataspace: contract.dataspace().to_path_buf(),
        entry: entry.clone(),
        revision: revision.to_string(),
    });
}

/// What an application hook is called with: the point, the dataspace, the
/// entry and the revision just read or stored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HookCall {
    pub point: HookPoint,
    pub dataspace: PathBuf,
    pub entry: Entry,
    pub revision: String,
}

/// An application's own hook, called in its process at a point after a read
/// or a write, as the dataspace's script is: 25.13's application-wide hook,
/// an application preference rather than a line of `data.toml`.
///
/// It is called on the flow's task and must return at once, handing any
/// work to a task of its own. It can't stop what it follows.
pub type AppHook = Arc<dyn Fn(&HookCall) + Send + Sync>;

struct Registered {
    id: u64,
    dataspace: PathBuf,
    point: HookPoint,
    hook: AppHook,
}

static REGISTERED: Mutex<Vec<Registered>> = Mutex::new(Vec::new());
static NEXT_ID: AtomicU64 = AtomicU64::new(0);

fn registered() -> std::sync::MutexGuard<'static, Vec<Registered>> {
    REGISTERED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Calls `hook` at `point` in the dataspace at `dataspace`, in this process,
/// until the returned guard is dropped.
#[must_use = "the hook is removed when the guard drops"]
pub fn register(dataspace: &Path, point: HookPoint, hook: AppHook) -> Registration {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    registered().push(Registered {
        id,
        dataspace: std::path::absolute(dataspace).unwrap_or_else(|_| dataspace.to_path_buf()),
        point,
        hook,
    });
    Registration { id }
}

/// Removes its hook when dropped.
#[derive(Debug)]
pub struct Registration {
    id: u64,
}

impl Drop for Registration {
    fn drop(&mut self) {
        registered().retain(|registered| registered.id != self.id);
    }
}

fn call_registered(call: &HookCall) {
    let dataspace = std::path::absolute(&call.dataspace).unwrap_or_else(|_| call.dataspace.clone());
    // Called outside the lock, so a hook may register or drop one.
    let hooks: Vec<AppHook> = registered()
        .iter()
        .filter(|registered| registered.point == call.point && registered.dataspace == dataspace)
        .map(|registered| Arc::clone(&registered.hook))
        .collect();
    for hook in hooks {
        hook(call);
    }
}

/// Whether a read that leaves the lock at `read`, where it was at `before`,
/// took a newer revision, which `post-read` follows.
pub(crate) fn newer<'a>(before: Option<&str>, read: Option<&'a str>) -> Option<&'a str> {
    read.filter(|read| before != Some(*read))
}

#[cfg(test)]
pub(crate) mod tests;
