//! The dataspace engine's autosync: quilt-rs's [`Runner`] ticking in place
//! of the ref engine's autopull tick, and a watcher on the default
//! dataspace's folder that tells the runner what changed.
//!
//! [`Runner`]: quilt_rs::dataspace::autosync::Runner

use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use std::time::SystemTime;

use notify::RecommendedWatcher;
use notify::RecursiveMode;
use notify_debouncer_full::Debouncer;
use notify_debouncer_full::RecommendedCache;
use notify_debouncer_full::new_debouncer;
use quilt_rs::dataspace::autosync::Tick;
use quilt_rs::io::remote::Remote;
use quilt_rs::queue::JobKey;
use tokio::sync::mpsc;

use super::Dataspace;
use super::fail;
use crate::engine::error::Error;
use crate::engine::telemetry::prelude::*;

/// One tick of the runner over the dataspaces on autosync, where either of
/// the app's autosync directions is on; `None` where both are off. The runner
/// reads, publishes and syncs as each entry's access says, so the two
/// directions are not told apart.
pub async fn tick<R: Remote + Sync>(
    ds: &Dataspace<R>,
    pull: bool,
    push: bool,
) -> Result<Option<Tick>, Error> {
    if !pull && !push {
        return Ok(None);
    }
    let _writes = ds.hold_writes().await;
    ds.runner()
        .tick(ds.storage(), ds.remote(), SystemTime::now())
        .await
        .map(Some)
        .map_err(fail)
}

/// Whether a change at `path` is one the tick's commit would take: inside
/// an entry's working folder, and not left out by that folder's rules, the
/// contract's `[ignore]` table among them.
pub async fn taken<R: Remote + Sync>(ds: &Dataspace<R>, path: &Path) -> bool {
    let Ok(contract) = ds.contract().await else {
        return false;
    };
    let Some(entry) = contract
        .entries()
        .iter()
        .find(|entry| path.starts_with(&entry.folder))
    else {
        return false;
    };
    let Ok(relative) = path.strip_prefix(&entry.folder) else {
        return false;
    };
    if relative.as_os_str().is_empty() {
        return true;
    }
    let Ok(rules) = ds.ignore_rules(&entry.folder).await else {
        return true;
    };
    // A path the event no longer finds was a file or a folder; either way
    // the rules that leave out its name leave it out.
    let dir = path.is_dir();
    let ignored = if dir {
        rules.dir_ignored_by(relative, false)
    } else {
        rules.file_ignored_by(relative, false)
    };
    ignored.is_none()
}

/// Queues the autosync jobs a change at `path` makes, as the runner's
/// watcher should: none for a path the tick's commit leaves out.
pub async fn changed<R: Remote + Sync>(
    ds: &Dataspace<R>,
    path: &Path,
) -> Result<Vec<JobKey>, Error> {
    if !taken(ds, path).await {
        return Ok(Vec::new());
    }
    ds.runner().changed(ds.storage(), path).await.map_err(fail)
}

/// Watches the default dataspace's folder, and queues each change's jobs.
/// The watch stops when the returned debouncer is dropped.
pub fn watch(
    ds: Arc<Dataspace>,
    debounce: Duration,
) -> Result<Debouncer<RecommendedWatcher, RecommendedCache>, Error> {
    let (tx, mut rx) = mpsc::unbounded_channel::<Vec<PathBuf>>();
    let mut debouncer = new_debouncer(
        debounce,
        None,
        move |result: notify_debouncer_full::DebounceEventResult| match result {
            Ok(events) => {
                let paths = events
                    .into_iter()
                    .flat_map(|event| event.event.paths)
                    .collect();
                // Fails only once the task below is gone.
                let _ = tx.send(paths);
            }
            Err(errors) => {
                for err in errors {
                    warn!("dataspace watcher: {err}");
                }
            }
        },
    )
    .map_err(fail)?;
    debouncer
        .watch(ds.dataspace(), RecursiveMode::Recursive)
        .map_err(fail)?;
    crate::engine::events::spawn(async move {
        while let Some(mut paths) = rx.recv().await {
            paths.sort();
            paths.dedup();
            for path in paths {
                if let Err(err) = changed(&ds, &path).await {
                    warn!("dataspace watcher: {}: {err}", path.display());
                }
            }
        }
    });
    Ok(debouncer)
}

#[cfg(test)]
mod tests;
