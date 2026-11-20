//! A dataspace's hooks: one script for each of four points around reading and
//! writing, declared in `data.toml` under `[hooks.<point>]` and run as the
//! user who runs the flow.
//!
//! `pre-read` runs before a flow reads an entry, and `pre-write` before it
//! publishes one, so a `pre-write` script may rewrite `metadata.toml` first;
//! either one exiting with a failure stops that entry. `post-read` runs once
//! a read moved the lock to a newer revision, even one whose files are all
//! unchanged, and `post-write` once a publish stored a revision; their
//! failure is logged, since what they follow is done.

use std::collections::BTreeMap;
use std::fmt;
use std::process::ExitStatus;
use std::process::Stdio;

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
/// script does: its failure is logged.
pub(crate) async fn after(contract: &Contract, point: HookPoint, entry: &Entry, revision: &str) {
    if let Err(err) = run(contract, point, entry, Some(revision)).await {
        warn!("{}: {err}", entry.key());
    }
}

/// Whether a read that leaves the lock at `read`, where it was at `before`,
/// took a newer revision, which `post-read` follows.
pub(crate) fn newer<'a>(before: Option<&str>, read: Option<&'a str>) -> Option<&'a str> {
    read.filter(|read| before != Some(*read))
}

#[cfg(test)]
pub(crate) mod tests;
