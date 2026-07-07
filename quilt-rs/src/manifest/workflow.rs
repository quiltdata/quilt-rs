//!
//! Workflow metadata attached to a manifest header.

use std::collections::HashMap;

use serde::Deserialize;
use serde::Serialize;

use crate::Error;
use quilt_uri::S3Uri;
use quilt_uri::UriError;

#[derive(Debug, Clone, PartialEq)]
pub struct MetadataSchema {
    pub id: String,
    pub url: S3Uri,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowId {
    pub id: String,
    pub metadata: Option<MetadataSchema>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Workflow {
    pub config: S3Uri,
    pub id: Option<WorkflowId>,
}

impl<'de> Deserialize<'de> for WorkflowId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Ok(WorkflowId {
            id: s,
            metadata: None, // This will be filled in from schemas
        })
    }
}

impl<'de> Deserialize<'de> for Workflow {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct WorkflowHelper {
            config: String,
            id: Option<String>,
            schemas: Option<HashMap<String, String>>,
        }

        let helper = WorkflowHelper::deserialize(deserializer)?;

        let id = match (helper.id, helper.schemas) {
            (None, _) => None,
            (Some(id), None) => Some(WorkflowId {
                id: id.clone(),
                metadata: None,
            }),
            (Some(id), Some(schemas)) => match schemas.iter().collect::<Vec<_>>().first() {
                Some((schema_id, schema_url)) => match schema_url.parse() {
                    Ok(url) => Some(WorkflowId {
                        id: id.clone(),
                        metadata: Some(MetadataSchema {
                            id: (*schema_id).clone(),
                            url,
                        }),
                    }),
                    Err(_) => {
                        return Err(serde::de::Error::custom(Error::Uri(UriError::S3(
                            (*schema_url).clone(),
                        ))));
                    }
                },
                None => None,
            },
        };

        Ok(Workflow {
            config: helper.config.parse().map_err(serde::de::Error::custom)?,
            id,
        })
    }
}

impl Serialize for Workflow {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        if let Some(workflow_id) = &self.id {
            let mut state = serializer.serialize_struct("Workflow", 3)?;
            state.serialize_field("config", &self.config.to_string())?;
            state.serialize_field("id", &workflow_id.id)?;
            if let Some(metadata) = &workflow_id.metadata {
                let mut schemas = HashMap::new();
                schemas.insert(metadata.id.clone(), metadata.url.to_string());
                state.serialize_field("schemas", &schemas)?;
            }
            state.end()
        } else {
            let mut state = serializer.serialize_struct("Workflow", 2)?;
            state.serialize_field("config", &self.config.to_string())?;
            state.serialize_field("id", &None::<String>)?;
            state.end()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_log::test;

    use crate::Res;

    #[test]
    fn test_workflow_deserialization() -> Res {
        let json = r#"{
            "config": "s3://workflow/config",
            "id": "test-workflow",
            "schemas": {
                "test-schema": "s3://bucket/workflows/test.json"
            }
        }"#;

        let workflow: Workflow = serde_json::from_str(json)?;

        assert_eq!(workflow.config, "s3://workflow/config".parse()?);
        assert_eq!(
            workflow.id,
            Some(WorkflowId {
                id: "test-workflow".to_string(),
                metadata: Some(MetadataSchema {
                    id: "test-schema".to_string(),
                    url: "s3://bucket/workflows/test.json".parse()?
                })
            })
        );
        Ok(())
    }

    #[test]
    fn test_workflow_deserialization_none() -> Res {
        let json = r#"{
            "config": "s3://workflow/config",
            "id": null
        }"#;

        let workflow: Workflow = serde_json::from_str(json)?;

        assert_eq!(workflow.config, "s3://workflow/config".parse()?);
        assert_eq!(workflow.id, None);
        Ok(())
    }

    #[test]
    fn test_workflow_serialization() -> Res {
        let workflow = Workflow {
            config: "s3://workflow/config".parse()?,
            id: Some(WorkflowId {
                id: "test-workflow".to_string(),
                metadata: Some(MetadataSchema {
                    id: "test-schema".to_string(),
                    url: "s3://bucket/workflows/test.json".parse()?,
                }),
            }),
        };

        let json = serde_json::to_value(&workflow).unwrap();

        assert_eq!(
            json,
            serde_json::json!({
                "config": "s3://workflow/config",
                "id": "test-workflow",
                "schemas": {
                    "test-schema": "s3://bucket/workflows/test.json"
                }
            })
        );
        Ok(())
    }

    #[test]
    fn test_workflow_serialization_none() -> Res {
        let workflow = Workflow {
            config: "s3://workflow/config".parse()?,
            id: None,
        };

        let json = serde_json::to_value(&workflow).unwrap();

        assert_eq!(
            json,
            serde_json::json!({
                "config": "s3://workflow/config",
                "id": null
            })
        );
        Ok(())
    }
}
