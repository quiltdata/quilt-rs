use serde_yaml::Mapping;
use serde_yaml::Value as YamlValue;
use tokio::io::AsyncReadExt;

use crate::io::remote::Remote;
use crate::io::storage::Storage;
use crate::manifest::Workflow;
use crate::manifest::WorkflowId;
use crate::uri::Namespace;
use crate::uri::S3Uri;
use crate::Error;
use crate::Res;

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
pub async fn resolve_workflow<S: Storage, R: Remote>(
    storage: &S,
    remote: &R,
    namespace: Namespace,
    workflow_id: Option<String>,
    lineage_reader: impl Fn() -> Res<Option<S3Uri>>,
) -> Res<Option<Workflow>> {
    let uri = match lineage_reader()? {
        Some(manifest_uri) => S3Uri {
            key: ".quilt/workflows/config.yml".to_string(),
            ..manifest_uri
        },
        None => return Err(Error::PackageNotInstalled(namespace)),
    };

    match remote.get_object_stream(&uri).await {
        Ok(stream) => {
            let mut bytes = Vec::new();
            stream
                .body
                .into_async_read()
                .read_to_end(&mut bytes)
                .await?;
            let yaml: YamlValue = serde_yaml::from_slice(&bytes)?;
            let schemas = yaml["schemas"].as_mapping().cloned().unwrap_or_default();

            Ok(Some(Workflow {
                config: stream.uri.to_string(),
                id: match &workflow_id {
                    Some(workflow_id_str) => {
                        // FIXME: get the schema_id from config.workflows[workflow_id_str]
                        if let Some(serde_yaml::Value::Mapping(schema)) =
                            schemas.get(workflow_id_str)
                        {
                            match schema.get("url") {
                                Some(serde_yaml::Value::String(url)) => Some(WorkflowId {
                                    id: workflow_id_str.to_string(),
                                    url: remote.resolve_url(&url.parse()?).await?,
                                }),
                                _ => {
                                    return Err(Error::Workflow(format!(
                                        "Schema URL not found for workflow ID: {}",
                                        workflow_id_str
                                    )))
                                }
                            }
                        } else {
                            return Err(Error::Workflow(format!(
                                "Schema URL not found for workflow ID: {}",
                                workflow_id_str
                            )));
                        }
                    }
                    None => None,
                },
            }))
        }
        Err(Error::S3(err_str)) => {
            if err_str.contains("NoSuchKey: The specified key does not exist") {
                match workflow_id {
                    Some(id) => Err(Error::Workflow(format!(
                        r#"There is no workflows config, but the workflow "{}" is set"#,
                        id
                    ))),
                    None => Ok(None),
                }
            } else {
                Err(Error::S3(err_str))
            }
        }
        Err(err) => Err(err),
    }
}
