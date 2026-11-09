//! A Diverged copy's change, merged onto `latest` and published as one
//! revision, the copy then moved to it.

use quilt_uri::Namespace;

use super::RefreshOutcome;
use super::WatchError;
use super::classify_sync_err;
use super::classify_transient_or_login;
use crate::engine::autopull::PausedReason;
use crate::engine::autopull::activity::ActivityOp;
use crate::engine::autopull::reporter::status_fingerprint;
use crate::engine::autopull::status::SyncTrayAggregator;
use crate::engine::model::QuiltModel;
use crate::engine::quilt;
use crate::engine::quilt::lineage::SyncScope;
use crate::engine::quilt::manifest::Manifest;
use crate::engine::telemetry::prelude::*;

/// Attempts in all, each from a fresh read of `latest`.
const ATTEMPTS: usize = 3;

/// Merge the change from the copy's base to its pushed revision onto
/// `latest`, publish it as `latest`'s child, and move the copy to it. A
/// refusal, or a `latest` that keeps moving, pauses the copy as Diverged.
pub(super) async fn onto_latest<M: QuiltModel>(
    model: &M,
    locked: &M::Locked,
    installed: &quilt::InstalledPackage,
    aggregator: &SyncTrayAggregator,
    scope: SyncScope,
) -> Result<RefreshOutcome, WatchError> {
    let namespace = &installed.namespace;
    let held = Held {
        model,
        locked,
        aggregator,
        namespace,
        scope,
    };
    let lineage = model
        .get_installed_package_lineage(installed)
        .await
        .map_err(WatchError::Transient)?;
    // Merged only from a pushed revision.
    let Some(ours_uri) = lineage
        .remote_uri
        .filter(|uri| lineage.commit.is_none() && !uri.hash.is_empty())
    else {
        return Err(WatchError::Conflict(PausedReason::Diverged));
    };
    let base_uri = quilt_uri::ManifestUri {
        hash: lineage.base_hash,
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
            return move_to(&held, &theirs_uri, None).await;
        }
        let message = merged.header.message.clone().unwrap_or_default();
        let pushed = {
            let _activity = aggregator.activity_guard(ActivityOp::Publish, namespace);
            model.push_onto(&theirs_uri, merged).await
        }
        .map_err(sync_err)?;
        if pushed.certified_latest {
            info!("autosync: merged namespace={namespace} onto latest");
            return move_to(&held, &pushed.manifest_uri, Some(message)).await;
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
/// The merge always carries the copy's message, so the message is no sign of
/// new work.
fn is_unchanged(merged: &Manifest, theirs: &Manifest) -> bool {
    let mut rows = theirs.rows.clone();
    rows.sort_by(|a, b| a.logical_key.cmp(&b.logical_key));
    merged.header.user_meta == theirs.header.user_meta && merged.rows == rows
}

/// The package the merge runs over, under its held lock.
struct Held<'a, M: QuiltModel> {
    model: &'a M,
    locked: &'a M::Locked,
    aggregator: &'a SyncTrayAggregator,
    namespace: &'a Namespace,
    scope: SyncScope,
}

/// Move the copy to `merged`, now `latest`, as a pull moves a Behind copy, and
/// report what that brought into the folder. An edit to a path the other side
/// changed keeps the copy Diverged at its own revision for the next tick; any
/// other file in the way pauses it as Diverged.
async fn move_to<M: QuiltModel>(
    held: &Held<'_, M>,
    merged: &quilt_uri::ManifestUri,
    published: Option<String>,
) -> Result<RefreshOutcome, WatchError> {
    let moved = {
        let _applying = held.aggregator.apply_guard(held.namespace);
        let _activity = held
            .aggregator
            .activity_guard(ActivityOp::Pull, held.namespace);
        held.model
            .locked_package_move_to(held.locked, merged, held.scope.clone())
            .await
    };
    let moved = match moved {
        Err(crate::engine::Error::Quilt(quilt::Error::PackageOp(
            quilt::PackageOpError::PullConflict(paths),
        ))) => Err(paths),
        Err(err) => return Err(sync_err(err)),
        Ok(pulled) => Ok(pulled),
    };
    let status = held
        .model
        .locked_package_status(held.locked, None)
        .await
        .map_err(classify_transient_or_login)?;
    let pulled = match moved {
        Ok(quilt::flow::Pulled::Applied(report)) => Some(report),
        Ok(quilt::flow::Pulled::UpToDate) => None,
        // Edits, which the next tick commits and merges again.
        Err(paths) if paths.iter().all(|path| status.changes.contains_key(path)) => {
            warn!(
                "autosync: {}: edited during the merge at {paths:?}; left Diverged",
                held.namespace
            );
            None
        }
        Err(paths) => {
            warn!(
                "autosync: {}: files in the way of the merged revision at {paths:?}",
                held.namespace
            );
            return Err(WatchError::Conflict(PausedReason::Diverged));
        }
    };
    Ok(RefreshOutcome {
        published,
        pulled,
        ..RefreshOutcome::observed(
            status.upstream_state,
            !status.changes.is_empty(),
            status_fingerprint(&status),
        )
    })
}

/// [`classify_sync_err`]'s classification, which is never `Ok`.
fn sync_err(err: crate::engine::Error) -> WatchError {
    classify_sync_err(err).expect_err("an error classifies as one")
}
