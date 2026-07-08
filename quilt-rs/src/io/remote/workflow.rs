use serde::Deserialize;
use serde::Serialize;
use serde_yaml::Value as YamlValue;
use tokio::io::AsyncReadExt;

use crate::Error;
use crate::Res;
use crate::error::RemoteCatalogError;
use crate::io::remote::Remote;
use crate::manifest::MetadataSchema;
use crate::manifest::Workflow;
use crate::manifest::WorkflowId;
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

    /// Metadata-schema id declared by the named workflow, mirroring the legacy
    /// lazy lookup (including its error variants) exactly.
    fn schema_id(&self, workflow_id: &str) -> Res<Option<String>> {
        match self.raw.get("workflows") {
            Some(YamlValue::Mapping(workflows)) => match workflows.get(workflow_id) {
                Some(YamlValue::Mapping(workflow)) => match workflow.get("metadata_schema") {
                    Some(YamlValue::String(schema_id)) => Ok(Some(schema_id.clone())),
                    None => Ok(None),
                    _ => Err(Error::RemoteCatalog(RemoteCatalogError::Workflow(format!(
                        "`metadata_schema` not found for workflow ID: {workflow_id}"
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

    /// Resolve the URL of the schema referenced by the named workflow.
    async fn schema_url<R: Remote>(
        &self,
        remote: &R,
        host: &Option<Host>,
        workflow_id: &str,
    ) -> Res<Option<(String, S3Uri)>> {
        match self.schema_id(workflow_id)? {
            Some(schema_id) => match self.raw.get("schemas") {
                Some(YamlValue::Mapping(schemas)) => match schemas.get(&schema_id) {
                    Some(YamlValue::Mapping(schema)) => match schema.get("url") {
                        Some(YamlValue::String(url)) => Ok(Some((
                            schema_id,
                            remote.resolve_url(host, &url.parse()?).await?,
                        ))),
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
            },
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
