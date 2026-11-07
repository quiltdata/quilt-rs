//! A Diverged copy's change, merged onto `latest` and published as one
//! revision, the copy then reset to it.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::PathBuf;

use quilt_uri::Namespace;

use super::RefreshOutcome;
use super::WatchError;
use super::classify_sync_err;
use super::classify_transient_or_login;
use crate::engine::autopull::PausedReason;
use crate::engine::autopull::activity::ActivityOp;
use crate::engine::autopull::reporter::settled_fingerprint;
use crate::engine::autopull::reporter::status_fingerprint;
use crate::engine::autopull::status::SyncTrayAggregator;
use crate::engine::model::QuiltModel;
use crate::engine::quilt;
use crate::engine::quilt::lineage::LineagePaths;
use crate::engine::quilt::lineage::UpstreamState;
use crate::engine::quilt::manifest::Manifest;
use crate::engine::quilt::object_hash::ObjectHash;
use crate::engine::telemetry::prelude::*;

/// Attempts in all, each from a fresh read of `latest`.
const ATTEMPTS: usize = 3;

/// Merge the change from the copy's base to its pushed revision onto
/// `latest`, publish it as `latest`'s child, and reset the copy to it while
/// the folder holds what was committed. A refusal, or a `latest` that keeps
/// moving, pauses the copy as Diverged.
pub(super) async fn onto_latest<M: QuiltModel>(
    model: &M,
    locked: &M::Locked,
    installed: &quilt::InstalledPackage,
    aggregator: &SyncTrayAggregator,
    ignored: &str,
) -> Result<RefreshOutcome, WatchError> {
    let namespace = &installed.namespace;
    let held = Held {
        model,
        locked,
        aggregator,
        namespace,
        ignored,
    };
    let lineage = model
        .get_installed_package_lineage(installed)
        .await
        .map_err(WatchError::Transient)?;
    // Merged only from a pushed revision.
    let Some(ours_uri) = lineage
        .remote_uri
        .clone()
        .filter(|uri| lineage.commit.is_none() && !uri.hash.is_empty())
    else {
        return Err(WatchError::Conflict(PausedReason::Diverged));
    };
    let base_uri = quilt_uri::ManifestUri {
        hash: lineage.base_hash.clone(),
        ..ours_uri.clone()
    };
    let base = read(model, &base_uri).await?;
    let ours = read(model, &ours_uri).await?;
    let package = quilt_uri::S3PackageUri {
        revision: quilt_uri::RevisionPointer::Tag(quilt_uri::Tag::Latest),
        ..(&ours_uri).into()
    };
    for _ in 0..ATTEMPTS {
        let theirs_uri = model
            .resolve_manifest_uri(&package)
            .await
            .map_err(classify_transient_or_login)?;
        let theirs = read(model, &theirs_uri).await?;
        let merged = match quilt::flow::merge_onto(&base, &ours, theirs.clone()) {
            Ok(merged) => merged,
            Err(refusal) => {
                warn!("autosync: {namespace}: cannot merge onto latest: {refusal:?}");
                return Err(WatchError::Conflict(PausedReason::Diverged));
            }
        };
        if is_unchanged(&merged, &theirs) {
            info!("autosync: {namespace}: latest already holds this copy's change");
            let landed = Landed {
                merged,
                uri: theirs_uri,
                published: false,
            };
            return reset(&held, &lineage.paths, &ours, landed).await;
        }
        let pushed = {
            let _activity = aggregator.activity_guard(ActivityOp::Publish, namespace);
            model.push_onto(&theirs_uri, merged.clone()).await
        }
        .map_err(sync_err)?;
        if pushed.certified_latest {
            info!("autosync: merged namespace={namespace} onto latest");
            let landed = Landed {
                merged,
                uri: pushed.manifest_uri,
                published: true,
            };
            return reset(&held, &lineage.paths, &ours, landed).await;
        }
    }
    warn!("autosync: {namespace}: latest kept moving; left Diverged");
    Err(WatchError::Conflict(PausedReason::Diverged))
}

async fn read<M: QuiltModel>(
    model: &M,
    uri: &quilt_uri::ManifestUri,
) -> Result<Manifest, WatchError> {
    model
        .browse_remote_manifest(uri)
        .await
        .map_err(classify_transient_or_login)
}

/// Whether the merge left `theirs` as it was, so there is nothing to publish.
fn is_unchanged(merged: &Manifest, theirs: &Manifest) -> bool {
    let mut rows = theirs.rows.clone();
    rows.sort_by(|a, b| a.logical_key.cmp(&b.logical_key));
    merged.header == theirs.header && merged.rows == rows
}

/// The package the merge runs over, under its held lock.
struct Held<'a, M: QuiltModel> {
    model: &'a M,
    locked: &'a M::Locked,
    aggregator: &'a SyncTrayAggregator,
    namespace: &'a Namespace,
    ignored: &'a str,
}

/// The merged revision as `latest`, published by this merge or already there.
struct Landed {
    merged: Manifest,
    uri: quilt_uri::ManifestUri,
    published: bool,
}

/// Reset the copy to `latest` while the folder holds what was committed, and
/// report what the merged revision brought into it. An edit since leaves the
/// copy Diverged at its own revision for the next tick.
async fn reset<M: QuiltModel>(
    held: &Held<'_, M>,
    tracked: &LineagePaths,
    ours: &Manifest,
    landed: Landed,
) -> Result<RefreshOutcome, WatchError> {
    let Landed {
        merged,
        uri,
        published,
    } = landed;
    let published = published.then(|| merged.header.message.clone().unwrap_or_default());
    let status = held
        .model
        .locked_package_status(held.locked, None)
        .await
        .map_err(classify_transient_or_login)?;
    if !status.changes.is_empty() {
        warn!("autosync: the folder changed during the merge; left Diverged");
        return Ok(RefreshOutcome {
            published,
            ..RefreshOutcome::observed(UpstreamState::Diverged, true, status_fingerprint(&status))
        });
    }
    let latest = {
        let _applying = held.aggregator.apply_guard(held.namespace);
        let _activity = held
            .aggregator
            .activity_guard(ActivityOp::Pull, held.namespace);
        held.model.locked_package_reset_local(held.locked).await
    }
    .map_err(sync_err)?;
    // A writer who published after the merge moved `latest` past it, and the
    // reset wrote theirs, which the merge cannot describe.
    let pulled = (latest.hash == uri.hash).then(|| report(tracked, ours, &merged, uri));
    Ok(RefreshOutcome {
        published,
        pulled,
        ..RefreshOutcome::observed(
            UpstreamState::UpToDate,
            false,
            settled_fingerprint(&latest.hash, held.ignored),
        )
    })
}

/// What the reset brought into the folder: the rows `merged` holds otherwise
/// than `ours`. A reset rewrites tracked paths only, so a path new to this
/// copy stays on the remote.
fn report(
    tracked: &LineagePaths,
    ours: &Manifest,
    merged: &Manifest,
    manifest_uri: quilt_uri::ManifestUri,
) -> quilt::flow::PullReport {
    let (ours, now) = (hashes(ours), hashes(merged));
    let keys: BTreeSet<&PathBuf> = ours.keys().chain(now.keys()).copied().collect();
    let mut report = quilt::flow::PullReport {
        manifest_uri,
        added: Vec::new(),
        added_not_fetched: Vec::new(),
        updated: Vec::new(),
        removed: Vec::new(),
        message: merged.header.message.clone().filter(|m| !m.is_empty()),
    };
    for key in keys {
        let is_tracked = tracked.contains_key(key);
        match (ours.get(key), now.get(key)) {
            (None, Some(_)) => report.added_not_fetched.push(key.clone()),
            (Some(was), Some(is)) if was != is && is_tracked => report.updated.push(key.clone()),
            (Some(_), None) if is_tracked => report.removed.push(key.clone()),
            _ => {}
        }
    }
    report
}

fn hashes(manifest: &Manifest) -> BTreeMap<&PathBuf, &ObjectHash> {
    manifest
        .rows
        .iter()
        .map(|row| (&row.logical_key, &row.hash))
        .collect()
}

/// [`classify_sync_err`]'s classification, which is never `Ok`.
fn sync_err(err: crate::engine::Error) -> WatchError {
    classify_sync_err(err).expect_err("an error classifies as one")
}
