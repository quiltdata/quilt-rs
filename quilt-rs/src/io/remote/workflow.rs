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
use crate::workflow::validate_package;
use quilt_uri::Host;
use quilt_uri::S3Uri;

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
/// Parsing is deliberately lenient: [`WorkflowsConfig::from_yaml`] never rejects
/// a malformed config (config-format validation is a separate slice). The typed
/// fields (`default_workflow`, `is_workflow_required`, `workflows`) surface what
/// the commit dialog needs, while the resolution helpers reproduce today's lazy,
/// ad-hoc digging by consulting the retained raw YAML — so a misconfiguration
/// only surfaces (and only as loudly as before) when a workflow is resolved.
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
    /// Never fails for well-formed YAML: unexpected shapes degrade to defaults
    /// rather than errors, preserving today's behaviour where malformed configs
    /// only bite at resolution time.
    pub fn from_yaml(yaml: &YamlValue) -> Res<WorkflowsConfig> {
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
    let Some(workflow) = workflow else {
        return Ok(());
    };
    let (_, config) = fetch_workflows_config(remote, host, &workflow.config).await?;
    let Some(config) = config else {
        return Ok(());
    };
    let rules = match &workflow.id {
        Some(workflow_id) => {
            Some(fetch_workflow_rules(remote, host, &config, &workflow_id.id).await?)
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
///   - `workflow_id` is `Some(id)` → hard error, mirroring quilt3's
///     "`{id}` workflow is specified, but no workflows config exist".
/// - Config exists:
///   - `workflow_id` is `Some(id)` missing from the current config → hard
///     error, mirroring quilt3's "There is no `{id}` workflow in config".
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
                "\"{id}\" workflow is specified, but no workflows config exist"
            )))),
        };
    };
    let rules = match workflow_id {
        Some(id) => {
            if config.workflow_entry(id).is_none() {
                return Err(Error::RemoteCatalog(RemoteCatalogError::Workflow(format!(
                    "There is no \"{id}\" workflow in config"
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
workflows:
  foo:
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
workflows:
  foo:
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
workflows:
  foo:
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
default_workflow: foo
workflows:
  foo:
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
default_workflow: ghost
workflows:
  foo:
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

        // `default_workflow` present but not a string → loud error.
        let config = r"
default_workflow: [not, a, string]
workflows:
  foo:
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
        assert!(err.to_string().contains("must be a string"));

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_bucket_default_explicit_null_key() -> Res<()> {
        let remote = MockRemote::default();
        let host = None;
        let uri: S3Uri = "s3://any/.quilt/workflows/config.yml".parse()?;

        // `default_workflow:` with no value is a YAML explicit null — not a string,
        // so the bucket-default intent errors loudly rather than governing silently.
        //
        // quilt3 agrees in direction: its config JSON schema types `default_workflow`
        // as a string and quilt3 schema-validates the whole config on load, so this
        // config fails every push there. quilt-rs rejecting only the bucket-default
        // intent is the narrower behavior.
        let config = r"
default_workflow:
workflows:
  foo:
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
        assert!(err.to_string().contains("must be a string"));

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
workflows:
  foo:
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
    async fn test_present_non_string_schema_key_is_an_honest_error() -> Res<()> {
        // `metadata_schema: ~` — the key IS present, just null. The error must
        // say the value is not a string, not claim the key was "not found".
        let remote = MockRemote::default();
        let host = None;
        let uri: S3Uri = "s3://any/.quilt/workflows/config.yml".parse()?;

        let config = r"
workflows:
  foo:
    metadata_schema: ~
";
        remote
            .put_object(&None, &uri, config.as_bytes().to_vec())
            .await?;

        let (_, parsed) = fetch_workflows_config(&remote, &host, &uri).await?;
        let parsed = parsed.expect("config present");
        let err = fetch_workflow_rules(&remote, &host, &parsed, "foo")
            .await
            .unwrap_err();

        assert!(matches!(
            err,
            Error::RemoteCatalog(RemoteCatalogError::Workflow(_))
        ));
        let message = err.to_string();
        assert!(
            message.contains("`metadata_schema` for workflow ID foo must be a string"),
            "unexpected message: {message}"
        );
        assert!(
            !message.contains("not found"),
            "unexpected message: {message}"
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
}
