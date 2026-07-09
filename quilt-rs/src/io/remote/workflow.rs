use std::sync::LazyLock;

use jsonschema::Validator;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use serde_yaml::Value as YamlValue;
use tokio::io::AsyncReadExt;

use crate::Error;
use crate::Res;
use crate::error::RemoteCatalogError;
use crate::io::remote::Remote;
use crate::manifest::ManifestRow;
use crate::manifest::MetadataSchema;
use crate::manifest::Workflow;
use crate::manifest::WorkflowId;
use crate::workflow::EntryView;
use crate::workflow::PackageCandidate;
use crate::workflow::WorkflowRules;
use crate::workflow::compile_config_schema;
use crate::workflow::validate_package;
use quilt_uri::Host;
use quilt_uri::S3Uri;

/// The workflows-config JSON Schema, vendored byte-identical from quilt3
/// (`quilt3/workflows/config-1.schema.json`). quilt3 validates every loaded
/// `.quilt/workflows/config.yml` against this with a plain `Draft7Validator`
/// (`quilt3.workflows._get_conf_validator`) and refuses a malformed config, so
/// we do the same — otherwise a YAML typo (e.g. a quoted `is_message_required`)
/// would silently disable a rule the bucket owner believes is enforced.
const CONFIG_SCHEMA: &str = include_str!("config-1.schema.json");

/// The compiled config-schema validator. Built once: the schema is a
/// compile-time constant, so compilation cannot fail on user input.
static CONFIG_VALIDATOR: LazyLock<Validator> = LazyLock::new(|| {
    let schema: Value = serde_json::from_str(CONFIG_SCHEMA)
        .expect("vendored workflows-config schema is valid JSON");
    compile_config_schema(&schema)
});

/// Validate a decoded `config.yml` document against the vendored quilt3 config
/// schema, exactly as quilt3 does at load. On any violation, fails with a
/// [`RemoteCatalogError::InvalidWorkflowsConfig`] (classified as a conflict, not
/// a transient, by the sync watcher) naming every offending path — so a
/// malformed config refuses everywhere rather than half-working.
fn validate_config_document(yaml: &YamlValue) -> Res<()> {
    use std::fmt::Write;

    // `serde_yaml::Value` is `Serialize`, so this reuses serde's own YAML→JSON
    // conversion rather than a hand-rolled walker. A YAML document JSON cannot
    // represent (e.g. a non-string mapping key) is itself an invalid config, so
    // route the conversion failure into the same variant rather than letting it
    // escape as an opaque `Error::Json`.
    let document: Value = serde_json::to_value(yaml).map_err(|err| {
        RemoteCatalogError::InvalidWorkflowsConfig(format!(
            "workflows/config.yml could not be converted for schema validation: {err}"
        ))
    })?;
    let mut message = String::new();
    for err in CONFIG_VALIDATOR.iter_errors(&document) {
        let _ = write!(message, "\n  - {err} (at {})", err.instance_path());
    }
    if message.is_empty() {
        Ok(())
    } else {
        Err(RemoteCatalogError::InvalidWorkflowsConfig(format!(
            "workflows/config.yml does not satisfy the workflows config schema:{message}"
        ))
        .into())
    }
}

/// Caller intent for resolving a package's workflow (the per-bucket quality-gate
/// reference stored in the manifest header).
///
/// - [`WorkflowIntent::BucketDefault`] — the caller has no opinion. Honour the
///   config's top-level `default_workflow` when it is declared; otherwise fall
///   back to today's outcome (a workflow record with `id: null` when a config
///   is present, or nothing when there is no config).
/// - [`WorkflowIntent::NoWorkflow`] — an explicit opt-out. Produces an `id: null`
///   record when a config is present, or nothing when there is no config.
/// - [`WorkflowIntent::Named`] — an exact workflow id, resolved against the
///   config or an error if it cannot be found.
///
/// Note: the config's `is_workflow_required` flag is deliberately **not**
/// enforced here — enforcement is a separate slice.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "kebab-case")]
pub enum WorkflowIntent {
    BucketDefault,
    NoWorkflow,
    Named(String),
}

impl WorkflowIntent {
    /// Map an optional workflow id to an intent: absent or blank → `BucketDefault`
    /// (never `Named("")`); a non-blank id → `Named` with surrounding whitespace
    /// trimmed. The shared normalization used by both frontends.
    pub fn from_optional_id(id: Option<&str>) -> Self {
        match id.map(str::trim) {
            Some(id) if !id.is_empty() => Self::Named(id.to_string()),
            _ => Self::BucketDefault,
        }
    }
}

/// A single workflow entry as declared under `workflows:` in
/// `.quilt/workflows/config.yml`, carrying its display metadata.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowInfo {
    pub id: String,
    pub name: Option<String>,
    pub description: Option<String>,
}

/// Typed view of `.quilt/workflows/config.yml`.
///
/// Construction is strict: [`WorkflowsConfig::from_yaml`] first validates the
/// whole document against quilt3's vendored config JSON Schema (see
/// `CONFIG_SCHEMA`) and refuses a malformed config, so a `WorkflowsConfig`
/// value can only exist for a config quilt3 would also accept. The typed fields
/// (`default_workflow`, `is_workflow_required`, `workflows`) surface what the
/// commit dialog needs; the resolution helpers dig into the retained raw YAML
/// for the remaining fields, now guaranteed well-typed by the schema.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowsConfig {
    /// Top-level `default_workflow`, when declared as a string.
    pub default_workflow: Option<String>,
    /// `is_workflow_required`; defaults to `true` when the key is absent (matches quilt3).
    pub is_workflow_required: bool,
    /// The declared workflows, in file order.
    pub workflows: Vec<WorkflowInfo>,
    /// Retained source, used to resolve schemas exactly as the legacy helpers did.
    raw: YamlValue,
}

impl WorkflowsConfig {
    /// Parse an already-decoded `config.yml` value into the typed view.
    ///
    /// Validates the whole document against quilt3's config schema first
    /// (mirroring quilt3's load-time `Draft7Validator`), so a malformed config
    /// — a typo'd type, a missing required key — is rejected here rather than
    /// silently degrading a rule to a default. This is the single construction
    /// point every consumer (commit/push enforcement, `resolve_workflow`, the
    /// bucket-workflows selector) reaches, so the check runs exactly once per
    /// loaded config on every path.
    pub fn from_yaml(yaml: &YamlValue) -> Res<WorkflowsConfig> {
        validate_config_document(yaml)?;

        let default_workflow = yaml
            .get("default_workflow")
            .and_then(YamlValue::as_str)
            .map(String::from);
        let is_workflow_required = yaml
            .get("is_workflow_required")
            .and_then(YamlValue::as_bool)
            .unwrap_or(true);
        let workflows = yaml
            .get("workflows")
            .and_then(YamlValue::as_mapping)
            .map(|workflows| {
                workflows
                    .iter()
                    .filter_map(|(id, entry)| {
                        Some(WorkflowInfo {
                            id: id.as_str()?.to_string(),
                            name: entry
                                .get("name")
                                .and_then(YamlValue::as_str)
                                .map(String::from),
                            description: entry
                                .get("description")
                                .and_then(YamlValue::as_str)
                                .map(String::from),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(WorkflowsConfig {
            default_workflow,
            is_workflow_required,
            workflows,
            raw: yaml.clone(),
        })
    }

    /// The raw mapping for a named workflow, if declared.
    fn workflow_entry(&self, workflow_id: &str) -> Option<&YamlValue> {
        self.raw.get("workflows")?.get(workflow_id)
    }

    /// The `handle_pattern` regex declared by a workflow, if any. Lenient:
    /// a non-string value degrades to `None`, matching the parser's stance.
    fn handle_pattern(&self, workflow_id: &str) -> Option<String> {
        self.workflow_entry(workflow_id)?
            .get("handle_pattern")
            .and_then(YamlValue::as_str)
            .map(String::from)
    }

    /// A workflow's `is_message_required` flag; defaults to `false` (matches quilt3).
    fn is_message_required(&self, workflow_id: &str) -> bool {
        self.workflow_entry(workflow_id)
            .and_then(|workflow| workflow.get("is_message_required"))
            .and_then(YamlValue::as_bool)
            .unwrap_or(false)
    }

    /// The schema id a workflow declares under `key` (`metadata_schema` or
    /// `entries_schema`), mirroring the legacy lazy lookup (including its
    /// error variants) exactly.
    fn schema_id(&self, workflow_id: &str, key: &str) -> Res<Option<String>> {
        match self.raw.get("workflows") {
            Some(YamlValue::Mapping(workflows)) => match workflows.get(workflow_id) {
                Some(YamlValue::Mapping(workflow)) => match workflow.get(key) {
                    Some(YamlValue::String(schema_id)) => Ok(Some(schema_id.clone())),
                    // Absent key: the workflow simply declares no such schema.
                    None => Ok(None),
                    // Present but not a string (explicit null, mapping, list):
                    // a misconfiguration, reported as such — not as "not found".
                    Some(_) => Err(Error::RemoteCatalog(RemoteCatalogError::Workflow(format!(
                        "`{key}` for workflow ID {workflow_id} must be a string"
                    )))),
                },
                _ => Err(Error::RemoteCatalog(RemoteCatalogError::Workflow(format!(
                    "Workflow {workflow_id} not found in workflows/config.yaml"
                )))),
            },
            _ => Err(Error::RemoteCatalog(RemoteCatalogError::Workflow(
                "Workflows not found in workflows/config.yaml".to_string(),
            ))),
        }
    }

    /// Resolve the URL of a schema by its id, looking it up in the `schemas`
    /// section. `workflow_id` is used only for error context.
    async fn resolve_schema_url<R: Remote>(
        &self,
        remote: &R,
        host: &Option<Host>,
        workflow_id: &str,
        schema_id: &str,
    ) -> Res<S3Uri> {
        match self.raw.get("schemas") {
            Some(YamlValue::Mapping(schemas)) => match schemas.get(schema_id) {
                Some(YamlValue::Mapping(schema)) => match schema.get("url") {
                    Some(YamlValue::String(url)) => {
                        Ok(remote.resolve_url(host, &url.parse()?).await?)
                    }
                    _ => Err(Error::RemoteCatalog(RemoteCatalogError::Workflow(format!(
                        "Schema {schema_id} doesn't have URL"
                    )))),
                },
                _ => Err(Error::RemoteCatalog(RemoteCatalogError::Workflow(format!(
                    "Schema {schema_id}, referenced by workflow {workflow_id} not found in workflows/config.yaml",
                )))),
            },
            _ => Err(Error::RemoteCatalog(RemoteCatalogError::Workflow(
                "Schemas not found in workflows/config.yaml".to_string(),
            ))),
        }
    }

    /// Resolve the id and URL of the metadata schema referenced by a workflow.
    async fn schema_url<R: Remote>(
        &self,
        remote: &R,
        host: &Option<Host>,
        workflow_id: &str,
    ) -> Res<Option<(String, S3Uri)>> {
        match self.schema_id(workflow_id, "metadata_schema")? {
            Some(schema_id) => {
                let url = self
                    .resolve_schema_url(remote, host, workflow_id, &schema_id)
                    .await?;
                Ok(Some((schema_id, url)))
            }
            None => Ok(None),
        }
    }

    /// Interpret the top-level `default_workflow` key for the bucket-default intent.
    ///
    /// - key absent → `Ok(None)`: caller produces a null-id record.
    /// - string → `Ok(Some(id))`: caller resolves it like a named workflow.
    /// - anything else (including explicit null) → `Err`: misconfiguration.
    fn bucket_default_id(&self) -> Res<Option<String>> {
        match self.raw.get("default_workflow") {
            None => Ok(None),
            Some(YamlValue::String(id)) => Ok(Some(id.clone())),
            Some(_) => Err(Error::RemoteCatalog(RemoteCatalogError::Workflow(
                "`default_workflow` in workflows/config.yaml must be a string".to_string(),
            ))),
        }
    }

    /// Resolve a named workflow id against this config, attaching the referenced
    /// metadata schema when the workflow declares one.
    async fn resolve_named<R: Remote>(
        &self,
        remote: &R,
        host: &Option<Host>,
        config: S3Uri,
        id: String,
    ) -> Res<Option<Workflow>> {
        if let Some((metadata_id, metadata_url)) = self.schema_url(remote, host, &id).await? {
            Ok(Some(Workflow {
                config,
                id: Some(WorkflowId {
                    id,
                    metadata: Some(MetadataSchema {
                        id: metadata_id,
                        url: metadata_url,
                    }),
                }),
            }))
        } else {
            Ok(Some(Workflow {
                config,
                id: Some(WorkflowId { id, metadata: None }),
            }))
        }
    }
}

pub(crate) async fn fetch_workflows_config<R: Remote>(
    remote: &R,
    host: &Option<Host>,
    uri: &S3Uri,
) -> Res<(S3Uri, Option<WorkflowsConfig>)> {
    if !remote.exists(host, uri).await? {
        return Ok((uri.clone(), None));
    }
    match remote.get_object_stream(host, uri).await {
        Ok(stream) => {
            let mut bytes = Vec::new();
            stream
                .body
                .into_async_read()
                .read_to_end(&mut bytes)
                .await?;
            let config = serde_yaml::from_slice::<Option<YamlValue>>(&bytes)?
                .map(|yaml| WorkflowsConfig::from_yaml(&yaml))
                .transpose()?;
            Ok((stream.uri, config))
        }
        Err(err) => Err(err),
    }
}

/// Fetch and parse the `.quilt/workflows/config.yml` for an arbitrary bucket,
/// independent of any package's already-set remote.
///
/// Builds the config address from `bucket` alone, so the pre-set-remote UI can
/// preview a bucket's declared workflows before the choice is committed.
/// Returns `Ok(None)` when the bucket has no config.
pub async fn fetch_workflows_config_for_bucket<R: Remote>(
    remote: &R,
    host: &Option<Host>,
    bucket: &str,
) -> Res<Option<WorkflowsConfig>> {
    let uri = S3Uri {
        key: ".quilt/workflows/config.yml".to_string(),
        bucket: bucket.to_string(),
        version: None,
    };
    let (_, config) = fetch_workflows_config(remote, host, &uri).await?;
    Ok(config)
}

/// Fetch a schema document from the remote and parse it as JSON.
async fn fetch_schema_doc<R: Remote>(remote: &R, host: &Option<Host>, uri: &S3Uri) -> Res<Value> {
    let stream = remote.get_object_stream(host, uri).await?;
    let mut bytes = Vec::new();
    stream
        .body
        .into_async_read()
        .read_to_end(&mut bytes)
        .await?;
    Ok(serde_json::from_slice(&bytes)?)
}

/// Assemble the pure-validator [`WorkflowRules`] for a named workflow: read the
/// `handle_pattern` / `is_message_required` flags straight from the parsed
/// config (no fetch), and fetch the `metadata_schema` / `entries_schema`
/// documents referenced by the workflow as `serde_json::Value`.
///
/// This is the I/O boundary between the remote layer and the pure gate in
/// `crate::workflow`: it turns config + S3 into the plain inputs
/// [`crate::workflow::validate_package`] consumes.
pub async fn fetch_workflow_rules<R: Remote>(
    remote: &R,
    host: &Option<Host>,
    config: &WorkflowsConfig,
    workflow_id: &str,
) -> Res<WorkflowRules> {
    let metadata_schema =
        fetch_schema_for_key(remote, host, config, workflow_id, "metadata_schema").await?;
    let entries_schema =
        fetch_schema_for_key(remote, host, config, workflow_id, "entries_schema").await?;

    Ok(WorkflowRules {
        handle_pattern: config.handle_pattern(workflow_id),
        is_message_required: config.is_message_required(workflow_id),
        metadata_schema,
        entries_schema,
    })
}

/// Fetch the schema document a workflow declares under `key`, or `None` when
/// the workflow declares no such schema.
async fn fetch_schema_for_key<R: Remote>(
    remote: &R,
    host: &Option<Host>,
    config: &WorkflowsConfig,
    workflow_id: &str,
    key: &str,
) -> Res<Option<Value>> {
    match config.schema_id(workflow_id, key)? {
        Some(schema_id) => {
            let uri = config
                .resolve_schema_url(remote, host, workflow_id, &schema_id)
                .await?;
            Ok(Some(fetch_schema_doc(remote, host, &uri).await?))
        }
        None => Ok(None),
    }
}

/// Project a manifest row to the [`EntryView`] the workflow gate inspects.
///
/// `meta` is the row's **unwrapped** user metadata — the value under the row's
/// `user_meta` key — matching quilt3, whose entry projection uses
/// `PackageEntry.meta` (`self._meta.get('user_meta', {})`). The row's own
/// `meta` wire value is the wrapped form `{"user_meta": {...}}`, so we peel one
/// level here; an absent `user_meta` maps to `None` and the `{}`-default is
/// applied by `project_entries` in the pure gate (matching quilt3's default).
///
/// Shared by the commit and push flows so both project rows identically.
///
/// TODO: the gates materialize every manifest row and project all entries into
/// one JSON array for `entries_schema` validation, so memory scales with
/// manifest size. When large manifests arrive (streamed formats such as
/// Parquet/Iceberg over [`crate::io::manifest::RowsStream`]), validate lazily
/// instead: classify the
/// schema up front and stream-validate the dominant subset (a single per-item
/// `items` schema, count/tuple/`contains` accumulators), materializing only
/// for array-level combinators that genuinely need the whole entry set.
pub(crate) fn entry_view(row: &ManifestRow) -> EntryView<'_> {
    EntryView {
        // Lossy: a non-UTF-8 logical key projects with U+FFFD replacement
        // characters, so the entries_schema validates an approximation of the
        // real key instead of a fabricated empty string. A valid key borrows.
        logical_key: row.logical_key.to_string_lossy(),
        size: row.size,
        meta: row.meta.as_ref().and_then(|meta| meta.get("user_meta")),
    }
}

/// Run the workflow quality gate against a candidate revision, using the
/// workflow already recorded in its manifest header.
///
/// This is the enforcement counterpart to [`resolve_workflow`]: resolution
/// stamps a workflow into the header, enforcement re-reads that workflow's
/// config and schema documents and checks the candidate against them. It is
/// the single I/O + gate seam shared by the commit and push flows.
///
/// A vacuously-valid revision is left untouched:
///
/// - a header with no workflow (`workflow` is `None`) — an ungoverned bucket;
/// - a header whose workflow's config has since disappeared from the bucket.
///
/// Otherwise the workflow's rules are fetched (schema documents included) and
/// [`validate_package`] decides. A rule failure surfaces as
/// [`crate::Error::WorkflowValidation`] — a distinct typed error the sync
/// watcher classifies as a conflict (pause the namespace), not a transient
/// (retry) — while a failed *fetch* stays an `Error::S3` and remains transient.
pub(crate) async fn validate_workflow<R: Remote>(
    remote: &R,
    host: &Option<Host>,
    name: &str,
    message: Option<&str>,
    user_meta: Option<&Value>,
    workflow: Option<&Workflow>,
    entries: &[EntryView<'_>],
) -> Res<()> {
    let config = match workflow {
        Some(workflow) => {
            fetch_workflows_config(remote, host, &workflow.config)
                .await?
                .1
        }
        None => None,
    };
    validate_workflow_with_config(
        remote,
        host,
        name,
        message,
        user_meta,
        workflow,
        config.as_ref(),
        entries,
    )
    .await
}

/// The workflow gate given a config the caller has already fetched and parsed,
/// skipping the `.quilt/workflows/config.yml` fetch [`validate_workflow`] would
/// otherwise perform via the header's pinned config URI.
///
/// `config` must be the parsed config that produced `workflow` — the same
/// object the pinned URI in `workflow.config` addresses — so reusing it is
/// semantically identical to re-fetching. Pass `None` for a bucket with no
/// config (a vacuously-valid revision, left untouched); a `None` `workflow` is
/// likewise vacuously valid. Otherwise identical to [`validate_workflow`]: same
/// rules, same [`validate_package`] decision.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn validate_workflow_with_config<R: Remote>(
    remote: &R,
    host: &Option<Host>,
    name: &str,
    message: Option<&str>,
    user_meta: Option<&Value>,
    workflow: Option<&Workflow>,
    config: Option<&WorkflowsConfig>,
    entries: &[EntryView<'_>],
) -> Res<()> {
    let Some(workflow) = workflow else {
        return Ok(());
    };
    let Some(config) = config else {
        return Ok(());
    };
    let rules = match &workflow.id {
        Some(workflow_id) => {
            Some(fetch_workflow_rules(remote, host, config, &workflow_id.id).await?)
        }
        None => None,
    };
    let candidate = PackageCandidate {
        name,
        message,
        user_meta,
        entries,
    };
    validate_package(rules.as_ref(), config.is_workflow_required, &candidate)?;
    Ok(())
}

/// Run the workflow quality gate at push time against the destination
/// bucket's **current** workflows config, ignoring the version-pinned config
/// URI recorded in the manifest header.
///
/// This is the push-side counterpart to [`validate_workflow`] (which trusts
/// the header's resolved, version-pinned workflow and is used by the commit
/// and `set_remote` gates). A revision may have been committed by an older or
/// different client, or against a config version that has since been deleted
/// or lifecycle-expired, so push re-loads `.quilt/workflows/config.yml` from
/// the destination bucket and decides against it — mirroring quilt3, which
/// re-loads the registry's current config on every push
/// (`quilt3/workflows/__init__.py::validate`).
///
/// Cases (with `workflow_id` = the id stamped in the header, if any):
///
/// - No config exists now:
///   - `workflow_id` is `None` → pass (ungoverned bucket).
///   - `workflow_id` is `Some(id)` → hard error, matching quilt3's decision
///     (its message reads "no workflows config exist"; ours fixes the
///     grammar).
/// - Config exists:
///   - `workflow_id` is `Some(id)` missing from the current config → hard
///     error, matching quilt3's decision ("There is no `{id}` workflow in
///     config"; ours adds the article).
///   - `workflow_id` is `None` → [`validate_package`] rejects with
///     `WorkflowRequired` when `is_workflow_required` (default true), else
///     passes.
///   - Otherwise the full gate (metadata/entries schemas, handle pattern,
///     message-required) runs against the current config's rules for `id`.
///
/// A rule failure surfaces as [`crate::Error::WorkflowValidation`] and a
/// missing/unknown workflow as [`crate::Error::RemoteCatalog`] — both
/// classified as conflicts by the sync watcher — while a failed *fetch* stays
/// an `Error::S3` and remains transient.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn validate_workflow_against_current_config<R: Remote>(
    remote: &R,
    host: &Option<Host>,
    bucket: &str,
    name: &str,
    message: Option<&str>,
    user_meta: Option<&Value>,
    header_workflow: Option<&Workflow>,
    entries: &[EntryView<'_>],
) -> Res<()> {
    let workflow_id = header_workflow
        .and_then(|workflow| workflow.id.as_ref())
        .map(|id| id.id.as_str());
    let config = fetch_workflows_config_for_bucket(remote, host, bucket).await?;
    let Some(config) = config else {
        return match workflow_id {
            None => Ok(()),
            Some(id) => Err(Error::RemoteCatalog(RemoteCatalogError::Workflow(format!(
                "\"{id}\" workflow is specified, but no workflows config exists"
            )))),
        };
    };
    let rules = match workflow_id {
        Some(id) => {
            if config.workflow_entry(id).is_none() {
                return Err(Error::RemoteCatalog(RemoteCatalogError::Workflow(format!(
                    "There is no \"{id}\" workflow in the config"
                ))));
            }
            Some(fetch_workflow_rules(remote, host, &config, id).await?)
        }
        None => None,
    };
    let candidate = PackageCandidate {
        name,
        message,
        user_meta,
        entries,
    };
    validate_package(rules.as_ref(), config.is_workflow_required, &candidate)?;
    Ok(())
}

/// Resolve the workflow to attach to a manifest header, given the caller's
/// [`WorkflowIntent`] and the presence/contents of `workflows/config.yaml`.
///
/// - [`WorkflowIntent::Named`] — a config is required; the id is resolved against
///   it (`""` and any unknown id error), otherwise it is an error.
/// - [`WorkflowIntent::NoWorkflow`] — `Some(Workflow { id: None })` when a config
///   is present, `None` when there is no config.
/// - [`WorkflowIntent::BucketDefault`] — `None` when there is no config;
///   otherwise the config's top-level `default_workflow` decides: absent →
///   `Some(Workflow { id: None })`; a string → resolved like [`WorkflowIntent::Named`]
///   (a missing referenced workflow errors — misconfiguration must be loud, not
///   silently ungoverned); a non-string value → error.
pub async fn resolve_workflow<R: Remote>(
    remote: &R,
    host: &Option<Host>,
    intent: WorkflowIntent,
    uri: &S3Uri,
) -> Res<Option<Workflow>> {
    let (config, parsed) = fetch_workflows_config(remote, host, uri).await?;
    resolve_workflow_from_config(remote, host, intent, config, parsed.as_ref()).await
}

/// Resolve a workflow from a config the caller has already fetched and parsed,
/// so a caller that must also enforce the workflow (e.g. `set_remote`, which
/// resolves then recommits) fetches `.quilt/workflows/config.yml` exactly once.
///
/// `config` is the (possibly version-pinned) config URI returned by
/// [`fetch_workflows_config`] and `parsed` its parsed value; the resulting
/// [`Workflow`] stamps `config`, so the pinned URI it carries and `parsed`
/// describe the same object. Same resolution as [`resolve_workflow`], which
/// delegates here after fetching.
pub(crate) async fn resolve_workflow_from_config<R: Remote>(
    remote: &R,
    host: &Option<Host>,
    intent: WorkflowIntent,
    config: S3Uri,
    parsed: Option<&WorkflowsConfig>,
) -> Res<Option<Workflow>> {
    match (parsed, intent) {
        (Some(parsed), WorkflowIntent::Named(id)) => {
            parsed.resolve_named(remote, host, config, id).await
        }
        (None, WorkflowIntent::Named(id)) => {
            Err(Error::RemoteCatalog(RemoteCatalogError::Workflow(format!(
                "There is no workflows config, but the workflow \"{id}\" is set"
            ))))
        }
        (Some(_), WorkflowIntent::NoWorkflow) => Ok(Some(Workflow { config, id: None })),
        (None, WorkflowIntent::NoWorkflow) => Ok(None),
        (None, WorkflowIntent::BucketDefault) => Ok(None),
        (Some(parsed), WorkflowIntent::BucketDefault) => match parsed.bucket_default_id()? {
            None => Ok(Some(Workflow { config, id: None })),
            Some(id) => parsed.resolve_named(remote, host, config, id).await,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::remote::mocks::MockRemote;
    use test_log::test;

    #[test(tokio::test)]
    async fn test_missing_schemas_section() -> Res<()> {
        let remote = MockRemote::default();
        let host = None;
        let uri: S3Uri = "s3://any/.quilt/workflows/config.yml".parse()?;

        // Put test config.yaml with workflow but no schemas section
        let config = r"
version: '1'
workflows:
  foo:
    name: Foo
    metadata_schema: bar
";
        remote
            .put_object(&None, &uri, config.as_bytes().to_vec())
            .await?;

        // Should error when trying to resolve a workflow with missing schema section
        let err = resolve_workflow(
            &remote,
            &host,
            WorkflowIntent::Named("foo".to_string()),
            &uri,
        )
        .await
        .unwrap_err();

        assert!(matches!(
            err,
            Error::RemoteCatalog(RemoteCatalogError::Workflow(_))
        ));
        assert!(err.to_string().contains("Schemas not found"));

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_no_config_yaml() -> Res<()> {
        let remote = MockRemote::default();
        let host = None;
        let uri: S3Uri = "s3://any/.quilt/workflows/config.yml".parse()?;

        // Case 1.a: No config.yaml and workflow_id is None
        let result = resolve_workflow(&remote, &host, WorkflowIntent::NoWorkflow, &uri).await?;
        assert!(result.is_none());

        // Case 1.b: No config.yaml but workflow_id is set
        let err = resolve_workflow(
            &remote,
            &host,
            WorkflowIntent::Named("test-workflow".to_string()),
            &uri,
        )
        .await
        .unwrap_err();
        assert!(matches!(
            err,
            Error::RemoteCatalog(RemoteCatalogError::Workflow(_))
        ));
        assert!(err.to_string().contains("There is no workflows config"));

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_with_config_yaml() -> Res<()> {
        let remote = MockRemote::default();
        let host = None;
        let uri: S3Uri = "s3://any/.quilt/workflows/config.yml".parse()?;

        // Put test config.yaml into mock remote storage
        let config = r"
version: '1'
workflows:
  foo:
    name: Foo
    metadata_schema: bar
schemas:
  bar:
    url: s3://test-bucket/schemas/test.json
";
        let schema_uri: S3Uri = "s3://test-bucket/schemas/test.json".parse()?;
        let schema = b"{}";
        remote
            .put_object(&None, &uri, config.as_bytes().to_vec())
            .await?;
        remote
            .put_object(&None, &schema_uri, schema.to_vec())
            .await?;

        // Case 2.a: Config exists, workflow_id is set and valid
        let result = resolve_workflow(
            &remote,
            &host,
            WorkflowIntent::Named("foo".to_string()),
            &uri,
        )
        .await?
        .unwrap();
        assert_eq!(result.config, uri);
        assert_eq!(
            result.id.unwrap(),
            WorkflowId {
                id: "foo".to_string(),
                metadata: Some(MetadataSchema {
                    id: "bar".to_string(),
                    url: "s3://test-bucket/schemas/test.json".parse()?
                })
            }
        );

        // Case 2.b: Config exists but workflow_id is None
        let result = resolve_workflow(&remote, &host, WorkflowIntent::NoWorkflow, &uri)
            .await?
            .unwrap();
        assert_eq!(result.config, uri);
        assert!(result.id.is_none());

        // Case 2.c: Config exists but workflow_id is not found
        let err = resolve_workflow(
            &remote,
            &host,
            WorkflowIntent::Named("non-existent".to_string()),
            &uri,
        )
        .await
        .unwrap_err();
        assert!(matches!(
            err,
            Error::RemoteCatalog(RemoteCatalogError::Workflow(_))
        ));
        assert!(err.to_string().contains("Workflow non-existent not found"));

        // Case 2.d: Config exists but workflow_id is empty
        let err = resolve_workflow(&remote, &host, WorkflowIntent::Named(String::new()), &uri)
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            Error::RemoteCatalog(RemoteCatalogError::Workflow(_))
        ));
        assert!(err.to_string().contains("Workflow  not found"));

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_bucket_default_no_config() -> Res<()> {
        let remote = MockRemote::default();
        let host = None;
        let uri: S3Uri = "s3://any/.quilt/workflows/config.yml".parse()?;

        // No config.yaml: bucket-default resolves to nothing.
        let result = resolve_workflow(&remote, &host, WorkflowIntent::BucketDefault, &uri).await?;
        assert!(result.is_none());

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_bucket_default_without_key() -> Res<()> {
        let remote = MockRemote::default();
        let host = None;
        let uri: S3Uri = "s3://any/.quilt/workflows/config.yml".parse()?;

        // Config present but no `default_workflow` key → today's null-id record.
        let config = r"
version: '1'
workflows:
  foo:
    name: Foo
    metadata_schema: bar
";
        remote
            .put_object(&None, &uri, config.as_bytes().to_vec())
            .await?;

        let result = resolve_workflow(&remote, &host, WorkflowIntent::BucketDefault, &uri)
            .await?
            .unwrap();
        assert_eq!(result.config, uri);
        assert!(result.id.is_none());

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_bucket_default_with_valid_key() -> Res<()> {
        let remote = MockRemote::default();
        let host = None;
        let uri: S3Uri = "s3://any/.quilt/workflows/config.yml".parse()?;

        // Config declares a `default_workflow` pointing at a valid workflow.
        let config = r"
version: '1'
default_workflow: foo
workflows:
  foo:
    name: Foo
    metadata_schema: bar
schemas:
  bar:
    url: s3://test-bucket/schemas/test.json
";
        let schema_uri: S3Uri = "s3://test-bucket/schemas/test.json".parse()?;
        remote
            .put_object(&None, &uri, config.as_bytes().to_vec())
            .await?;
        remote
            .put_object(&None, &schema_uri, b"{}".to_vec())
            .await?;

        let result = resolve_workflow(&remote, &host, WorkflowIntent::BucketDefault, &uri)
            .await?
            .unwrap();
        assert_eq!(result.config, uri);
        assert_eq!(
            result.id.unwrap(),
            WorkflowId {
                id: "foo".to_string(),
                metadata: Some(MetadataSchema {
                    id: "bar".to_string(),
                    url: "s3://test-bucket/schemas/test.json".parse()?
                })
            }
        );

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_bucket_default_missing_referenced_workflow() -> Res<()> {
        let remote = MockRemote::default();
        let host = None;
        let uri: S3Uri = "s3://any/.quilt/workflows/config.yml".parse()?;

        // `default_workflow` names a workflow that is not declared → loud error.
        let config = r"
version: '1'
default_workflow: ghost
workflows:
  foo:
    name: Foo
    metadata_schema: bar
";
        remote
            .put_object(&None, &uri, config.as_bytes().to_vec())
            .await?;

        let err = resolve_workflow(&remote, &host, WorkflowIntent::BucketDefault, &uri)
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            Error::RemoteCatalog(RemoteCatalogError::Workflow(_))
        ));
        assert!(err.to_string().contains("Workflow ghost not found"));

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_bucket_default_non_string_key() -> Res<()> {
        let remote = MockRemote::default();
        let host = None;
        let uri: S3Uri = "s3://any/.quilt/workflows/config.yml".parse()?;

        // `default_workflow` present but not a string → the config-schema
        // validation at load rejects it (quilt3 types `default_workflow` as a
        // string), so it never reaches resolution.
        let config = r"
version: '1'
default_workflow: [not, a, string]
workflows:
  foo:
    name: Foo
    metadata_schema: bar
";
        remote
            .put_object(&None, &uri, config.as_bytes().to_vec())
            .await?;

        let err = resolve_workflow(&remote, &host, WorkflowIntent::BucketDefault, &uri)
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            Error::RemoteCatalog(RemoteCatalogError::InvalidWorkflowsConfig(_))
        ));
        let message = err.to_string();
        assert!(
            message.contains("does not satisfy the workflows config schema"),
            "unexpected message: {message}"
        );
        assert!(
            message.contains("/default_workflow"),
            "violation must name the offending path, got: {message}"
        );

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_bucket_default_explicit_null_key() -> Res<()> {
        let remote = MockRemote::default();
        let host = None;
        let uri: S3Uri = "s3://any/.quilt/workflows/config.yml".parse()?;

        // `default_workflow:` with no value is a YAML explicit null — not a
        // string, so the config-schema validation at load rejects it (quilt3
        // types `default_workflow` as a string and schema-validates the whole
        // config on load, so this config fails every push there too).
        let config = r"
version: '1'
default_workflow:
workflows:
  foo:
    name: Foo
    metadata_schema: bar
";
        remote
            .put_object(&None, &uri, config.as_bytes().to_vec())
            .await?;

        let err = resolve_workflow(&remote, &host, WorkflowIntent::BucketDefault, &uri)
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            Error::RemoteCatalog(RemoteCatalogError::InvalidWorkflowsConfig(_))
        ));
        let message = err.to_string();
        assert!(
            message.contains("does not satisfy the workflows config schema"),
            "unexpected message: {message}"
        );
        assert!(
            message.contains("/default_workflow"),
            "violation must name the offending path, got: {message}"
        );

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_non_string_mapping_key_is_invalid_config() -> Res<()> {
        // A YAML mapping whose key is not a string cannot be represented as
        // JSON, so the schema validator's serde YAML→JSON conversion fails.
        // This used to escape as an opaque `Error::Json`; it now surfaces as
        // the same `InvalidWorkflowsConfig` variant a schema violation does, so
        // both malformed-config shapes classify identically everywhere.
        let yaml: YamlValue = serde_yaml::from_str(
            r"
1: not-a-string-key
version: '1'
",
        )?;

        let err = WorkflowsConfig::from_yaml(&yaml).unwrap_err();
        assert!(
            matches!(
                err,
                Error::RemoteCatalog(RemoteCatalogError::InvalidWorkflowsConfig(_))
            ),
            "expected InvalidWorkflowsConfig, got: {err:?}"
        );

        Ok(())
    }

    #[test]
    fn test_workflows_config_parse_rich() -> Res<()> {
        // Fixture mirroring a sandbox bucket: several workflows with names and
        // descriptions, a declared default, and the required flag set true.
        let yaml: YamlValue = serde_yaml::from_str(
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
    metadata_schema: alpha-schema
schemas:
  alpha-schema:
    url: s3://sandbox/schemas/alpha.json
"#,
        )?;

        let config = WorkflowsConfig::from_yaml(&yaml)?;

        assert_eq!(config.default_workflow, Some("dummy".to_string()));
        assert!(config.is_workflow_required);
        assert_eq!(
            config.workflows,
            vec![
                WorkflowInfo {
                    id: "dummy".to_string(),
                    name: Some("Dummy workflow".to_string()),
                    description: Some("Do nothing.".to_string()),
                },
                WorkflowInfo {
                    id: "alpha".to_string(),
                    name: Some("Alpha".to_string()),
                    description: Some("First workflow.".to_string()),
                },
            ]
        );

        Ok(())
    }

    #[test]
    fn test_workflows_config_required_defaults_true() -> Res<()> {
        // `is_workflow_required` absent → defaults to true (matches quilt3).
        let yaml: YamlValue = serde_yaml::from_str(
            r"
version: '1'
workflows:
  foo:
    name: Foo
    metadata_schema: bar
",
        )?;

        let config = WorkflowsConfig::from_yaml(&yaml)?;

        assert!(config.is_workflow_required);
        assert_eq!(config.default_workflow, None);

        Ok(())
    }

    #[test]
    fn test_workflows_config_required_explicit_false() -> Res<()> {
        // Explicit `is_workflow_required: false` → false.
        let yaml: YamlValue = serde_yaml::from_str(
            r"
version: '1'
is_workflow_required: false
workflows:
  foo:
    name: Foo
",
        )?;

        let config = WorkflowsConfig::from_yaml(&yaml)?;

        assert!(!config.is_workflow_required);
        assert_eq!(
            config.workflows,
            vec![WorkflowInfo {
                id: "foo".to_string(),
                name: Some("Foo".to_string()),
                description: None,
            }]
        );

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_fetch_workflows_config_for_bucket_present() -> Res<()> {
        let remote = MockRemote::default();
        let host = None;

        let uri: S3Uri = "s3://my-bucket/.quilt/workflows/config.yml".parse()?;
        let config = r"
version: '1'
default_workflow: foo
workflows:
  foo:
    name: Foo
";
        remote
            .put_object(&None, &uri, config.as_bytes().to_vec())
            .await?;

        let parsed = fetch_workflows_config_for_bucket(&remote, &host, "my-bucket")
            .await?
            .expect("config present → Some");
        assert_eq!(parsed.default_workflow, Some("foo".to_string()));
        assert_eq!(
            parsed.workflows,
            vec![WorkflowInfo {
                id: "foo".to_string(),
                name: Some("Foo".to_string()),
                description: None,
            }]
        );

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_fetch_workflows_config_for_bucket_absent() -> Res<()> {
        let remote = MockRemote::default();
        let host = None;

        // No config object for this bucket → None.
        let result = fetch_workflows_config_for_bucket(&remote, &host, "empty-bucket").await?;
        assert!(result.is_none());

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_fetch_workflow_rules() -> Res<()> {
        let remote = MockRemote::default();
        let host = None;
        let uri: S3Uri = "s3://any/.quilt/workflows/config.yml".parse()?;

        let config = r#"
version: "1"
workflows:
  foo:
    name: Foo
    handle_pattern: "^team/"
    is_message_required: true
    metadata_schema: meta
    entries_schema: entries
schemas:
  meta:
    url: s3://test-bucket/schemas/meta.json
  entries:
    url: s3://test-bucket/schemas/entries.json
"#;
        let meta_uri: S3Uri = "s3://test-bucket/schemas/meta.json".parse()?;
        let entries_uri: S3Uri = "s3://test-bucket/schemas/entries.json".parse()?;
        remote
            .put_object(&None, &uri, config.as_bytes().to_vec())
            .await?;
        remote
            .put_object(
                &None,
                &meta_uri,
                br#"{"type": "object", "required": ["owner"]}"#.to_vec(),
            )
            .await?;
        remote
            .put_object(&None, &entries_uri, br#"{"type": "array"}"#.to_vec())
            .await?;

        let (_, parsed) = fetch_workflows_config(&remote, &host, &uri).await?;
        let parsed = parsed.expect("config present");
        let rules = fetch_workflow_rules(&remote, &host, &parsed, "foo").await?;

        assert_eq!(
            rules,
            WorkflowRules {
                handle_pattern: Some("^team/".to_string()),
                is_message_required: true,
                metadata_schema: Some(serde_json::json!({
                    "type": "object", "required": ["owner"]
                })),
                entries_schema: Some(serde_json::json!({ "type": "array" })),
            }
        );

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_fetch_workflow_rules_no_schemas() -> Res<()> {
        // A workflow declaring neither schema nor the optional flags yields
        // empty rules: no fetch attempted, defaults applied.
        let remote = MockRemote::default();
        let host = None;
        let uri: S3Uri = "s3://any/.quilt/workflows/config.yml".parse()?;

        let config = r"
version: '1'
workflows:
  bare:
    name: Bare
";
        remote
            .put_object(&None, &uri, config.as_bytes().to_vec())
            .await?;

        let (_, parsed) = fetch_workflows_config(&remote, &host, &uri).await?;
        let parsed = parsed.expect("config present");
        let rules = fetch_workflow_rules(&remote, &host, &parsed, "bare").await?;

        assert_eq!(
            rules,
            WorkflowRules {
                handle_pattern: None,
                is_message_required: false,
                metadata_schema: None,
                entries_schema: None,
            }
        );

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_non_string_schema_key_rejected_by_config_schema() -> Res<()> {
        // `metadata_schema: ~` — the key IS present, just null. The config
        // schema types it as a string, so the whole config is now rejected at
        // load (before any resolution), with a violation naming the exact path
        // — a typo can no longer silently disable schema validation.
        let remote = MockRemote::default();
        let host = None;
        let uri: S3Uri = "s3://any/.quilt/workflows/config.yml".parse()?;

        let config = r"
version: '1'
workflows:
  foo:
    name: Foo
    metadata_schema: ~
";
        remote
            .put_object(&None, &uri, config.as_bytes().to_vec())
            .await?;

        let err = fetch_workflows_config(&remote, &host, &uri)
            .await
            .unwrap_err();

        assert!(matches!(
            err,
            Error::RemoteCatalog(RemoteCatalogError::InvalidWorkflowsConfig(_))
        ));
        let message = err.to_string();
        assert!(
            message.contains("does not satisfy the workflows config schema"),
            "unexpected message: {message}"
        );
        assert!(
            message.contains("/workflows/foo/metadata_schema"),
            "violation must name the offending path, got: {message}"
        );

        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn test_entry_view_non_utf8_logical_key_is_lossy() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;
        use std::path::PathBuf;

        use crate::checksum::ObjectHash;

        // A logical key with an invalid UTF-8 byte must project as the lossy
        // form (U+FFFD replacement character), not as a fabricated empty key.
        let row = ManifestRow {
            logical_key: PathBuf::from(OsStr::from_bytes(b"bad-\xFF.txt")),
            physical_key: "s3://bucket/bad".to_string(),
            hash: ObjectHash::default(),
            size: 1,
            meta: None,
        };
        let view = entry_view(&row);
        assert_eq!(view.logical_key, "bad-\u{FFFD}.txt");
    }

    #[test]
    fn test_from_optional_id() {
        assert_eq!(
            WorkflowIntent::from_optional_id(None),
            WorkflowIntent::BucketDefault
        );
        assert_eq!(
            WorkflowIntent::from_optional_id(Some("x")),
            WorkflowIntent::Named("x".to_string())
        );
        assert_eq!(
            WorkflowIntent::from_optional_id(Some("")),
            WorkflowIntent::BucketDefault
        );
        assert_eq!(
            WorkflowIntent::from_optional_id(Some("  ")),
            WorkflowIntent::BucketDefault
        );
        assert_eq!(
            WorkflowIntent::from_optional_id(Some("  x  ")),
            WorkflowIntent::Named("x".to_string())
        );
    }

    #[test]
    fn test_workflow_intent_serde_round_trip() -> Res<()> {
        for (intent, wire) in [
            (
                WorkflowIntent::BucketDefault,
                serde_json::json!({ "kind": "bucket-default" }),
            ),
            (
                WorkflowIntent::NoWorkflow,
                serde_json::json!({ "kind": "no-workflow" }),
            ),
            (
                WorkflowIntent::Named("foo".to_string()),
                serde_json::json!({ "kind": "named", "id": "foo" }),
            ),
        ] {
            assert_eq!(serde_json::to_value(&intent)?, wire);
            assert_eq!(serde_json::from_value::<WorkflowIntent>(wire)?, intent);
        }

        Ok(())
    }

    /// (a) A quoted `is_message_required: "true"` is a YAML string, not a bool.
    /// The lenient parser used to read it as `false` (rule silently off); the
    /// config schema now rejects it, naming the exact path.
    #[test]
    fn test_quoted_is_message_required_rejected_by_config_schema() {
        let yaml: YamlValue = serde_yaml::from_str(
            r#"
version: "1"
workflows:
  foo:
    name: Foo
    is_message_required: "true"
"#,
        )
        .expect("valid YAML");

        let err = WorkflowsConfig::from_yaml(&yaml).unwrap_err();
        assert!(matches!(
            err,
            Error::RemoteCatalog(RemoteCatalogError::InvalidWorkflowsConfig(_))
        ));
        let message = err.to_string();
        assert!(
            message.contains("does not satisfy the workflows config schema"),
            "unexpected message: {message}"
        );
        assert!(
            message.contains("/workflows/foo/is_message_required"),
            "violation must name the offending path, got: {message}"
        );
    }

    /// (b) A list `handle_pattern` is not the string the schema requires: the
    /// config is rejected at load with a violation naming the path.
    #[test]
    fn test_list_handle_pattern_rejected_by_config_schema() {
        let yaml: YamlValue = serde_yaml::from_str(
            r#"
version: "1"
workflows:
  foo:
    name: Foo
    handle_pattern: ["^team/"]
"#,
        )
        .expect("valid YAML");

        let err = WorkflowsConfig::from_yaml(&yaml).unwrap_err();
        assert!(matches!(
            err,
            Error::RemoteCatalog(RemoteCatalogError::InvalidWorkflowsConfig(_))
        ));
        let message = err.to_string();
        assert!(
            message.contains("/workflows/foo/handle_pattern"),
            "violation must name the offending path, got: {message}"
        );
    }

    /// (c) A fully-valid config — including `format: regex` / `format: uri`
    /// annotations the schema declares but quilt3 never asserts — still parses.
    #[test]
    fn test_valid_config_with_format_annotations_parses() -> Res<()> {
        let yaml: YamlValue = serde_yaml::from_str(
            r#"
version: "1"
is_workflow_required: true
default_workflow: foo
workflows:
  foo:
    name: Foo
    handle_pattern: "^team/"
    is_message_required: true
    metadata_schema: meta
schemas:
  meta:
    url: s3://bucket/schemas/meta.json
"#,
        )?;

        let config = WorkflowsConfig::from_yaml(&yaml)?;
        assert_eq!(config.default_workflow, Some("foo".to_string()));
        assert!(config.is_workflow_required);
        assert!(config.is_message_required("foo"));
        assert_eq!(config.handle_pattern("foo"), Some("^team/".to_string()));

        Ok(())
    }
}
