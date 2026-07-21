//!
//! Workflow metadata attached to a manifest header.

use std::collections::BTreeMap;
use std::collections::HashMap;

use serde::Deserialize;
use serde::Serialize;

use quilt_uri::S3Uri;
use quilt_uri::UriError;

#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowId {
    pub id: String,
    /// The content-addressed schemas the workflow declares, keyed by schema id
    /// (the key in the config's top-level `schemas:` section) → version-pinned
    /// object URL. Mirrors quilt3's `data_to_store['schemas']`: one entry per
    /// declared schema (`metadata_schema` and/or `entries_schema`), deduped by
    /// id, and empty when the workflow declares none. A `BTreeMap` keeps the
    /// in-memory order deterministic — the wire form is an unordered object and
    /// the top-hash re-sorts keys, but determinism here removes any read-side
    /// ambiguity.
    pub schemas: BTreeMap<String, S3Uri>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Workflow {
    pub config: S3Uri,
    pub id: Option<WorkflowId>,
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

        let id = match helper.id {
            None => None,
            Some(id) => {
                // Keep every declared schema — quilt3 stamps one entry per
                // schema the workflow references, so dropping any (as an
                // arbitrary single-entry pick would) forks the top-hash.
                let mut schemas = BTreeMap::new();
                for (schema_id, schema_url) in helper.schemas.unwrap_or_default() {
                    let url = schema_url
                        .parse()
                        .map_err(|_| serde::de::Error::custom(UriError::S3(schema_url.clone())))?;
                    schemas.insert(schema_id, url);
                }
                Some(WorkflowId { id, schemas })
            }
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
            // Omit `schemas` entirely when the workflow declares none, matching
            // quilt3's `if self.loaded_schemas:` guard — a schema-less workflow
            // stamps as `{config, id}`.
            let emit_schemas = !workflow_id.schemas.is_empty();
            let mut state =
                serializer.serialize_struct("Workflow", 2 + usize::from(emit_schemas))?;
            state.serialize_field("config", &self.config.to_string())?;
            state.serialize_field("id", &workflow_id.id)?;
            if emit_schemas {
                let schemas: BTreeMap<&str, String> = workflow_id
                    .schemas
                    .iter()
                    .map(|(id, url)| (id.as_str(), url.to_string()))
                    .collect();
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

    type Res = Result<(), Box<dyn std::error::Error>>;

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
                schemas: BTreeMap::from([(
                    "test-schema".to_string(),
                    "s3://bucket/workflows/test.json".parse()?
                )]),
            })
        );
        Ok(())
    }

    #[test]
    fn test_workflow_deserialization_two_schemas() -> Res {
        // A quilt3-authored stamp for a workflow declaring both a
        // metadata_schema and an entries_schema: both must survive the
        // round-trip (the old single-schema model dropped one).
        let json = r#"{
            "config": "s3://workflow/config",
            "id": "dual",
            "schemas": {
                "meta-schema": "s3://bucket/workflows/meta.json",
                "entries-schema": "s3://bucket/workflows/entries.json"
            }
        }"#;

        let workflow: Workflow = serde_json::from_str(json)?;

        assert_eq!(
            workflow.id,
            Some(WorkflowId {
                id: "dual".to_string(),
                schemas: BTreeMap::from([
                    (
                        "meta-schema".to_string(),
                        "s3://bucket/workflows/meta.json".parse()?
                    ),
                    (
                        "entries-schema".to_string(),
                        "s3://bucket/workflows/entries.json".parse()?
                    ),
                ]),
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
                schemas: BTreeMap::from([(
                    "test-schema".to_string(),
                    "s3://bucket/workflows/test.json".parse()?,
                )]),
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
    fn test_workflow_serialization_two_schemas() -> Res {
        let workflow = Workflow {
            config: "s3://workflow/config".parse()?,
            id: Some(WorkflowId {
                id: "dual".to_string(),
                schemas: BTreeMap::from([
                    (
                        "meta-schema".to_string(),
                        "s3://bucket/workflows/meta.json".parse()?,
                    ),
                    (
                        "entries-schema".to_string(),
                        "s3://bucket/workflows/entries.json".parse()?,
                    ),
                ]),
            }),
        };

        let json = serde_json::to_value(&workflow).unwrap();

        assert_eq!(
            json,
            serde_json::json!({
                "config": "s3://workflow/config",
                "id": "dual",
                "schemas": {
                    "meta-schema": "s3://bucket/workflows/meta.json",
                    "entries-schema": "s3://bucket/workflows/entries.json"
                }
            })
        );
        Ok(())
    }

    #[test]
    fn test_workflow_serialization_no_schemas_omits_field() -> Res {
        // A workflow that declares no schema stamps as `{config, id}` — the
        // `schemas` key is absent, not an empty object.
        let workflow = Workflow {
            config: "s3://workflow/config".parse()?,
            id: Some(WorkflowId {
                id: "schema-less".to_string(),
                schemas: BTreeMap::new(),
            }),
        };

        let json = serde_json::to_value(&workflow).unwrap();

        assert_eq!(
            json,
            serde_json::json!({
                "config": "s3://workflow/config",
                "id": "schema-less"
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
