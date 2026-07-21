//! `get_merge_data` / `get_commit_data` — commit-workflow queries for the Leptos UI.

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;

use serde::Serialize;
use tokio::sync::Mutex;
use tokio::sync::OnceCell;

use quilt_rs::io::remote::WORKFLOWS_CONFIG_KEY;
use quilt_rs::io::remote::WorkflowsConfig;
use quilt_rs::workflow::PackageCandidate;
use quilt_rs::workflow::RuleViolation;
use quilt_rs::workflow::WorkflowRules;
use quilt_rs::workflow::WorkflowValidationError;
use quilt_rs::workflow::validate_candidate_fields;
use quilt_uri::Host;
use quilt_uri::S3Uri;

use crate::Error;
use crate::model;
use crate::model::QuiltModel;
use crate::quilt;

use super::package_data::InstalledPackageEntryData;

// ── Merge data for Leptos UI ──

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeData {
    pub namespace: String,
    pub uri: Option<quilt_uri::S3PackageUri>,
}

async fn get_merge_data_from_model(
    m: &impl model::QuiltModel,
    tracing: &crate::telemetry::Telemetry,
    namespace: &quilt_uri::Namespace,
) -> Result<MergeData, Error> {
    let installed_package = m.get_installed_package(namespace).await?.ok_or_else(|| {
        Error::from(quilt::InstallPackageError::NotInstalled(
            namespace.to_owned(),
        ))
    })?;

    let lineage = m.get_installed_package_lineage(&installed_package).await?;

    let uri = lineage
        .remote_uri
        .as_ref()
        .map(quilt_uri::S3PackageUri::from);
    if let Some(host) = uri.as_ref().and_then(|u| u.catalog.as_ref()) {
        tracing.add_host(host);
    }

    Ok(MergeData {
        namespace: namespace.to_string(),
        uri,
    })
}

#[tauri::command]
pub async fn get_merge_data(
    m: tauri::State<'_, model::Model>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
    namespace: String,
) -> Result<MergeData, String> {
    let namespace: quilt_uri::Namespace = namespace
        .try_into()
        .map_err(|e: quilt_uri::UriError| e.to_string())?;

    get_merge_data_from_model(&*m, &tracing, &namespace)
        .await
        .map_err(|e| e.to_frontend_string())
}

// ── Commit data for Leptos UI ──

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitData {
    pub namespace: String,
    pub uri: Option<quilt_uri::S3PackageUri>,
    pub status: String,
    pub message: String,
    pub user_meta: String,
    pub user_meta_error: Option<String>,
    pub workflow: Option<CommitWorkflowData>,
    pub workflows: CommitWorkflows,
    pub entries: Vec<InstalledPackageEntryData>,
    pub ignored_count: usize,
    pub unmodified_count: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitWorkflowData {
    pub id: Option<String>,
    pub url: Option<String>,
    pub config_url: Option<String>,
}

/// A workflow declared under `workflows:` in the bucket's config, surfaced to
/// the commit dialog so the user can pick one. Distinct from
/// [`CommitWorkflowData`], which is the previous revision's stamped selection.
///
/// `metadata_schema_url` / `entries_schema_url` are catalog HTTPS links to the
/// schema objects the workflow declares, pre-formatted for the currently-known
/// catalog host — `None` when the workflow declares no such schema, or when
/// there is no catalog host to link against.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitWorkflowInfo {
    pub id: String,
    pub name: Option<String>,
    pub description: Option<String>,
    pub metadata_schema_url: Option<String>,
    pub entries_schema_url: Option<String>,
}

/// The bucket's workflow-selection situation, as the commit dialog should
/// present it. Splits the cases the backend can distinguish so the UI never
/// conflates "ungoverned bucket", "couldn't load the config", and "the config
/// is broken":
/// - `Available` — the bucket has a workflows config; carry its choices.
/// - `NotConfigured` — no config (or no remote); the bucket is ungoverned.
/// - `Unavailable` — a transient/network failure loading the config; a commit
///   will retry resolving the bucket default.
/// - `Invalid` — the config exists but is malformed (schema violation). Every
///   commit to this bucket will FAIL until it is fixed, so the UI must not
///   promise a fallback; `reason` names the violation succinctly.
#[derive(Serialize)]
#[serde(
    tag = "state",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum CommitWorkflows {
    Available {
        workflows: Vec<CommitWorkflowInfo>,
        default_workflow: Option<String>,
        is_workflow_required: bool,
        /// Catalog HTTPS link to the bucket's `.quilt/workflows/config.yml`
        /// object, or `None` when there is no catalog host to link against.
        config_url: Option<String>,
    },
    NotConfigured,
    Unavailable,
    Invalid {
        reason: String,
        /// Catalog HTTPS link to the bucket's `.quilt/workflows/config.yml`
        /// object, so the user can open the broken config to fix it. `None`
        /// when there is no catalog host (or bucket) to link against.
        config_url: Option<String>,
    },
}

/// Format the catalog HTTPS link for an S3 object, or `None` when there is no
/// catalog host to link against. Reuses [`S3Uri::display_for_host`] — the same
/// on-demand catalog-link formatting the rest of the app uses.
fn catalog_object_url(uri: &S3Uri, host: Option<&Host>) -> Option<String> {
    let host = host?;
    uri.display_for_host(host).ok().map(|u| u.to_string())
}

/// Catalog HTTPS link to a bucket's `.quilt/workflows/config.yml`, or `None`
/// when there is no catalog host or bucket to link against. Shared by the
/// `Available` and `Invalid` states so the config link is built one way.
fn config_object_url(host: Option<&Host>, bucket: Option<&str>) -> Option<String> {
    let bucket = bucket?;
    catalog_object_url(
        &S3Uri {
            bucket: bucket.to_string(),
            key: WORKFLOWS_CONFIG_KEY.to_string(),
            version: None,
        },
        host,
    )
}

/// Map a fetched workflows config to the UI's state. Shared by the commit
/// dialog and the pre-set-remote bucket preview so both present the same shape:
/// `Ok(Some)` → `Available`, `Ok(None)` → `NotConfigured`; an `Err` splits by
/// variant — a malformed config (`InvalidWorkflowsConfig`) → `Invalid` (commits
/// will fail until fixed), any other error → `Unavailable` (transient; logged,
/// never fatal, so the caller degrades gracefully).
fn workflows_config_to_commit_workflows(
    config: Result<Option<WorkflowsConfig>, Error>,
    host: Option<&Host>,
    bucket: Option<&str>,
) -> CommitWorkflows {
    match config {
        Ok(Some(config)) => {
            let config_url = config_object_url(host, bucket);
            // Resolve each workflow's schema links first — this borrow of
            // `config` ends here — so the strings can then be *moved* out of
            // `config.workflows` instead of cloned. A misconfigured `schemas`
            // section degrades to no links rather than dropping the dialog.
            let schema_urls: Vec<(Option<String>, Option<String>)> = config
                .workflows
                .iter()
                .map(|w| {
                    let schemas = config.schema_uris(&w.id);
                    (
                        schemas
                            .metadata_schema
                            .and_then(|uri| catalog_object_url(&uri, host)),
                        schemas
                            .entries_schema
                            .and_then(|uri| catalog_object_url(&uri, host)),
                    )
                })
                .collect();
            let workflows = config
                .workflows
                .into_iter()
                .zip(schema_urls)
                .map(
                    |(w, (metadata_schema_url, entries_schema_url))| CommitWorkflowInfo {
                        id: w.id,
                        name: w.name,
                        description: w.description,
                        metadata_schema_url,
                        entries_schema_url,
                    },
                )
                .collect();
            CommitWorkflows::Available {
                workflows,
                default_workflow: config.default_workflow,
                is_workflow_required: config.is_workflow_required,
                config_url,
            }
        }
        Ok(None) => CommitWorkflows::NotConfigured,
        // A malformed config is not a load failure — the file is present and
        // readable, it just violates the schema. Distinguish it by variant (not
        // by message text) so the UI can tell the user commits will fail until
        // the config is fixed, instead of promising a bucket-default fallback
        // that would actually error.
        Err(Error::Quilt(quilt::Error::RemoteCatalog(
            quilt::RemoteCatalogError::InvalidWorkflowsConfig(reason),
        ))) => CommitWorkflows::Invalid {
            reason,
            config_url: config_object_url(host, bucket),
        },
        Err(e) => {
            tracing::warn!(
                "Failed to load the bucket's workflows config; the caller will fall back to the bucket default: {e}"
            );
            CommitWorkflows::Unavailable
        }
    }
}

async fn get_commit_data_from_model(
    m: &impl model::QuiltModel,
    tracing: &crate::telemetry::Telemetry,
    namespace: &quilt_uri::Namespace,
) -> Result<CommitData, Error> {
    let installed_package = m.get_installed_package(namespace).await?.ok_or_else(|| {
        Error::from(quilt::InstallPackageError::NotInstalled(
            namespace.to_owned(),
        ))
    })?;

    let status = m
        .get_installed_package_status(&installed_package, None)
        .await?;

    let pkg_status_str = match status.upstream_state {
        quilt::lineage::UpstreamState::UpToDate => "up_to_date",
        quilt::lineage::UpstreamState::Ahead => "ahead",
        quilt::lineage::UpstreamState::Behind => "behind",
        quilt::lineage::UpstreamState::Diverged => "diverged",
        quilt::lineage::UpstreamState::Local => "local",
        quilt::lineage::UpstreamState::Error => "error",
    };

    let lineage = m.get_installed_package_lineage(&installed_package).await?;

    let typed_uri = lineage
        .remote_uri
        .as_ref()
        .map(quilt_uri::S3PackageUri::from);
    let origin_host = typed_uri.as_ref().and_then(|u| u.catalog.as_ref());
    if let Some(host) = origin_host {
        tracing.add_host(host);
    }

    // Build lookup maps for junky files
    let junky_map: std::collections::HashMap<_, _> = status
        .junky_changes
        .iter()
        .map(|(p, pat)| (p.clone(), pat.clone()))
        .collect();

    // Modified entries
    let mut entries_list = Vec::new();
    for (filename, change) in &status.changes {
        let (status_str, size) = match change {
            quilt::lineage::Change::Added(r) => ("added", r.size),
            quilt::lineage::Change::Modified(r) => ("modified", r.size),
            quilt::lineage::Change::Removed(r) => ("deleted", r.size),
        };
        entries_list.push(InstalledPackageEntryData {
            filename: filename.display().to_string(),
            size,
            status: status_str.to_string(),
            junky_pattern: junky_map.get(filename).cloned(),
            ignored_by: None,
            namespace: namespace.to_string(),
        });
        if entries_list.len() > 1000 {
            break;
        }
    }

    // Unmodified entries (from manifest, not changed)
    let manifest_entries = m.get_installed_package_records(&installed_package).await?;
    for (filename, row) in &manifest_entries {
        if status.changes.contains_key(filename) {
            continue;
        }
        entries_list.push(InstalledPackageEntryData {
            filename: filename.display().to_string(),
            size: row.size,
            status: if lineage.paths.contains_key(filename) {
                "pristine"
            } else {
                "remote"
            }
            .to_string(),
            junky_pattern: None,
            ignored_by: None,
            namespace: namespace.to_string(),
        });
        if entries_list.len() > 1000 {
            break;
        }
    }

    // Ignored files
    for (filename, pattern, size) in &status.ignored_files {
        entries_list.push(InstalledPackageEntryData {
            filename: filename.display().to_string(),
            size: *size,
            status: "pristine".to_string(),
            junky_pattern: None,
            ignored_by: Some(pattern.clone()),
            namespace: namespace.to_string(),
        });
        if entries_list.len() > 1000 {
            break;
        }
    }

    entries_list.sort_by(|a, b| a.filename.cmp(&b.filename));

    // Compute counts from the full source data, not the capped entries_list,
    // so the filter toolbar is shown even when the list is truncated.
    let ignored_count = status.ignored_files.len();
    let unmodified_count = manifest_entries
        .keys()
        .filter(|f| !status.changes.contains_key(*f))
        .count();

    // Generate commit message from changes
    let message = crate::commit_message::generate(&status.changes);

    // Load remote manifest for user_meta and workflow
    let (user_meta, user_meta_error, workflow) =
        match lineage.remote_uri.as_ref().filter(|r| !r.hash.is_empty()) {
            Some(remote_uri) => {
                let remote_manifest = m.browse_remote_manifest(remote_uri).await?;
                let (meta_value, meta_error) = match &remote_manifest.header.user_meta {
                    Some(meta) => match serde_json::to_string(meta) {
                        Ok(v) => (v, None),
                        Err(_) => (String::new(), Some("Failed to stringify meta".to_string())),
                    },
                    None => (String::new(), None),
                };
                let workflow = origin_host.and_then(|host| {
                    remote_manifest
                        .header
                        .workflow
                        .as_ref()
                        .map(|w| CommitWorkflowData {
                            id: w.id.as_ref().map(|id| id.id.clone()),
                            url: catalog_object_url(&w.config, Some(host)),
                            config_url: None,
                        })
                });
                (meta_value, meta_error, workflow)
            }
            None => (String::new(), None, None),
        };

    // Fetch the bucket's declared workflows for the commit dialog. A fetch
    // failure (or a package with no remote) must NOT fail the command — the
    // dialog still opens, and the UI presents the honest fallback state.
    let workflows = workflows_config_to_commit_workflows(
        m.get_workflows_config(&installed_package).await,
        origin_host,
        typed_uri.as_ref().map(|u| u.bucket.as_str()),
    );

    Ok(CommitData {
        namespace: namespace.to_string(),
        uri: typed_uri,
        status: pkg_status_str.to_string(),
        message,
        user_meta,
        user_meta_error,
        workflow,
        workflows,
        entries: entries_list,
        ignored_count,
        unmodified_count,
    })
}

#[tauri::command]
pub async fn get_commit_data(
    m: tauri::State<'_, model::Model>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
    namespace: String,
) -> Result<CommitData, String> {
    let namespace: quilt_uri::Namespace = namespace
        .try_into()
        .map_err(|e: quilt_uri::UriError| e.to_string())?;

    get_commit_data_from_model(&*m, &tracing, &namespace)
        .await
        .map_err(|e| e.to_frontend_string())
}

// ── Bucket workflows preview for the set-remote popup ──

async fn get_bucket_workflows_from_model(
    m: &impl model::QuiltModel,
    host: Option<Host>,
    bucket: &str,
) -> CommitWorkflows {
    let config = m.get_bucket_workflows_config(host.clone(), bucket).await;
    workflows_config_to_commit_workflows(config, host.as_ref(), Some(bucket))
}

/// Fetch a bucket's declared workflows before its remote is set, so the popup
/// can present the same tri-state (`Available` / `NotConfigured` /
/// `Unavailable`) the commit dialog uses. A fetch failure maps to
/// `Unavailable` rather than an error, so the popup still opens.
#[tauri::command]
pub async fn get_bucket_workflows(
    m: tauri::State<'_, model::Model>,
    host: Option<String>,
    bucket: String,
) -> Result<CommitWorkflows, String> {
    let host = match host {
        Some(host) => Some(quilt_uri::Host::from_str(&host).map_err(|e| e.to_string())?),
        None => None,
    };
    Ok(get_bucket_workflows_from_model(&*m, host, &bucket).await)
}

// ── Live commit-dialog validation ──
//
// The commit dialog validates the user's input against the selected workflow's
// rules as they type (debounced), showing advisory inline errors before the
// commit attempt. This is a convenience layer only — the commit-time gate in
// quilt-rs remains the authority and is unchanged; the buttons stay enabled.
//
// Two commands split the work so the per-keystroke path never touches the
// network: `load_workflow_rules` fetches + compiles a workflow's rules once and
// caches them; `validate_commit_candidate` validates {message, user_meta, name}
// against the CACHED rules with no I/O. The entries schema is deliberately NOT
// validated live — projecting a candidate's entries needs the built manifest's
// rows (the heavier flow machinery), so entries stay the commit gate's job.

/// Which commit-dialog input a [`CommitViolation`] belongs under, so the UI can
/// render each violation beneath the field the user must fix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ViolationField {
    Message,
    Metadata,
    Name,
}

/// A single advisory workflow violation for the commit dialog: the field it
/// applies to plus a human-readable message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitViolation {
    pub field: ViolationField,
    pub message: String,
}

/// Cache of compiled workflow rules, keyed by `(namespace, workflow_id)`, held
/// in Tauri-managed app-lifetime state (no eviction, no TTL). Because the cache
/// outlives any single dialog, a config.yml edited between two opens of the same
/// package's commit dialog would otherwise be served stale until app restart. To
/// stay honest, each dialog session refreshes on mount: the first
/// `ensure_loaded` of a session passes `refresh = true`, which drops every
/// cached entry for that namespace and re-fetches; later selection-change loads
/// within the same session pass `refresh = false` and stay cache-friendly. The
/// stored `Option` distinguishes "loaded, this workflow has rules" from "loaded,
/// no rules" (ungoverned / no config), so both are cached and neither re-fetches.
///
/// Each key maps to an `Arc<OnceCell<..>>` rather than the value directly, so
/// concurrent loads of the same key single-flight: the map lock is held only to
/// get-or-insert the cell, and the fetch runs inside `OnceCell::get_or_try_init`
/// with the map lock released — so a slow fetch never blocks the keystroke
/// [`validate`] path, and the same key is fetched exactly once even under
/// concurrent callers.
#[derive(Default)]
pub struct WorkflowRulesCache {
    rules: Mutex<HashMap<(String, String), RulesCell>>,
}

/// One cache slot: an `Arc<OnceCell<..>>` so concurrent loads of a key share a
/// single fetch. The inner `Option` is the fetched result — `Some(rules)` when
/// the workflow has rules, `None` when the bucket is ungoverned.
type RulesCell = Arc<OnceCell<Option<WorkflowRules>>>;

impl WorkflowRulesCache {
    /// Ensure the rules for `(namespace, workflow_id)` are cached, fetching them
    /// once on a miss. Returns whether the workflow has rules to validate
    /// against (`false` when the bucket is ungoverned). A cache hit performs no
    /// model calls at all, and concurrent misses on the same key share one
    /// fetch — the per-selection fetch runs exactly once.
    ///
    /// `refresh` (set on a dialog's first load) drops the namespace's cached
    /// entries first, so a config.yml change since the last dialog open is
    /// picked up rather than served stale from the app-lifetime cache.
    async fn ensure_loaded(
        &self,
        m: &impl QuiltModel,
        namespace: &str,
        workflow_id: &str,
        refresh: bool,
    ) -> Result<bool, Error> {
        let key = (namespace.to_string(), workflow_id.to_string());
        // Brief map lock: on refresh drop the namespace's cells (replacing them
        // with fresh, uninitialised ones on next access), then get-or-insert
        // this key's cell. The fetch itself happens below with the lock
        // released.
        let cell = {
            let mut guard = self.rules.lock().await;
            if refresh {
                guard.retain(|(ns, _), _| ns != namespace);
            }
            Arc::clone(guard.entry(key).or_default())
        };
        let rules = cell
            .get_or_try_init(|| async {
                let ns = quilt_uri::Namespace::try_from(namespace)?;
                let package = m
                    .get_installed_package(&ns)
                    .await?
                    .ok_or_else(|| Error::from(quilt::InstallPackageError::NotInstalled(ns)))?;
                m.get_workflow_rules(&package, workflow_id).await
            })
            .await?;
        Ok(rules.is_some())
    }

    /// Validate a candidate against the cached rules for
    /// `(namespace, workflow_id)`. Reads the cache only — no I/O, and never
    /// waits on an in-flight fetch (the cell is read via `get`, not
    /// `get_or_init`). When nothing is cached (rules not loaded yet, or the
    /// fetch is still in flight) or the workflow has no rules, there is nothing
    /// to validate against, so no violations are returned.
    async fn validate(
        &self,
        namespace: &str,
        workflow_id: &str,
        message: &str,
        user_meta: &str,
        name: &str,
    ) -> Vec<CommitViolation> {
        let key = (namespace.to_string(), workflow_id.to_string());
        let cell = {
            let guard = self.rules.lock().await;
            guard.get(&key).map(Arc::clone)
        };
        let Some(cell) = cell else {
            return Vec::new();
        };
        let Some(Some(rules)) = cell.get() else {
            return Vec::new();
        };
        validate_candidate(rules, message, user_meta, name)
    }
}

/// Validate one candidate's fields against already-fetched rules, mapping the
/// pure gate's outcome to per-field [`CommitViolation`]s. Unparseable user
/// metadata is surfaced as the metadata violation while the message and handle
/// checks still run (the commit path validates every field regardless of
/// whether the metadata parses); the backend does no network I/O here, so
/// parsing server-side is cheap and keeps one parse authority.
fn validate_candidate(
    rules: &WorkflowRules,
    message: &str,
    user_meta: &str,
    name: &str,
) -> Vec<CommitViolation> {
    // Empty metadata validates as `{}` (the gate's default for absent metadata);
    // a non-empty value must parse as JSON or it is itself the violation.
    let (meta_value, parse_violation) = if user_meta.trim().is_empty() {
        (None, None)
    } else {
        match serde_json::from_str::<serde_json::Value>(user_meta) {
            Ok(value) => (Some(value), None),
            Err(err) => (
                None,
                Some(CommitViolation {
                    field: ViolationField::Metadata,
                    message: format!("Metadata is not valid JSON: {err}"),
                }),
            ),
        }
    };

    let candidate = PackageCandidate {
        name,
        message: Some(message),
        user_meta: meta_value.as_ref(),
        entries: &[],
    };
    let mut violations = match validate_candidate_fields(rules, &candidate) {
        Ok(()) => Vec::new(),
        Err(err) => violations_from_error(&err),
    };
    if let Some(parse_violation) = parse_violation {
        // The schema check ran against `{}` in place of the unparseable text,
        // so any metadata-schema violation it produced is misleading — the
        // parse error is the only honest metadata feedback.
        violations.retain(|violation| violation.field != ViolationField::Metadata);
        violations.insert(0, parse_violation);
    }
    violations
}

/// Map a [`WorkflowValidationError`] to per-field advisory violations. Rule
/// failures route by kind; a misconfigured `handle_pattern` lands under the name
/// field, and any schema-compilation problem under metadata (the entries schema
/// is never compiled on this path, so its hard errors cannot occur).
fn violations_from_error(err: &WorkflowValidationError) -> Vec<CommitViolation> {
    match err {
        WorkflowValidationError::Rejected(violations) => violations
            .list
            .iter()
            .map(|violation| CommitViolation {
                field: match violation {
                    RuleViolation::MessageRequired | RuleViolation::WorkflowRequired => {
                        ViolationField::Message
                    }
                    RuleViolation::HandleMismatch { .. } => ViolationField::Name,
                    RuleViolation::MetadataInvalid(_) | RuleViolation::EntriesInvalid(_) => {
                        ViolationField::Metadata
                    }
                },
                message: violation.to_string(),
            })
            .collect(),
        WorkflowValidationError::InvalidHandlePattern { .. } => vec![CommitViolation {
            field: ViolationField::Name,
            message: err.to_string(),
        }],
        _ => vec![CommitViolation {
            field: ViolationField::Metadata,
            message: err.to_string(),
        }],
    }
}

/// Fetch and cache the selected workflow's rules for live validation. Called
/// when the workflow selection changes; the fetch runs once per
/// `(namespace, workflow)` and subsequent calls hit the cache. Returns whether
/// the workflow has rules the dialog can validate against.
///
/// `refresh` is set on the dialog's first load of a session to drop the
/// namespace's cached entries and re-fetch, so a config.yml change since the
/// last open is picked up (the cache is app-lifetime state).
#[tauri::command]
pub async fn load_workflow_rules(
    m: tauri::State<'_, model::Model>,
    cache: tauri::State<'_, WorkflowRulesCache>,
    namespace: String,
    workflow_id: String,
    refresh: bool,
) -> Result<bool, String> {
    cache
        .ensure_loaded(&*m, &namespace, &workflow_id, refresh)
        .await
        .map_err(|e| e.to_frontend_string())
}

/// Validate the current commit-dialog input against the cached rules for the
/// selected workflow. Pure cache read — no network I/O — so it is safe on the
/// per-keystroke (debounced) path. Returns the advisory violations, routed per
/// field; an empty list means the input satisfies the workflow (or no rules are
/// loaded yet).
#[tauri::command]
pub async fn validate_commit_candidate(
    cache: tauri::State<'_, WorkflowRulesCache>,
    namespace: String,
    workflow_id: String,
    message: String,
    user_meta: String,
    name: String,
) -> Result<Vec<CommitViolation>, String> {
    Ok(cache
        .validate(&namespace, &workflow_id, &message, &user_meta, &name)
        .await)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::commands::test_support::*;
    use crate::model::mocks;

    #[tokio::test]
    async fn test_get_merge_data() -> Result<(), String> {
        let mut model = mocks::create();
        mocks::mock_installed_package(&mut model);
        let tracing = crate::telemetry::Telemetry::default();
        let namespace = ("foo", "bar").into();

        let data = get_merge_data_from_model(&model, &tracing, &namespace)
            .await
            .map_err(|e| e.to_string())?;

        assert_eq!(data.namespace, "foo/bar");
        let uri = data.uri.as_ref().expect("URI present");
        assert_eq!(uri.bucket, "quilt-example");
        assert_eq!(catalog_host(&data.uri).as_deref(), Some("test.quilt.dev"));
        Ok(())
    }

    #[tokio::test]
    async fn test_get_merge_data_not_installed() {
        let mut model = mocks::create();
        model.expect_get_installed_package().returning(|_| Ok(None));
        let tracing = crate::telemetry::Telemetry::default();
        let namespace = ("missing", "package").into();

        let result = get_merge_data_from_model(&model, &tracing, &namespace).await;
        assert!(result.is_err());
    }

    // ── Commit data tests ──

    #[tokio::test]
    async fn test_get_commit_data() -> Result<(), String> {
        let mut model = mocks::create();
        mocks::mock_installed_package(&mut model);
        let tracing = crate::telemetry::Telemetry::default();
        let namespace = ("foo", "bar").into();

        let data = get_commit_data_from_model(&model, &tracing, &namespace)
            .await
            .map_err(|e| e.to_string())?;

        assert_eq!(data.namespace, "foo/bar");
        let uri = data.uri.as_ref().expect("URI present");
        assert_eq!(uri.bucket, "quilt-example");
        assert_eq!(catalog_host(&data.uri).as_deref(), Some("test.quilt.dev"));
        Ok(())
    }

    #[tokio::test]
    async fn test_get_commit_data_not_installed() {
        let mut model = mocks::create();
        model.expect_get_installed_package().returning(|_| Ok(None));
        let tracing = crate::telemetry::Telemetry::default();
        let namespace = ("missing", "package").into();

        let result = get_commit_data_from_model(&model, &tracing, &namespace).await;
        assert!(result.is_err());
    }

    // (Adapted from pages/commit.rs: test_workflow_with_value)

    #[tokio::test]
    async fn test_get_commit_data_with_workflow() -> Result<(), String> {
        let mut model = mocks::create();

        let remote_manifest = quilt_uri::ManifestUri {
            bucket: "quilt-example".to_string(),
            namespace: ("foo", "bar").into(),
            hash: "abcdef".to_string(),
            origin: Some("test.quilt.dev".parse().unwrap()),
        };
        model
            .expect_get_installed_package()
            .returning(move |_| Ok(Some(make_installed_package(("foo", "bar")))));
        model
            .expect_get_installed_package_lineage()
            .returning(move |_| {
                Ok(quilt::lineage::PackageLineage::from_remote(
                    remote_manifest.clone(),
                    remote_manifest.hash.clone(),
                ))
            });
        let status = Ok(quilt::lineage::InstalledPackageStatus::default());
        model
            .expect_get_installed_package_status()
            .return_once(move |_, _| status);
        model
            .expect_get_installed_package_records()
            .returning(|_| Ok(std::collections::BTreeMap::new()));
        model.expect_get_workflows_config().returning(|_| Ok(None));
        // Return a manifest with workflow data
        model.expect_browse_remote_manifest().returning(|_| {
            let config_uri = quilt_uri::S3Uri {
                bucket: "quilt-example".to_string(),
                key: ".quilt/workflows/config.yaml".to_string(),
                version: None,
            };
            Ok(quilt::manifest::Manifest {
                header: quilt::manifest::ManifestHeader {
                    version: "v0".to_string(),
                    message: None,
                    user_meta: None,
                    workflow: Some(quilt::manifest::Workflow {
                        config: config_uri,
                        id: Some(quilt::manifest::WorkflowId {
                            id: "gamma".to_string(),
                            schemas: std::collections::BTreeMap::new(),
                        }),
                    }),
                },
                rows: Vec::new(),
            })
        });

        let tracing = crate::telemetry::Telemetry::default();
        let namespace = ("foo", "bar").into();

        let data = get_commit_data_from_model(&model, &tracing, &namespace)
            .await
            .map_err(|e| e.to_string())?;

        assert!(data.workflow.is_some());
        let workflow = data.workflow.unwrap();
        assert_eq!(workflow.id, Some("gamma".to_string()));
        assert!(workflow.url.is_some());
        Ok(())
    }

    // (Adapted from pages/commit.rs: test_workflow_null_checked)

    #[tokio::test]
    async fn test_get_commit_data_workflow_null_id() -> Result<(), String> {
        let mut model = mocks::create();

        let remote_manifest = quilt_uri::ManifestUri {
            bucket: "quilt-example".to_string(),
            namespace: ("foo", "bar").into(),
            hash: "abcdef".to_string(),
            origin: Some("test.quilt.dev".parse().unwrap()),
        };
        model
            .expect_get_installed_package()
            .returning(move |_| Ok(Some(make_installed_package(("foo", "bar")))));
        model
            .expect_get_installed_package_lineage()
            .returning(move |_| {
                Ok(quilt::lineage::PackageLineage::from_remote(
                    remote_manifest.clone(),
                    remote_manifest.hash.clone(),
                ))
            });
        let status = Ok(quilt::lineage::InstalledPackageStatus::default());
        model
            .expect_get_installed_package_status()
            .return_once(move |_, _| status);
        model
            .expect_get_installed_package_records()
            .returning(|_| Ok(std::collections::BTreeMap::new()));
        model.expect_get_workflows_config().returning(|_| Ok(None));
        // Workflow exists but has no ID (null/checked state)
        model.expect_browse_remote_manifest().returning(|_| {
            let config_uri = quilt_uri::S3Uri {
                bucket: "quilt-example".to_string(),
                key: ".quilt/workflows/config.yaml".to_string(),
                version: None,
            };
            Ok(quilt::manifest::Manifest {
                header: quilt::manifest::ManifestHeader {
                    version: "v0".to_string(),
                    message: None,
                    user_meta: None,
                    workflow: Some(quilt::manifest::Workflow {
                        config: config_uri,
                        id: None,
                    }),
                },
                rows: Vec::new(),
            })
        });

        let tracing = crate::telemetry::Telemetry::default();
        let namespace = ("foo", "bar").into();

        let data = get_commit_data_from_model(&model, &tracing, &namespace)
            .await
            .map_err(|e| e.to_string())?;

        assert!(data.workflow.is_some());
        let workflow = data.workflow.unwrap();
        assert!(workflow.id.is_none());
        assert!(workflow.url.is_some());
        Ok(())
    }

    // (Adapted from pages/commit.rs: test_workflow_not_available)

    #[tokio::test]
    async fn test_get_commit_data_no_workflow() -> Result<(), String> {
        let mut model = mocks::create();

        let remote_manifest = quilt_uri::ManifestUri {
            bucket: "quilt-example".to_string(),
            namespace: ("foo", "bar").into(),
            hash: "abcdef".to_string(),
            origin: Some("test.quilt.dev".parse().unwrap()),
        };
        model
            .expect_get_installed_package()
            .returning(move |_| Ok(Some(make_installed_package(("foo", "bar")))));
        model
            .expect_get_installed_package_lineage()
            .returning(move |_| {
                Ok(quilt::lineage::PackageLineage::from_remote(
                    remote_manifest.clone(),
                    remote_manifest.hash.clone(),
                ))
            });
        let status = Ok(quilt::lineage::InstalledPackageStatus::default());
        model
            .expect_get_installed_package_status()
            .return_once(move |_, _| status);
        model
            .expect_get_installed_package_records()
            .returning(|_| Ok(std::collections::BTreeMap::new()));
        model.expect_get_workflows_config().returning(|_| Ok(None));
        // No workflow in manifest
        model.expect_browse_remote_manifest().returning(|_| {
            Ok(quilt::manifest::Manifest {
                header: quilt::manifest::ManifestHeader {
                    version: "v0".to_string(),
                    message: None,
                    user_meta: None,
                    workflow: None,
                },
                rows: Vec::new(),
            })
        });

        let tracing = crate::telemetry::Telemetry::default();
        let namespace = ("foo", "bar").into();

        let data = get_commit_data_from_model(&model, &tracing, &namespace)
            .await
            .map_err(|e| e.to_string())?;

        assert!(data.workflow.is_none());
        Ok(())
    }

    // ── Bucket workflow list (the `workflows` / `default_workflow` /
    //    `is_workflow_required` fields) ──

    /// Set up the model mocks shared by the workflow-list tests: an installed
    /// package with a remote whose manifest carries no workflow stamp. The
    /// caller wires up `expect_get_workflows_config`.
    fn base_commit_model() -> crate::model::MockQuiltModel {
        let mut model = mocks::create();

        let remote_manifest = quilt_uri::ManifestUri {
            bucket: "quilt-example".to_string(),
            namespace: ("foo", "bar").into(),
            hash: "abcdef".to_string(),
            origin: Some("test.quilt.dev".parse().unwrap()),
        };
        model
            .expect_get_installed_package()
            .returning(move |_| Ok(Some(make_installed_package(("foo", "bar")))));
        model
            .expect_get_installed_package_lineage()
            .returning(move |_| {
                Ok(quilt::lineage::PackageLineage::from_remote(
                    remote_manifest.clone(),
                    remote_manifest.hash.clone(),
                ))
            });
        model
            .expect_get_installed_package_status()
            .returning(|_, _| Ok(quilt::lineage::InstalledPackageStatus::default()));
        model
            .expect_get_installed_package_records()
            .returning(|_| Ok(std::collections::BTreeMap::new()));
        model.expect_browse_remote_manifest().returning(|_| {
            Ok(quilt::manifest::Manifest {
                header: quilt::manifest::ManifestHeader {
                    version: "v0".to_string(),
                    message: None,
                    user_meta: None,
                    workflow: None,
                },
                rows: Vec::new(),
            })
        });
        model
    }

    #[tokio::test]
    async fn test_get_commit_data_workflows_available() -> Result<(), String> {
        let mut model = base_commit_model();
        // A sandbox-shaped config: multiple workflows, a declared default, and
        // the required flag set true.
        let yaml: serde_yaml::Value = serde_yaml::from_str(
            r#"
version: "1"
is_workflow_required: true
default_workflow: dummy
workflows:
  dummy:
    name: Dummy workflow
    description: Do nothing.
  alpha:
    name: Alpha
    description: First workflow.
"#,
        )
        .map_err(|e| e.to_string())?;
        let config =
            quilt::io::remote::WorkflowsConfig::from_yaml(&yaml).map_err(|e| e.to_string())?;
        model
            .expect_get_workflows_config()
            .return_once(move |_| Ok(Some(config)));

        let tracing = crate::telemetry::Telemetry::default();
        let namespace = ("foo", "bar").into();

        let data = get_commit_data_from_model(&model, &tracing, &namespace)
            .await
            .map_err(|e| e.to_string())?;

        // `Ok(Some(cfg))` → Available, carrying the list/default/required flag.
        let CommitWorkflows::Available {
            workflows,
            default_workflow,
            is_workflow_required,
            config_url,
        } = data.workflows
        else {
            return Err("expected Available".to_string());
        };
        assert_eq!(default_workflow, Some("dummy".to_string()));
        assert!(is_workflow_required);
        let ids: Vec<_> = workflows.iter().map(|w| w.id.as_str()).collect();
        assert_eq!(ids, vec!["dummy", "alpha"]);
        assert_eq!(workflows[0].name.as_deref(), Some("Dummy workflow"));
        assert_eq!(workflows[0].description.as_deref(), Some("Do nothing."));
        // The package has a catalog host (test.quilt.dev) and remote bucket
        // (quilt-example), so the config object gets a catalog link. These
        // workflows declare no schemas, so their schema links stay None.
        assert_eq!(
            config_url.as_deref(),
            Some("https://test.quilt.dev/b/quilt-example/tree/.quilt/workflows/config.yml")
        );
        assert!(
            workflows
                .iter()
                .all(|w| w.metadata_schema_url.is_none() && w.entries_schema_url.is_none())
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_get_commit_data_workflows_not_configured() -> Result<(), String> {
        // `Ok(None)` (no config in the bucket) → NotConfigured: the bucket is
        // ungoverned, distinct from a fetch failure.
        let mut model = base_commit_model();
        model.expect_get_workflows_config().returning(|_| Ok(None));

        let tracing = crate::telemetry::Telemetry::default();
        let namespace = ("foo", "bar").into();

        let data = get_commit_data_from_model(&model, &tracing, &namespace)
            .await
            .map_err(|e| e.to_string())?;

        assert!(matches!(data.workflows, CommitWorkflows::NotConfigured));
        Ok(())
    }

    #[tokio::test]
    async fn test_get_commit_data_workflows_unavailable() -> Result<(), String> {
        // `Err(_)` must NOT fail get_commit_data: the dialog still opens, and
        // the state is Unavailable so the UI falls back to the bucket default.
        let mut model = base_commit_model();
        model.expect_get_workflows_config().returning(|_| {
            Err(Error::from(quilt::InstallPackageError::NotInstalled(
                ("foo", "bar").into(),
            )))
        });

        let tracing = crate::telemetry::Telemetry::default();
        let namespace = ("foo", "bar").into();

        let data = get_commit_data_from_model(&model, &tracing, &namespace)
            .await
            .map_err(|e| e.to_string())?;

        assert!(matches!(data.workflows, CommitWorkflows::Unavailable));
        Ok(())
    }

    /// The tagged JSON `CommitWorkflows` serializes to is the wire contract the
    /// UI mirror (`quilt_sync_ui::commands::CommitWorkflows`) deserializes. If
    /// these strings drift, the commit dialog silently loses the workflow list.
    #[test]
    fn commit_workflows_wire_form_is_verbatim() {
        let available = CommitWorkflows::Available {
            workflows: vec![CommitWorkflowInfo {
                id: "alpha".to_string(),
                name: Some("Alpha".to_string()),
                description: None,
                metadata_schema_url: Some("https://catalog/b/bucket/tree/meta.json".to_string()),
                entries_schema_url: None,
            }],
            default_workflow: Some("alpha".to_string()),
            is_workflow_required: true,
            config_url: Some(
                "https://catalog/b/bucket/tree/.quilt/workflows/config.yml".to_string(),
            ),
        };
        assert_eq!(
            serde_json::to_value(&available).unwrap(),
            serde_json::json!({
                "state": "available",
                "workflows": [{
                    "id": "alpha",
                    "name": "Alpha",
                    "description": null,
                    "metadataSchemaUrl": "https://catalog/b/bucket/tree/meta.json",
                    "entriesSchemaUrl": null,
                }],
                "defaultWorkflow": "alpha",
                "isWorkflowRequired": true,
                "configUrl": "https://catalog/b/bucket/tree/.quilt/workflows/config.yml",
            })
        );
        assert_eq!(
            serde_json::to_value(CommitWorkflows::NotConfigured).unwrap(),
            serde_json::json!({"state": "notConfigured"})
        );
        assert_eq!(
            serde_json::to_value(CommitWorkflows::Unavailable).unwrap(),
            serde_json::json!({"state": "unavailable"})
        );
        assert_eq!(
            serde_json::to_value(CommitWorkflows::Invalid {
                reason: "bad schema".to_string(),
                config_url: Some(
                    "https://catalog/b/bucket/tree/.quilt/workflows/config.yml".to_string(),
                ),
            })
            .unwrap(),
            serde_json::json!({
                "state": "invalid",
                "reason": "bad schema",
                "configUrl": "https://catalog/b/bucket/tree/.quilt/workflows/config.yml",
            })
        );
    }

    /// A malformed workflows config (`InvalidWorkflowsConfig`) must map to the
    /// distinct `Invalid` case carrying the reason — NOT `Unavailable` — so the
    /// dialog can warn that commits will fail until the config is fixed.
    #[tokio::test]
    async fn test_get_commit_data_workflows_invalid() -> Result<(), String> {
        let mut model = base_commit_model();
        model.expect_get_workflows_config().returning(|_| {
            Err(Error::from(quilt::Error::RemoteCatalog(
                quilt::RemoteCatalogError::InvalidWorkflowsConfig(
                    "workflows/config.yml does not satisfy the workflows config schema".to_string(),
                ),
            )))
        });

        let tracing = crate::telemetry::Telemetry::default();
        let namespace = ("foo", "bar").into();

        let data = get_commit_data_from_model(&model, &tracing, &namespace)
            .await
            .map_err(|e| e.to_string())?;

        let CommitWorkflows::Invalid { reason, config_url } = data.workflows else {
            return Err("expected Invalid".to_string());
        };
        assert!(
            reason.contains("does not satisfy the workflows config schema"),
            "reason must carry the violation, got: {reason}"
        );
        // The malformed-config notice carries a link to the config object so
        // the user can open and fix it (catalog host + remote bucket in scope).
        assert_eq!(
            config_url.as_deref(),
            Some("https://test.quilt.dev/b/quilt-example/tree/.quilt/workflows/config.yml")
        );
        Ok(())
    }

    /// A transient/network failure (an S3 error) keeps mapping to `Unavailable`
    /// — a soft "couldn't load", distinct from the malformed-config case.
    #[tokio::test]
    async fn test_get_commit_data_workflows_unavailable_on_s3_error() -> Result<(), String> {
        let mut model = base_commit_model();
        model.expect_get_workflows_config().returning(|_| {
            Err(Error::from(quilt::Error::S3(quilt::S3Error::new(
                quilt::S3ErrorKind::GetObject("timeout".to_string()),
            ))))
        });

        let tracing = crate::telemetry::Telemetry::default();
        let namespace = ("foo", "bar").into();

        let data = get_commit_data_from_model(&model, &tracing, &namespace)
            .await
            .map_err(|e| e.to_string())?;

        assert!(matches!(data.workflows, CommitWorkflows::Unavailable));
        Ok(())
    }

    /// The same malformed-config discrimination on the pre-set-remote preview
    /// path (`get_bucket_workflows`): `Invalid`, not `Unavailable`.
    #[tokio::test]
    async fn test_get_bucket_workflows_invalid() -> Result<(), String> {
        let mut model = mocks::create();
        model
            .expect_get_bucket_workflows_config()
            .returning(|_, _| {
                Err(Error::from(quilt::Error::RemoteCatalog(
                    quilt::RemoteCatalogError::InvalidWorkflowsConfig(
                        "workflows/config.yml does not satisfy the workflows config schema"
                            .to_string(),
                    ),
                )))
            });

        let workflows = get_bucket_workflows_from_model(&model, None, "my-bucket").await;
        let CommitWorkflows::Invalid { reason, config_url } = workflows else {
            return Err("expected Invalid".to_string());
        };
        assert!(reason.contains("does not satisfy the workflows config schema"));
        // No catalog host was passed for this preview, so there is nothing to
        // link against and the config link is absent.
        assert!(config_url.is_none());
        Ok(())
    }

    // ── Bucket workflows preview (set-remote popup) ──

    #[tokio::test]
    async fn test_get_bucket_workflows_available() -> Result<(), String> {
        // `Ok(Some(cfg))` → Available, carrying the bucket's declared workflows.
        let mut model = mocks::create();
        let yaml: serde_yaml::Value = serde_yaml::from_str(
            r#"
version: "1"
is_workflow_required: true
default_workflow: dummy
workflows:
  dummy:
    name: Dummy workflow
    description: Do nothing.
  alpha:
    name: Alpha
    description: First workflow.
"#,
        )
        .map_err(|e| e.to_string())?;
        let config =
            quilt::io::remote::WorkflowsConfig::from_yaml(&yaml).map_err(|e| e.to_string())?;
        model
            .expect_get_bucket_workflows_config()
            .return_once(move |_, _| Ok(Some(config)));

        let workflows = get_bucket_workflows_from_model(&model, None, "my-bucket").await;

        let CommitWorkflows::Available {
            workflows,
            default_workflow,
            is_workflow_required,
            config_url,
        } = workflows
        else {
            return Err("expected Available".to_string());
        };
        assert_eq!(default_workflow, Some("dummy".to_string()));
        assert!(is_workflow_required);
        let ids: Vec<_> = workflows.iter().map(|w| w.id.as_str()).collect();
        assert_eq!(ids, vec!["dummy", "alpha"]);
        // No catalog host was passed for this preview, so there is nothing to
        // link against and the config link is absent.
        assert!(config_url.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn test_get_bucket_workflows_available_formats_catalog_links() -> Result<(), String> {
        // A config declaring per-workflow schemas, previewed against a catalog
        // host: the config object and each declared schema get a catalog HTTPS
        // link. The schema objects live under their own bucket, taken from the
        // `schemas` section's URLs.
        let mut model = mocks::create();
        let yaml: serde_yaml::Value = serde_yaml::from_str(
            r#"
version: "1"
default_workflow: alpha
workflows:
  alpha:
    name: Alpha
    metadata_schema: meta
    entries_schema: entries
  bare:
    name: Bare
schemas:
  meta:
    url: s3://schemas-bucket/meta.json
  entries:
    url: s3://schemas-bucket/entries.json
"#,
        )
        .map_err(|e| e.to_string())?;
        let config =
            quilt::io::remote::WorkflowsConfig::from_yaml(&yaml).map_err(|e| e.to_string())?;
        model
            .expect_get_bucket_workflows_config()
            .return_once(move |_, _| Ok(Some(config)));

        let host: quilt_uri::Host = "test.quilt.dev".parse().map_err(|_| "bad host")?;
        let workflows = get_bucket_workflows_from_model(&model, Some(host), "my-bucket").await;

        let CommitWorkflows::Available {
            workflows,
            config_url,
            ..
        } = workflows
        else {
            return Err("expected Available".to_string());
        };
        assert_eq!(
            config_url.as_deref(),
            Some("https://test.quilt.dev/b/my-bucket/tree/.quilt/workflows/config.yml")
        );
        // `alpha` declares both schemas; each is linked against its own bucket.
        let alpha = &workflows[0];
        assert_eq!(
            alpha.metadata_schema_url.as_deref(),
            Some("https://test.quilt.dev/b/schemas-bucket/tree/meta.json")
        );
        assert_eq!(
            alpha.entries_schema_url.as_deref(),
            Some("https://test.quilt.dev/b/schemas-bucket/tree/entries.json")
        );
        // `bare` declares neither schema, so it has no schema links.
        let bare = &workflows[1];
        assert!(bare.metadata_schema_url.is_none() && bare.entries_schema_url.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn test_get_bucket_workflows_not_configured() {
        // `Ok(None)` (ungoverned bucket) → NotConfigured.
        let mut model = mocks::create();
        model
            .expect_get_bucket_workflows_config()
            .returning(|_, _| Ok(None));

        let workflows = get_bucket_workflows_from_model(&model, None, "my-bucket").await;
        assert!(matches!(workflows, CommitWorkflows::NotConfigured));
    }

    #[tokio::test]
    async fn test_get_bucket_workflows_unavailable() {
        // `Err(_)` must NOT fail the command: the popup still opens with the
        // Unavailable fallback state.
        let mut model = mocks::create();
        model
            .expect_get_bucket_workflows_config()
            .returning(|_, _| {
                Err(Error::from(quilt::InstallPackageError::NotInstalled(
                    ("foo", "bar").into(),
                )))
            });

        let workflows = get_bucket_workflows_from_model(&model, None, "my-bucket").await;
        assert!(matches!(workflows, CommitWorkflows::Unavailable));
    }

    // ── Live commit-dialog validation ──

    fn strict_rules() -> WorkflowRules {
        WorkflowRules {
            handle_pattern: Some("^team/".to_string()),
            is_message_required: true,
            metadata_schema: Some(serde_json::json!({
                "type": "object",
                "required": ["owner"],
            })),
            entries_schema: None,
        }
    }

    /// The rules cache fetches once per `(namespace, workflow)` and serves every
    /// later load from memory: the `.times(1)` expectations on both model calls
    /// fail if the second `ensure_loaded` re-fetches.
    #[tokio::test]
    async fn workflow_rules_cache_fetches_once_then_hits_cache() -> Result<(), String> {
        let mut model = mocks::create();
        model
            .expect_get_installed_package()
            .times(1)
            .returning(|_| Ok(Some(make_installed_package(("foo", "bar")))));
        model
            .expect_get_workflow_rules()
            .times(1)
            .returning(|_, _| Ok(Some(strict_rules())));

        let cache = WorkflowRulesCache::default();
        assert!(
            cache
                .ensure_loaded(&model, "foo/bar", "wf", false)
                .await
                .map_err(|e| e.to_string())?
        );
        // Second load hits the cache — no further model calls (enforced above).
        assert!(
            cache
                .ensure_loaded(&model, "foo/bar", "wf", false)
                .await
                .map_err(|e| e.to_string())?
        );
        Ok(())
    }

    /// A dialog re-open passes `refresh = true` on its first load, which must
    /// drop the namespace's cached entry and re-fetch — so a config.yml change
    /// since the last open is picked up. The `.times(2)` expectations fail if
    /// the refresh silently serves the stale cache.
    #[tokio::test]
    async fn workflow_rules_cache_refresh_refetches() -> Result<(), String> {
        let mut model = mocks::create();
        model
            .expect_get_installed_package()
            .times(2)
            .returning(|_| Ok(Some(make_installed_package(("foo", "bar")))));
        model
            .expect_get_workflow_rules()
            .times(2)
            .returning(|_, _| Ok(Some(strict_rules())));

        let cache = WorkflowRulesCache::default();
        assert!(
            cache
                .ensure_loaded(&model, "foo/bar", "wf", false)
                .await
                .map_err(|e| e.to_string())?
        );
        // Second dialog session: refresh drops the entry and re-fetches.
        assert!(
            cache
                .ensure_loaded(&model, "foo/bar", "wf", true)
                .await
                .map_err(|e| e.to_string())?
        );
        Ok(())
    }

    /// Two concurrent `ensure_loaded` calls for the same key must single-flight:
    /// the cell's `get_or_try_init` runs the fetch once, so the `.times(1)`
    /// expectations hold even though both callers miss the cache together.
    #[tokio::test]
    async fn workflow_rules_cache_single_flights_concurrent_loads() -> Result<(), String> {
        let mut model = mocks::create();
        model
            .expect_get_installed_package()
            .times(1)
            .returning(|_| Ok(Some(make_installed_package(("foo", "bar")))));
        model
            .expect_get_workflow_rules()
            .times(1)
            .returning(|_, _| Ok(Some(strict_rules())));

        let cache = WorkflowRulesCache::default();
        let (a, b) = tokio::join!(
            cache.ensure_loaded(&model, "foo/bar", "wf", false),
            cache.ensure_loaded(&model, "foo/bar", "wf", false),
        );
        assert!(a.map_err(|e| e.to_string())?);
        assert!(b.map_err(|e| e.to_string())?);
        Ok(())
    }

    /// An ungoverned package (no rules) caches the negative result and reports
    /// no rules to validate against; validation is then a clean no-op.
    #[tokio::test]
    async fn workflow_rules_cache_ungoverned_is_clean_no_op() -> Result<(), String> {
        let mut model = mocks::create();
        model
            .expect_get_installed_package()
            .returning(|_| Ok(Some(make_installed_package(("foo", "bar")))));
        model
            .expect_get_workflow_rules()
            .times(1)
            .returning(|_, _| Ok(None));

        let cache = WorkflowRulesCache::default();
        // No rules → `false`, and a second load still does not re-fetch.
        assert!(
            !cache
                .ensure_loaded(&model, "foo/bar", "wf", false)
                .await
                .unwrap()
        );
        assert!(
            !cache
                .ensure_loaded(&model, "foo/bar", "wf", false)
                .await
                .unwrap()
        );
        // Validation against an ungoverned selection yields no violations.
        assert!(
            cache
                .validate("foo/bar", "wf", "", "", "foo/bar")
                .await
                .is_empty()
        );
        Ok(())
    }

    /// Once rules are cached, validation reads them (no I/O) and returns typed
    /// violations routed to the field the user must fix; a satisfying candidate
    /// clears them.
    #[tokio::test]
    async fn validate_returns_typed_violations_from_cache() {
        let mut model = mocks::create();
        model
            .expect_get_installed_package()
            .returning(|_| Ok(Some(make_installed_package(("foo", "bar")))));
        model
            .expect_get_workflow_rules()
            .returning(|_, _| Ok(Some(strict_rules())));

        let cache = WorkflowRulesCache::default();
        cache
            .ensure_loaded(&model, "foo/bar", "wf", false)
            .await
            .unwrap();

        // Missing message, non-matching name, metadata missing `owner`.
        let violations = cache.validate("foo/bar", "wf", "", "{}", "other/pkg").await;
        let fields: Vec<_> = violations.iter().map(|v| v.field).collect();
        assert!(fields.contains(&ViolationField::Message));
        assert!(fields.contains(&ViolationField::Name));
        assert!(fields.contains(&ViolationField::Metadata));

        // A fully-satisfying candidate clears every violation.
        let violations = cache
            .validate(
                "foo/bar",
                "wf",
                "a message",
                r#"{"owner":"alice"}"#,
                "team/x",
            )
            .await;
        assert!(
            violations.is_empty(),
            "expected no violations: {violations:?}"
        );
    }

    /// Validating before any rules are loaded is a clean no-op — the dialog just
    /// hasn't fetched yet, so there is nothing to complain about.
    #[tokio::test]
    async fn validate_without_loaded_rules_is_clean() {
        let cache = WorkflowRulesCache::default();
        assert!(
            cache
                .validate("foo/bar", "wf", "", "", "foo/bar")
                .await
                .is_empty()
        );
    }

    /// Unparseable user metadata surfaces as the single metadata violation,
    /// mirroring the commit path (which parses the raw string server-side).
    #[test]
    fn validate_candidate_flags_unparseable_metadata() {
        let rules = WorkflowRules {
            handle_pattern: None,
            is_message_required: false,
            metadata_schema: None,
            entries_schema: None,
        };
        let violations = validate_candidate(&rules, "msg", "{ not json", "foo/bar");
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].field, ViolationField::Metadata);
        assert!(violations[0].message.contains("not valid JSON"));
    }

    /// A metadata parse error must not swallow violations on the other fields:
    /// the commit path validates message and handle regardless of whether the
    /// metadata parses, so the advisory path reports them together.
    #[test]
    fn validate_candidate_reports_other_fields_alongside_parse_error() {
        let rules = WorkflowRules {
            handle_pattern: Some("^prefix/".to_string()),
            is_message_required: true,
            metadata_schema: None,
            entries_schema: None,
        };
        let violations = validate_candidate(&rules, "", "{ not json", "foo/bar");
        let fields: Vec<ViolationField> = violations.iter().map(|v| v.field).collect();
        assert_eq!(violations[0].field, ViolationField::Metadata);
        assert!(violations[0].message.contains("not valid JSON"));
        assert!(fields.contains(&ViolationField::Message));
        assert!(fields.contains(&ViolationField::Name));
        assert_eq!(violations.len(), 3);
    }

    /// The tagged JSON `CommitViolation` / `ViolationField` serialize to is the
    /// wire contract the UI mirror deserializes; if these strings drift, the
    /// commit dialog routes violations to the wrong field (or drops them).
    #[test]
    fn commit_violation_wire_form_is_verbatim() {
        assert_eq!(
            serde_json::to_value(CommitViolation {
                field: ViolationField::Metadata,
                message: "bad".to_string(),
            })
            .unwrap(),
            serde_json::json!({ "field": "metadata", "message": "bad" })
        );
        assert_eq!(
            serde_json::to_value(ViolationField::Message).unwrap(),
            serde_json::json!("message")
        );
        assert_eq!(
            serde_json::to_value(ViolationField::Name).unwrap(),
            serde_json::json!("name")
        );
    }
}
