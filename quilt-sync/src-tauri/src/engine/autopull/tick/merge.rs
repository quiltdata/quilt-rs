//! A Diverged copy's change, merged onto `latest` and published as one
//! revision, the copy then moved to it.

use quilt_uri::Namespace;

use super::Published;
use super::RefreshOutcome;
use super::WatchError;
use super::classify_sync_err;
use super::classify_transient_or_login;
use crate::engine::autopull::MergeBlock;
use crate::engine::autopull::PausedReason;
use crate::engine::autopull::activity::ActivityOp;
use crate::engine::autopull::reporter::status_fingerprint;
use crate::engine::autopull::status::SyncTrayAggregator;
use crate::engine::model::QuiltModel;
use crate::engine::quilt;
use crate::engine::quilt::lineage::SyncScope;
use crate::engine::quilt::manifest::Manifest;
use crate::engine::record::Draft;
use crate::engine::telemetry::prelude::*;

/// Attempts in all, each from a fresh read of `latest`.
const ATTEMPTS: usize = 3;

/// Merge the change from the copy's base to its pushed revision onto
/// `latest`, publish it as `latest`'s child, and move the copy to it. A
/// refusal, or a `latest` that keeps moving, pauses the copy on what blocked it.
pub(super) async fn onto_latest<M: QuiltModel>(
    model: &M,
    locked: &M::Locked,
    installed: &quilt::InstalledPackage,
    aggregator: &SyncTrayAggregator,
    scope: SyncScope,
    published: &mut Vec<Published>,
) -> Result<RefreshOutcome, WatchError> {
    let namespace = &installed.namespace;
    let held = Held {
        model,
        locked,
        aggregator,
        namespace,
        scope,
    };
    let Some(Sides {
        base,
        ours,
        package,
    }) = sides(model, installed).await?
    else {
        return Err(WatchError::Conflict(PausedReason::Diverged));
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
                let block = match refusal {
                    quilt::flow::MergeRefusal::Paths(paths) => {
                        MergeBlock::Paths(paths.iter().map(|p| p.display().to_string()).collect())
                    }
                    quilt::flow::MergeRefusal::UserMeta => MergeBlock::Metadata,
                };
                return Err(WatchError::Conflict(PausedReason::MergeRefused(block)));
            }
        };
        if is_unchanged(&merged, &theirs) {
            info!("autosync: {namespace}: latest already holds this copy's change");
            return move_to(&held, &theirs_uri).await;
        }
        let message = merged.header.message.clone().unwrap_or_default();
        let changes = crate::engine::record::changes_between(&theirs.rows, &merged.rows);
        let pushed = {
            let _activity = aggregator.activity_guard(ActivityOp::Publish, namespace);
            model.push_onto(&theirs_uri, merged).await
        }
        .map_err(sync_err)?;
        // Kept as soon as it is uploaded, `latest` or not: the move after can
        // fail, and a later attempt can run out.
        let entry = Draft::published(
            namespace,
            Some(&theirs_uri.hash),
            &pushed.manifest_uri.hash,
            &changes,
        );
        published.push(Published {
            message,
            entry: entry.latest_or_not(pushed.certified_latest),
        });
        if pushed.certified_latest {
            info!("autosync: merged namespace={namespace} onto latest");
            return move_to(&held, &pushed.manifest_uri).await;
        }
    }
    warn!("autosync: {namespace}: latest kept moving; left Diverged");
    Err(WatchError::Conflict(PausedReason::MergeRefused(
        MergeBlock::LatestMoved,
    )))
}

/// Move a Diverged copy onto `latest` before its new edits are committed, when
/// `latest` already holds its pushed revision: committed onto that revision, an
/// edit to a file the copy published would collide with `latest`'s copy of it.
/// `None` when `latest` lacks the copy's change or edits block the move; the
/// tick then commits and merges.
pub(super) async fn landed<M: QuiltModel>(
    model: &M,
    locked: &M::Locked,
    installed: &quilt::InstalledPackage,
    aggregator: &SyncTrayAggregator,
    scope: SyncScope,
) -> Result<Option<RefreshOutcome>, WatchError> {
    let Some(Sides {
        base,
        ours,
        package,
    }) = sides(model, installed).await?
    else {
        return Ok(None);
    };
    let latest_uri = model
        .resolve_manifest_uri(&package)
        .await
        .map_err(classify_transient_or_login)?;
    let latest = read(model, &latest_uri).await?;
    match quilt::flow::merge_onto(&base, &ours, latest.clone()) {
        Ok(merged) if is_unchanged(&merged, &latest) => {}
        _ => return Ok(None),
    }
    info!(
        "autosync: {}: latest already holds this copy's change; moving before committing",
        installed.namespace
    );
    let held = Held {
        model,
        locked,
        aggregator,
        namespace: &installed.namespace,
        scope,
    };
    let (pulled, status) = try_move(&held, &latest_uri).await?;
    Ok(pulled.map(|pulled| outcome(Some(pulled), &status)))
}

/// What a merge starts from: the copy's base and its pushed revision, and the
/// package whose `latest` it merges onto. `None` for a copy with no pushed
/// revision.
struct Sides {
    base: Manifest,
    ours: Manifest,
    package: quilt_uri::S3PackageUri,
}

async fn sides<M: QuiltModel>(
    model: &M,
    installed: &quilt::InstalledPackage,
) -> Result<Option<Sides>, WatchError> {
    let lineage = model
        .get_installed_package_lineage(installed)
        .await
        .map_err(WatchError::Transient)?;
    let Some(ours_uri) = lineage
        .remote_uri
        .filter(|uri| lineage.commit.is_none() && !uri.hash.is_empty())
    else {
        return Ok(None);
    };
    let base_uri = quilt_uri::ManifestUri {
        hash: lineage.base_hash,
        ..ours_uri.clone()
    };
    Ok(Some(Sides {
        base: read(model, &base_uri).await?,
        ours: read(model, &ours_uri).await?,
        package: quilt_uri::S3PackageUri {
            revision: quilt_uri::RevisionPointer::Tag(quilt_uri::Tag::Latest),
            ..(&ours_uri).into()
        },
    }))
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

/// Move the copy to `merged`, now `latest`, and report what that brought into
/// the folder.
async fn move_to<M: QuiltModel>(
    held: &Held<'_, M>,
    merged: &quilt_uri::ManifestUri,
) -> Result<RefreshOutcome, WatchError> {
    let (pulled, status) = try_move(held, merged).await?;
    Ok(outcome(pulled, &status))
}

/// Move the copy to `target`, `latest`, as a pull moves a Behind copy, and walk
/// the folder after. `None` when edits to paths the other side changed block
/// it, which keeps the copy Diverged at its own revision; any other file in the
/// way pauses it as Diverged.
async fn try_move<M: QuiltModel>(
    held: &Held<'_, M>,
    target: &quilt_uri::ManifestUri,
) -> Result<
    (
        Option<quilt::flow::Pulled>,
        quilt::lineage::InstalledPackageStatus,
    ),
    WatchError,
> {
    let moved = {
        let _applying = held.aggregator.apply_guard(held.namespace);
        let _activity = held
            .aggregator
            .activity_guard(ActivityOp::Pull, held.namespace);
        held.model
            .locked_package_move_to(held.locked, target, held.scope.clone())
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
    match moved {
        Ok(pulled) => Ok((Some(pulled), status)),
        // Edits, which the tick commits and merges.
        Err(paths) if paths.iter().all(|path| status.changes.contains_key(path)) => {
            warn!(
                "autosync: {}: edits at {paths:?} block the move; left Diverged",
                held.namespace
            );
            Ok((None, status))
        }
        Err(paths) => {
            warn!(
                "autosync: {}: files in the way of latest at {paths:?}",
                held.namespace
            );
            Err(WatchError::Conflict(PausedReason::Diverged))
        }
    }
}

/// The copy after its move, `None` where edits blocked it, as the walk after
/// the move found it.
fn outcome(
    pulled: Option<quilt::flow::Pulled>,
    status: &quilt::lineage::InstalledPackageStatus,
) -> RefreshOutcome {
    RefreshOutcome {
        pulled: match pulled {
            Some(quilt::flow::Pulled::Applied(report)) => Some(report),
            Some(quilt::flow::Pulled::UpToDate) | None => None,
        },
        ..RefreshOutcome::observed(
            status.upstream_state,
            !status.changes.is_empty(),
            status_fingerprint(status),
        )
    }
}

/// [`classify_sync_err`]'s classification, which is never `Ok`.
fn sync_err(err: crate::engine::Error) -> WatchError {
    classify_sync_err(err).expect_err("an error classifies as one")
}
