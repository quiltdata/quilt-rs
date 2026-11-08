//! A run's outcome that did not become `latest`, published again onto it.

use std::collections::BTreeMap;
use std::path::Path;
use std::path::PathBuf;

use quilt_uri::ManifestUri;
use quilt_uri::Namespace;
use serde_json::Value;
use serde_json::json;

use super::agent_exec;
use crate::engine::Error;
use crate::engine::model::QuiltModel;
use crate::engine::quilt::manifest::Manifest;
use crate::engine::quilt::manifest::ManifestHeader;
use crate::engine::quilt::manifest::ManifestRow;
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
) -> Result<bool, Error> {
    let package = quilt_uri::S3PackageUri {
        revision: quilt_uri::RevisionPointer::Tag(quilt_uri::Tag::Latest),
        ..outcome.into()
    };
    let base = model.browse_remote_manifest(base).await?;
    let outcome = model.browse_remote_manifest(outcome).await?;
    let delta = delta(&base.rows, &outcome.rows);
    let claim = outcome
        .header
        .user_meta
        .as_ref()
        .map_or(Value::Null, |meta| meta["agent_exec"]["claim"].clone());
    for _ in 0..ATTEMPTS {
        let latest_uri = model.resolve_manifest_uri(&package).await?;
        let Manifest { header, rows } = model.browse_remote_manifest(&latest_uri).await?;
        let Some(user_meta) = header
            .user_meta
            .as_ref()
            .filter(|meta| agent_exec::holds_same_claim(head_meta, meta))
            .map(|meta| agent_exec::with_claim(meta, claim.clone()))
        else {
            warn!("tasks: {namespace}: latest no longer holds the claim; left Diverged");
            return Ok(false);
        };
        let Some(rows) = rebased_rows(&delta, rows) else {
            warn!("tasks: {namespace}: latest changed what the run changed; left Diverged");
            return Ok(false);
        };
        let rebased = Manifest {
            header: ManifestHeader {
                message: outcome.header.message.clone(),
                user_meta: Some(user_meta),
                ..header
            },
            rows,
        };
        if model
            .push_onto(&latest_uri, rebased)
            .await?
            .certified_latest
        {
            return Ok(true);
        }
    }
    warn!("tasks: {namespace}: latest kept moving; left Diverged");
    Ok(false)
}

/// An entry's `meta` as the top hash reads it: null and absent are `{}`.
fn entry_meta(row: &ManifestRow) -> Value {
    match &row.meta {
        None | Some(Value::Null) => json!({}),
        Some(meta @ Value::Object(_)) => meta.clone(),
        Some(meta) => json!({ "user_meta": meta }),
    }
}

fn same(a: Option<&ManifestRow>, b: Option<&ManifestRow>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => a.hash == b.hash && entry_meta(a) == entry_meta(b),
        _ => false,
    }
}

/// Each key whose row differs from `base` to `outcome`, with both rows.
type Delta<'a> = BTreeMap<&'a Path, (Option<&'a ManifestRow>, Option<&'a ManifestRow>)>;

fn delta<'a>(base: &'a [ManifestRow], outcome: &'a [ManifestRow]) -> Delta<'a> {
    let by_key = |rows: &'a [ManifestRow]| -> BTreeMap<&'a Path, &'a ManifestRow> {
        rows.iter()
            .map(|row| (row.logical_key.as_path(), row))
            .collect()
    };
    let (base, outcome) = (by_key(base), by_key(outcome));
    base.keys()
        .chain(outcome.keys())
        .map(|&key| (key, (base.get(key).copied(), outcome.get(key).copied())))
        .filter(|(_, (was, now))| !same(*was, *now))
        .collect()
}

/// `latest`'s rows with `delta` applied; `None` when `latest` changed a key of
/// the delta to a third row, or the result would hold a file where it holds a
/// folder.
fn rebased_rows(delta: &Delta, latest: Vec<ManifestRow>) -> Option<Vec<ManifestRow>> {
    let mut rebased: BTreeMap<PathBuf, ManifestRow> = latest
        .into_iter()
        .map(|row| (row.logical_key.clone(), row))
        .collect();
    for (&key, &(was, now)) in delta {
        let theirs = rebased.get(key);
        if same(theirs, now) {
            continue;
        }
        if !same(theirs, was) {
            return None;
        }
        match now {
            Some(row) => rebased.insert(key.to_path_buf(), row.clone()),
            None => rebased.remove(key),
        };
    }
    // Keys sort by component, so a key that is a folder of others sorts right
    // before one of them.
    let keys: Vec<&PathBuf> = rebased.keys().collect();
    if keys.windows(2).any(|pair| pair[1].starts_with(pair[0])) {
        return None;
    }
    Some(rebased.into_values().collect())
}
