//! A run's outcome that did not become `latest`, published again onto it.

use quilt_uri::ManifestUri;
use quilt_uri::Namespace;
use serde_json::Value;

use super::agent_exec;
use crate::engine::Error;
use crate::engine::model::QuiltModel;
use crate::engine::quilt;
use crate::engine::quilt::manifest::Manifest;
use crate::engine::quilt::manifest::ManifestHeader;
use crate::engine::telemetry::prelude::*;

/// Attempts in all, each from a fresh read of `latest`.
const ATTEMPTS: usize = 3;

/// Publish the run's own entry delta, from `base` to `outcome`, onto `latest`
/// as its child, while `latest` still holds the claim `head_meta` holds;
/// whether it became `latest`.
pub(super) async fn onto_latest<M: QuiltModel>(
    model: &M,
    namespace: &Namespace,
    base: &ManifestUri,
    outcome: &ManifestUri,
    head_meta: &Value,
    record: &super::Record,
) -> Result<bool, Error> {
    let package = quilt_uri::S3PackageUri {
        revision: quilt_uri::RevisionPointer::Tag(quilt_uri::Tag::Latest),
        ..outcome.into()
    };
    let base = model.browse_remote_manifest(base).await?;
    let outcome = model.browse_remote_manifest(outcome).await?;
    let claim = outcome
        .header
        .user_meta
        .as_ref()
        .map_or(Value::Null, |meta| meta["agent_exec"]["claim"].clone());
    for _ in 0..ATTEMPTS {
        let latest_uri = model.resolve_manifest_uri(&package).await?;
        let Manifest {
            header,
            rows: latest_rows,
        } = model.browse_remote_manifest(&latest_uri).await?;
        let Some(user_meta) = header
            .user_meta
            .as_ref()
            .filter(|meta| agent_exec::holds_same_claim(head_meta, meta))
            .map(|meta| agent_exec::with_claim(meta, claim.clone()))
        else {
            warn!("tasks: {namespace}: latest no longer holds the claim; left Diverged");
            return Ok(false);
        };
        let Ok(rows) = quilt::flow::merge_rows_onto(&base.rows, &outcome.rows, latest_rows.clone())
        else {
            warn!("tasks: {namespace}: latest changed what the run changed; left Diverged");
            return Ok(false);
        };
        let changes = crate::engine::record::changes_between(&latest_rows, &rows);
        let rebased = Manifest {
            header: ManifestHeader {
                message: outcome.header.message.clone(),
                user_meta: Some(user_meta),
                ..header
            },
            rows,
        };
        let pushed = model.push_onto(&latest_uri, rebased).await?;
        // Each upload is a revision in the bucket, `latest` or not.
        record(
            crate::engine::record::Draft::published(
                namespace,
                Some(&latest_uri.hash),
                &pushed.manifest_uri.hash,
                &changes,
            )
            .latest_or_not(pushed.certified_latest),
        );
        if pushed.certified_latest {
            return Ok(true);
        }
    }
    warn!("tasks: {namespace}: latest kept moving; left Diverged");
    Ok(false)
}
