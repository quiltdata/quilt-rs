use serde_yaml::Value as YamlValue;
use tokio::io::AsyncReadExt;

use crate::io::remote::Remote;
use crate::manifest::Workflow;
use crate::manifest::WorkflowId;
use crate::uri::S3Uri;
use crate::Error;
use crate::Res;

fn get_schema_id(yaml: &YamlValue, workflow_id: &str) -> Res<String> {
    match &yaml["workflows"] {
        YamlValue::Mapping(workflows) => match &workflows[workflow_id] {
            YamlValue::Mapping(workflow) => match &workflow["metadata_schema"] {
                YamlValue::String(schema_id) => Ok(schema_id.clone()),
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

async fn get_schema_url<R: Remote>(remote: &R, yaml: YamlValue, workflow_id: &str) -> Res<S3Uri> {
    let schema_id = get_schema_id(&yaml, workflow_id)?;
    match &yaml["schemas"] {
        YamlValue::Mapping(schemas) => match &schemas[&schema_id] {
            YamlValue::Mapping(schema) => match &schema["url"] {
                YamlValue::String(url) => Ok(remote.resolve_url(&url.parse()?).await?),
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
    }
}

async fn fetch_workflows_config<R: Remote>(
    remote: &R,
    uri: S3Uri,
) -> Res<(S3Uri, Option<YamlValue>)> {
    match remote.get_object_stream(&uri).await {
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
            if err_str.contains("NoSuchKey: The specified key does not exist") {
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
    workflow_id: Option<String>,
    uri: S3Uri,
) -> Res<Option<Workflow>> {
    let (uri, yaml) = fetch_workflows_config(remote, uri).await?;
    match yaml {
        Some(yaml) => match workflow_id {
            Some(id) => Ok(Some(Workflow {
                config: uri,
                id: Some(WorkflowId {
                    id: id.clone(),
                    url: get_schema_url(remote, yaml, &id).await?,
                }),
            })),
            None => Ok(Some(Workflow {
                config: uri,
                id: None,
            })),
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
