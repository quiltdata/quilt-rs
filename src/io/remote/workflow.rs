use serde_yaml::Value as YamlValue;
use tokio::io::AsyncReadExt;

use crate::io::remote::Remote;
use crate::manifest::Workflow;
use crate::manifest::WorkflowId;
use crate::uri::Host;
use crate::uri::S3Uri;
use crate::Error;
use crate::Res;

fn get_schema_id(yaml: &YamlValue, workflow_id: &str) -> Res<Option<String>> {
    match &yaml.get("workflows") {
        Some(YamlValue::Mapping(workflows)) => match &workflows.get(workflow_id) {
            Some(YamlValue::Mapping(workflow)) => match &workflow.get("metadata_schema") {
                Some(YamlValue::String(schema_id)) => Ok(Some(schema_id.clone())),
                None => Ok(None),
                _ => Err(Error::Workflow(format!(
                    "`metadata_schema` not found for workflow ID: {}",
                    workflow_id
                ))),
            },
            _ => Err(Error::Workflow(format!(
                "Workflow {} not found in workflows/config.yaml",
                workflow_id
            ))),
        },
        _ => Err(Error::Workflow(
            "Workflows not found in workflows/config.yaml".to_string(),
        )),
    }
}

async fn get_schema_url<R: Remote>(
    remote: &R,
    host: &Option<Host>,
    yaml: YamlValue,
    workflow_id: &str,
) -> Res<Option<S3Uri>> {
    match get_schema_id(&yaml, workflow_id)? {
        Some(schema_id) => match &yaml.get("schemas") {
            Some(YamlValue::Mapping(schemas)) => match &schemas.get(&schema_id) {
                Some(YamlValue::Mapping(schema)) => match &schema.get("url") {
                    Some(YamlValue::String(url)) => {
                        Ok(Some(remote.resolve_url(host, &url.parse()?).await?))
                    }
                    _ => Err(Error::Workflow(format!(
                        "Schema {} doesn't have URL",
                        schema_id
                    ))),
                },
                _ => Err(Error::Workflow(format!(
                    "Schema {}, referenced by workflow {} not found in workflows/config.yaml",
                    schema_id, workflow_id,
                ))),
            },
            _ => Err(Error::Workflow(
                "Schemas not found in workflows/config.yaml".to_string(),
            )),
        },
        None => Ok(None),
    }
}

async fn fetch_workflows_config<R: Remote>(
    remote: &R,
    host: &Option<Host>,
    uri: &S3Uri,
) -> Res<(S3Uri, Option<YamlValue>)> {
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
        Err(Error::S3(err_str)) => {
            if err_str.contains("NoSuchKey") {
                Ok((uri.clone(), None))
            } else {
                Err(Error::S3(err_str))
            }
        }
        Err(err) => Err(err),
    }
}

/// 1. No `workflows/config.yaml`
///
///    Return `None` or `Err`
///
///    1.a. `workflow_id` is null/None              → None
///    1.b. `workflow_id` is set                    → Err
///
/// 2. `workflows/config.yaml` is present
///
///    Return `Some(Workflow)` with `config` property
///    And the `id` is:
///
///    2.a. `workflow_id` is set and valid          → Some(WorkflowId)
///    2.b. `workflow_id` is null/None              → None
///    2.c. `workflow_id` is set but not found      → Err
///    2.d. `workflow_id` is "" (edge case for 2.c) → Err
pub async fn resolve_workflow<R: Remote>(
    remote: &R,
    host: &Option<Host>,
    workflow_id: Option<String>,
    uri: &S3Uri,
) -> Res<Option<Workflow>> {
    let (config, yaml) = fetch_workflows_config(remote, host, uri).await?;
    match yaml {
        Some(yaml) => match workflow_id {
            Some(id) => {
                let metadata_url = get_schema_url(remote, host, yaml, &id).await?;
                Ok(Some(Workflow {
                    config,
                    id: Some(WorkflowId { id, metadata_url }),
                }))
            }
            None => Ok(Some(Workflow { config, id: None })),
        },
        None => match workflow_id {
            Some(workflow_id) => Err(Error::Workflow(format!(
                "There is no workflows config, but the workflow \"{}\" is set",
                workflow_id
            ))),
            None => Ok(None),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::remote::mocks::MockRemote;

    #[tokio::test]
    async fn test_no_config_yaml() -> Res<()> {
        let remote = MockRemote::default();
        let host = None;
        let uri: S3Uri = "s3://any/.quilt/workflows/config.yml".parse()?;

        // Case 1.a: No config.yaml and workflow_id is None
        let result = resolve_workflow(&remote, &host, None, &uri).await?;
        assert!(result.is_none());

        // Case 1.b: No config.yaml but workflow_id is set
        let err = resolve_workflow(&remote, &host, Some("test-workflow".to_string()), &uri)
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Workflow(_)));
        assert!(err.to_string().contains("There is no workflows config"));

        Ok(())
    }

    #[tokio::test]
    async fn test_with_config_yaml() -> Res<()> {
        let remote = MockRemote::default();
        let host = None;
        let uri: S3Uri = "s3://any/.quilt/workflows/config.yml".parse()?;

        // Put test config.yaml into mock remote storage
        let config = r#"
workflows:
  foo:
    metadata_schema: bar
schemas:
  bar:
    url: s3://test-bucket/schemas/test.json
"#;
        let schema_uri: S3Uri = "s3://test-bucket/schemas/test.json".parse()?;
        let schema = b"{}";
        remote
            .put_object(&None, &uri, config.as_bytes().to_vec())
            .await?;
        remote
            .put_object(&None, &schema_uri, schema.to_vec())
            .await?;

        // Case 2.a: Config exists, workflow_id is set and valid
        let result = resolve_workflow(&remote, &host, Some("foo".to_string()), &uri)
            .await?
            .unwrap();
        assert_eq!(result.config, uri);
        assert_eq!(
            result.id.unwrap(),
            WorkflowId {
                id: "foo".to_string(),
                metadata_url: Some("s3://test-bucket/schemas/test.json".parse()?)
            }
        );

        // Case 2.b: Config exists but workflow_id is None
        let result = resolve_workflow(&remote, &host, None, &uri).await?.unwrap();
        assert_eq!(result.config, uri);
        assert!(result.id.is_none());

        // Case 2.c: Config exists but workflow_id is not found
        let err = resolve_workflow(&remote, &host, Some("non-existent".to_string()), &uri)
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Workflow(_)));
        assert!(err.to_string().contains("Workflow non-existent not found"));

        // Case 2.d: Config exists but workflow_id is empty
        let err = resolve_workflow(&remote, &host, Some("".to_string()), &uri)
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Workflow(_)));
        assert!(err.to_string().contains("Workflow  not found"));

        Ok(())
    }
}
