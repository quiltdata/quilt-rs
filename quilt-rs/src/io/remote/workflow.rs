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

fn get_schema_id(yaml: &YamlValue, workflow_id: &str) -> Res<Option<String>> {
    match &yaml.get("workflows") {
        Some(YamlValue::Mapping(workflows)) => match &workflows.get(workflow_id) {
            Some(YamlValue::Mapping(workflow)) => match &workflow.get("metadata_schema") {
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

async fn get_schema_url<R: Remote>(
    remote: &R,
    host: &Option<Host>,
    yaml: &YamlValue,
    workflow_id: &str,
) -> Res<Option<(String, S3Uri)>> {
    match get_schema_id(yaml, workflow_id)? {
        Some(schema_id) => match &yaml.get("schemas") {
            Some(YamlValue::Mapping(schemas)) => match &schemas.get(&schema_id) {
                Some(YamlValue::Mapping(schema)) => match &schema.get("url") {
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

async fn fetch_workflows_config<R: Remote>(
    remote: &R,
    host: &Option<Host>,
    uri: &S3Uri,
) -> Res<(S3Uri, Option<YamlValue>)> {
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
            Ok((stream.uri, serde_yaml::from_slice(&bytes)?))
        }
        Err(err) => Err(err),
    }
}

/// Resolve a named workflow id against an already-fetched config, attaching the
/// referenced metadata schema when the workflow declares one.
async fn resolve_named<R: Remote>(
    remote: &R,
    host: &Option<Host>,
    yaml: &YamlValue,
    config: S3Uri,
    id: String,
) -> Res<Option<Workflow>> {
    if let Some((metadata_id, metadata_url)) = get_schema_url(remote, host, yaml, &id).await? {
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
    let (config, yaml) = fetch_workflows_config(remote, host, uri).await?;
    match (yaml, intent) {
        (Some(yaml), WorkflowIntent::Named(id)) => {
            resolve_named(remote, host, &yaml, config, id).await
        }
        (None, WorkflowIntent::Named(id)) => {
            Err(Error::RemoteCatalog(RemoteCatalogError::Workflow(format!(
                "There is no workflows config, but the workflow \"{id}\" is set"
            ))))
        }
        (Some(_), WorkflowIntent::NoWorkflow) => Ok(Some(Workflow { config, id: None })),
        (None, WorkflowIntent::NoWorkflow) => Ok(None),
        (None, WorkflowIntent::BucketDefault) => Ok(None),
        (Some(yaml), WorkflowIntent::BucketDefault) => match yaml.get("default_workflow") {
            None => Ok(Some(Workflow { config, id: None })),
            Some(YamlValue::String(id)) => {
                let id = id.clone();
                resolve_named(remote, host, &yaml, config, id).await
            }
            Some(_) => Err(Error::RemoteCatalog(RemoteCatalogError::Workflow(
                "`default_workflow` in workflows/config.yaml must be a string".to_string(),
            ))),
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

        Ok(())
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
