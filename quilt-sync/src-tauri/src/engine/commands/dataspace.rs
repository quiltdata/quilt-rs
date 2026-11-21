//! The commands' bodies on the dataspace engine, which a command runs in
//! place of its ref-path body where [`Services::dataspace`] is set.
//!
//! Each answers what its command answers, with the same lines, so the pages
//! need not know which engine ran. They send no telemetry events and touch
//! none of the ref engine's watcher state: the dataspace engine has its own
//! runner, and its status reads `conflict.json` and the lock, not a pause.
//!
//! [`Services::dataspace`]: crate::engine::services::Services::dataspace

use std::path::PathBuf;

use quilt_rs::dataspace::contract::Access;
use quilt_rs::dataspace::contract::Entry;
use quilt_rs::dataspace::contract::Registry;
use quilt_rs::dataspace::create::Create;
use quilt_rs::dataspace::install::Install;
use quilt_rs::dataspace::keep::Keep;
use quilt_rs::dataspace::keep::Scope;
use quilt_rs::dataspace::publish::Published;
use quilt_rs::dataspace::sync::EntrySync;
use quilt_rs::io::remote::Remote;
use quilt_uri::Namespace;

use super::FreedSpace;
use super::KeepingScope;
use super::MainPagePackage;
use super::MainPagePackageRefresh;
use super::MainPagePackages;
use super::PackageStateDto;
use super::RemotePackageResult;
use super::RevisionHistoryData;
use super::RevisionHistoryRow;
use super::StorageSize;
use super::package_ops::CommitResponse;
use serde::Serialize;

use crate::engine::dataspace::Dataspace;
use crate::engine::dataspace::conflicts;
pub use crate::engine::dataspace::conflicts::ConflictRow;
pub use crate::engine::dataspace::conflicts::Decision;
use crate::engine::error::Error;
use crate::engine::notify::Notify;
use crate::engine::services::Services;

/// The message of a revision a hand publish or pull commits of the folder's
/// changes, as the sync it runs takes them.
pub(super) const PUBLISH_MESSAGE: &str = "Published from QuiltSync";

fn namespace(text: &str) -> Result<Namespace, Error> {
    Ok(Namespace::try_from(text)?)
}

/// What `entry`'s sync ended with, as an error where it did not complete.
fn settled(entry: &Entry, outcome: &EntrySync) -> Result<(), Error> {
    match outcome {
        EntrySync::Blocked { conflicts, .. } => Err(Error::Dataspace(format!(
            "{} has {} conflicting files; decide them, then sync again",
            entry.package,
            conflicts.len()
        ))),
        EntrySync::Diverged { .. } => Err(Error::Dataspace(format!(
            "{} diverged from its latest revision; reset it, or certify it",
            entry.package
        ))),
        EntrySync::Busy => Err(Error::Dataspace(format!(
            "{} is busy in another process; try again",
            entry.package
        ))),
        EntrySync::Failed(err) => Err(Error::Dataspace(err.to_string())),
        EntrySync::Ran {
            published: Some(Published::WorkflowBlocked { reason, .. }),
            ..
        } => Err(Error::Dataspace(format!(
            "{}: the bucket's workflow refused the revision: {reason}",
            entry.package
        ))),
        EntrySync::Ran {
            published: Some(Published::Stored { .. } | Published::Behind { .. }),
            ..
        } => Err(Error::Dataspace(format!(
            "{} was stored, but latest names another revision; pull, then publish again",
            entry.package
        ))),
        // It ran, or read the folder back.
        _ => Ok(()),
    }
}

/// Syncs the default dataspace, and answers how `namespace`'s entry ended:
/// the dataspace moves as a whole, its lock being one file.
async fn sync_package<R: Remote + Sync>(
    ds: &Dataspace<R>,
    namespace: &Namespace,
    message: &str,
) -> Result<(), Error> {
    let entry = ds.entry(namespace).await?;
    match entry.access {
        Access::Read => {
            if Box::pin(ds.read()).await?.completed() {
                return Ok(());
            }
            return Err(Error::Dataspace(format!("{namespace} was not read whole")));
        }
        Access::Write | Access::Replace => {
            if Box::pin(ds.publish()).await?.completed() {
                return Ok(());
            }
            return Err(Error::Dataspace(format!("{namespace} was not published")));
        }
        Access::Sync => {}
    }
    // Boxed: the sync's state machine is past the `large_futures` budget.
    let synced = Box::pin(ds.sync(message)).await?;
    if synced.read_only {
        return Err(Error::Dataspace(format!(
            "{namespace} was read, not published: the dataspace is not read in full yet; try again"
        )));
    }
    match synced.entries.iter().find(|(synced, _)| *synced == entry) {
        Some((entry, outcome)) => settled(entry, outcome),
        None => Ok(()),
    }
}

pub(super) async fn commit<R: Remote + Sync>(
    ds: &Dataspace<R>,
    text: &str,
    message: &str,
) -> Result<CommitResponse, String> {
    let notify = Notify::new(format!("Committing package {text}"));
    let result = async {
        if message.trim().is_empty() {
            return Err(Error::Commit("Message is required".to_string()));
        }
        ds.commit(&namespace(text)?, message).await
    }
    .await;
    let nothing_to_commit = matches!(&result, Ok(committed) if committed.commit.is_none());
    let msg_ok = if nothing_to_commit {
        format!("Package {text} has nothing to commit")
    } else {
        format!("Successfully committed {text}")
    };
    notify
        .map(result, msg_ok, |err| format!("Failed to commit: {err}"))
        .map(|message| CommitResponse {
            message,
            workflow_warning: None,
            nothing_to_commit,
        })
}

/// Commits, then syncs, which pushes the commit.
pub(super) async fn commit_and_push<R: Remote + Sync>(
    ds: &Dataspace<R>,
    text: &str,
    message: &str,
) -> Result<String, String> {
    let result = async {
        if message.trim().is_empty() {
            return Err(Error::Commit("Message is required".to_string()));
        }
        let namespace = namespace(text)?;
        ds.commit(&namespace, message).await?;
        sync_package(ds, &namespace, message).await
    }
    .await;
    Notify::new(format!("Publishing package {text}")).map(result, String::new(), |err| {
        format!("Failed to publish package: {err}")
    })
}

/// Publishes: a sync, which commits the folder and pushes what the lock
/// holds.
pub(super) async fn publish<R: Remote + Sync>(
    ds: &Dataspace<R>,
    text: &str,
    message: &str,
) -> Result<String, String> {
    let result = async { sync_package(ds, &namespace(text)?, message).await }.await;
    Notify::new(format!("Publishing package {text}")).map(result, String::new(), |err| {
        format!("Failed to publish package: {err}")
    })
}

/// Pulls: a sync, which reads what `latest` moved; a `read` entry is read.
pub(super) async fn pull<R: Remote + Sync>(
    ds: &Dataspace<R>,
    text: &str,
    message: &str,
) -> Result<String, String> {
    let result = async { sync_package(ds, &namespace(text)?, message).await }.await;
    Notify::new(format!("Pulling package {text}")).map(
        result,
        format!("Successfully pulled {text}"),
        |err| format!("Failed to pull package: {err}"),
    )
}

pub(super) async fn certify<R: Remote + Sync>(
    ds: &Dataspace<R>,
    text: &str,
) -> Result<String, String> {
    let result = async { ds.certify(&namespace(text)?).await }.await;
    Notify::new(format!("Certifying latest for {text}")).map(
        result,
        format!("Successfully certified latest for {text}"),
        |err| format!("Failed to certify latest: {err}"),
    )
}

pub(super) async fn reset<R: Remote + Sync>(
    ds: &Dataspace<R>,
    text: &str,
) -> Result<String, String> {
    let result = async { ds.reset(&namespace(text)?).await }.await;
    Notify::new(format!("Resetting local for {text}")).map(
        result,
        format!("Successfully reset local for {text}"),
        |err| format!("Failed to reset local: {err}"),
    )
}

pub(super) async fn undo<R: Remote + Sync>(
    ds: &Dataspace<R>,
    text: &str,
) -> Result<String, String> {
    let result = async { ds.undo(&namespace(text)?).await }.await;
    Notify::new(format!("Undoing the last revision of {text}")).map(
        result,
        format!("Undid the last revision of {text}"),
        |err| format!("Failed to undo the last revision: {err}"),
    )
}

/// Uninstalls under the package's lock; a prune that fails after it is
/// logged, as on the ref path, since the package is gone.
pub(super) async fn uninstall<R: Remote + Sync>(
    ds: &Dataspace<R>,
    text: &str,
    prune: bool,
) -> Result<String, String> {
    let result = async { ds.uninstall(&namespace(text)?, prune).await }.await;
    Notify::new(format!("Uninstalling package {text}")).map(
        result,
        format!("Successfully uninstalled package {text}"),
        |err| format!("Failed to uninstall package: {err}"),
    )
}

/// The paths a Keeping choice names, as `data.toml` spells them (25t
/// decision 1): the whole package, or each folder ending in `/`. A choice of
/// individual files names no file here, so it is refused.
fn scope(scope: KeepingScope, folders: &[String]) -> Result<Scope, Error> {
    match scope {
        KeepingScope::EntirePackage => Ok(Scope::Whole),
        KeepingScope::Folders => {
            let paths: Vec<String> = folders
                .iter()
                .map(|folder| folder.trim_end_matches('/'))
                .map(|folder| {
                    let named = !folder.is_empty()
                        && !folder.starts_with('/')
                        && !folder.split('/').any(|part| part == ".." || part == ".");
                    named
                        .then(|| format!("{folder}/"))
                        .ok_or_else(|| Error::Dataspace(format!("{folder:?} is not a folder name")))
                })
                .collect::<Result<_, _>>()?;
            if paths.is_empty() {
                return Err(Error::Dataspace("choose at least one folder".to_string()));
            }
            Ok(Scope::Only(paths))
        }
        KeepingScope::IndividualFiles => Err(Error::Dataspace(
            "a dataspace keeps the files data.toml names; choose folders or the entire package"
                .to_string(),
        )),
    }
}

pub(super) async fn set_scope<R: Remote + Sync>(
    ds: &Dataspace<R>,
    text: &str,
    keeping: KeepingScope,
    folders: Vec<String>,
) -> Result<(), String> {
    let result = async {
        let mut request = Keep::new(ds.package_key(&namespace(text)?).await?);
        request.paths = Some(scope(keeping, &folders)?);
        ds.keep(&request).await
    }
    .await;
    result.map(|_| ()).map_err(|err| err.to_string())
}

/// Creates `text` in this machine's local registry, under `sync`, from
/// `source` where one is given, as the ref path creates a package with no
/// remote.
pub(super) async fn create<R: Remote + Sync>(
    ds: &Dataspace<R>,
    text: &str,
    source: Option<PathBuf>,
    message: Option<String>,
) -> Result<String, String> {
    let result = async {
        let namespace = namespace(text)?;
        let name = ds
            .domain()
            .local()
            .names(ds.storage())
            .await
            .map_err(|err| Error::Dataspace(err.to_string()))?
            .name;
        let mut request = Create::new(format!("quilt+local://{name}#package={namespace}"));
        request.folder = source;
        request.message = message;
        ds.create(&request).await
    }
    .await;
    Notify::new(format!("Creating package {text}")).map(
        result,
        format!("Successfully created package {text}"),
        |err| format!("Failed to create package: {err}"),
    )
}

/// Installs the package a link names under `sync`, as the contract places
/// it, and reads it.
pub(super) async fn install<R: Remote + Sync>(
    ds: &Dataspace<R>,
    uri: &str,
    namespace: Namespace,
) -> Result<RemotePackageResult, String> {
    let installed = ds
        .install(&Install::new(uri))
        .await
        .map_err(|err| err.to_string())?;
    if !installed.read.completed() {
        return Err(format!(
            "{namespace} was declared, but not read: {:?}",
            installed.read
        ));
    }
    Ok(RemotePackageResult {
        namespace,
        banner: None,
    })
}

fn bucket(entry: &Entry) -> Option<String> {
    match &entry.registry {
        Registry::S3(bucket) => Some(bucket.clone()),
        _ => None,
    }
}

/// The main page's rows: each package the default dataspace declares, with
/// its state from the lock and the folder, `latest` from the index.
pub(super) async fn main_page_packages<R: Remote + Sync>(
    ds: &Dataspace<R>,
) -> Result<MainPagePackages, String> {
    let declared = ds.list(false).await.map_err(|err| err.to_string())?;
    let mut packages: Vec<MainPagePackage> = Vec::new();
    for package in declared {
        // A package under `read` and `write` is one row.
        if packages
            .iter()
            .any(|row| row.namespace == package.entry.package)
        {
            continue;
        }
        let state = if package.conflicts > 0 {
            PackageStateDto::PullConflict { files: Vec::new() }
        } else {
            PackageStateDto::from(package.state)
        };
        packages.push(MainPagePackage {
            namespace: package.entry.package.clone(),
            state,
            changed_at: None,
            bucket: bucket(&package.entry),
            host: package.entry.catalog.as_ref().map(ToString::to_string),
            provisional: false,
            role_switch_host: None,
        });
    }
    Ok(MainPagePackages { packages })
}

/// One row's state, from `latest` asked of its registry now.
pub(super) async fn refresh_package<R: Remote + Sync>(
    ds: &Dataspace<R>,
    text: &str,
) -> Result<MainPagePackageRefresh, String> {
    let status = async { ds.entry_status(&namespace(text)?, true).await }
        .await
        .map_err(|err| err.to_string())?;
    let state = if status.conflicts.is_empty() {
        PackageStateDto::from(status.state())
    } else {
        PackageStateDto::PullConflict {
            files: status
                .conflicts
                .keys()
                .map(|key| key.as_path().display().to_string())
                .collect(),
        }
    };
    Ok(MainPagePackageRefresh {
        state,
        role_switch_host: None,
        downloads: None,
    })
}

/// A dataspace package's history, newest first. Nothing is offered for
/// removal from here: Remove revisions names its hashes.
pub(super) async fn revision_history<R: Remote + Sync>(
    ds: &Dataspace<R>,
    text: &str,
) -> Result<RevisionHistoryData, String> {
    let revisions = async { ds.history(&namespace(text)?).await }
        .await
        .map_err(|err| err.to_string())?;
    let rows = revisions
        .into_iter()
        .map(|revision| RevisionHistoryRow {
            hash: revision.hash,
            message: revision.message,
            obtained_at: revision.time.map_or(0.0, |time| {
                #[allow(clippy::cast_precision_loss, reason = "milliseconds since 1970 fit")]
                let millis = time.as_millisecond() as f64;
                millis
            }),
            published: !revision.local,
            catalog_url: None,
            kept: Vec::new(),
            frees: None,
        })
        .collect();
    Ok(RevisionHistoryData {
        rows,
        removable_frees: None,
    })
}

pub(super) async fn remove_revisions<R: Remote + Sync>(
    ds: &Dataspace<R>,
    text: &str,
    hashes: &[String],
) -> Result<String, String> {
    let result = async { ds.remove_revisions(&namespace(text)?, hashes).await }.await;
    Notify::new(format!("Removing old revisions of {text}")).map(result, String::new(), |err| {
        format!("Could not remove old revisions: {err}")
    })
}

pub(super) async fn gc<R: Remote + Sync>(ds: &Dataspace<R>) -> Result<FreedSpace, String> {
    let result = ds.gc().await;
    let freed_bytes = result
        .as_ref()
        .map_or(0, |collected| collected.total().bytes);
    let msg_ok = result
        .as_ref()
        .map_or_else(|_| String::new(), |collected| collected.total().to_string());
    Notify::new("Freeing up space".to_string())
        .map(result, msg_ok, |err| {
            format!("Failed to free up space: {err}")
        })
        .map(|message| FreedSpace {
            message,
            freed_bytes,
        })
}

pub(super) async fn measure<R: Remote + Sync>(ds: &Dataspace<R>) -> Result<StorageSize, String> {
    let size = ds
        .measure()
        .await
        .map_err(|err| format!("Failed to measure local storage: {err}"))?;
    let freeable = ds
        .gc_estimate()
        .await
        .map_or(0, |collected| collected.total().bytes);
    Ok(StorageSize {
        total_bytes: size.total_bytes,
        freeable_bytes: freeable,
    })
}

/// Makes `directory` the default dataspace.
pub(super) async fn setup<R: Remote + Sync>(
    ds: &Dataspace<R>,
    directory: &str,
) -> Result<String, String> {
    let result = ds.set_home(std::path::Path::new(directory)).await;
    Notify::new(format!("Setup with directory {directory}")).map(
        result,
        format!("Successfully set up directory: {directory}"),
        |err| format!("Failed to create QuiltSync directory: {err}"),
    )
}

/// What a command only the dataspace engine has answers with it off.
const ENGINE_OFF: &str = "This needs the dataspace engine, which is off";

/// The rows of the default dataspace's `conflict.json`; none with the
/// dataspace engine off.
pub async fn get_conflicts(s: &Services) -> Result<Vec<ConflictRow>, String> {
    match &s.dataspace {
        Some(ds) => conflicts::rows(ds).await.map_err(|err| err.to_string()),
        None => Ok(Vec::new()),
    }
}

/// Writes a person's decision on one row of `conflict.json`, for the next
/// read or sync to check and take.
pub async fn decide_conflict(
    s: &Services,
    package: String,
    key: String,
    decision: Decision,
) -> Result<(), String> {
    let ds = s.dataspace.as_ref().ok_or(ENGINE_OFF)?;
    conflicts::decide(ds, &package, &key, decision)
        .await
        .map_err(|err| err.to_string())
}

/// A local commit of a package that no lock names, which reattach puts
/// back.
#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnattachedCommit {
    pub namespace: Namespace,
    pub hash: String,
    pub message: Option<String>,
}

/// The local commits of each package the default dataspace declares that
/// its lock does not name, as an undo or an uninstall and install again
/// leave them; none with the dataspace engine off.
pub async fn get_unattached_commits(s: &Services) -> Result<Vec<UnattachedCommit>, String> {
    match &s.dataspace {
        Some(ds) => unattached(ds).await.map_err(|err| err.to_string()),
        None => Ok(Vec::new()),
    }
}

async fn unattached<R: Remote + Sync>(ds: &Dataspace<R>) -> Result<Vec<UnattachedCommit>, Error> {
    let namespaces: std::collections::BTreeSet<Namespace> = ds
        .contract()
        .await?
        .entries()
        .iter()
        .map(|entry| entry.package.clone())
        .collect();
    let mut commits = Vec::new();
    for namespace in namespaces {
        for commit in ds.unattached(&namespace).await? {
            commits.push(UnattachedCommit {
                namespace: namespace.clone(),
                hash: commit.hash,
                message: commit.message,
            });
        }
    }
    Ok(commits)
}

/// Puts `hash` back as `namespace`'s last local commit, on the base the
/// lock holds for it, so the next sync pushes it.
pub async fn reattach_commit(
    s: &Services,
    namespace: String,
    hash: String,
) -> Result<String, String> {
    let ds = s.dataspace.as_ref().ok_or(ENGINE_OFF)?;
    let result = async { ds.reattach(&self::namespace(&namespace)?, &hash).await }.await;
    Notify::new(format!("Reattaching {hash} to {namespace}")).map(
        result,
        format!("Reattached the commit to {namespace}"),
        |err| format!("Failed to reattach the commit: {err}"),
    )
}

#[cfg(test)]
mod tests;
