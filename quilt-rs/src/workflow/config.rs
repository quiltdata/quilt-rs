//! Typed, schema-validated view of `.quilt/workflows/config.yml`, plus the
//! caller intent and display-facing helpers.
//!
//! This module is **pure**: it parses and validates an already-fetched YAML
//! document and answers questions about it. Fetching the config from a remote,
//! and version-resolving the schema URLs a workflow declares, live in
//! `quilt-rs` (`quilt_rs::io::remote`).

use std::sync::LazyLock;

use jsonschema::Validator;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use serde_yaml::Value as YamlValue;

use super::error::ConfigError;
use super::validate::compile_config_schema;
use quilt_uri::S3Uri;

/// The workflows-config JSON Schema, vendored byte-identical from quilt3
/// (`quilt3/workflows/config-1.schema.json`). quilt3 validates every loaded
/// `.quilt/workflows/config.yml` against this with a plain `Draft7Validator`
/// (`quilt3.workflows._get_conf_validator`) and refuses a malformed config, so
/// we do the same — otherwise a YAML typo (e.g. a quoted `is_message_required`)
/// would silently disable a rule the bucket owner believes is enforced.
const CONFIG_SCHEMA: &str = include_str!("config-1.schema.json");

/// The bucket key of the workflows config object every governed bucket carries.
/// The single source of truth for `.quilt/workflows/config.yml` across the
/// workspace, so the address is built the same way everywhere (quilt-sync
/// imports it too).
pub const WORKFLOWS_CONFIG_KEY: &str = ".quilt/workflows/config.yml";

/// The compiled config-schema validator. Built once: the schema is a
/// compile-time constant, so compilation cannot fail on user input.
static CONFIG_VALIDATOR: LazyLock<Validator> = LazyLock::new(|| {
    let schema: Value = serde_json::from_str(CONFIG_SCHEMA)
        .expect("vendored workflows-config schema is valid JSON");
    compile_config_schema(&schema)
});

/// Validate a decoded `config.yml` document against the vendored quilt3 config
/// schema, exactly as quilt3 does at load. On any violation, fails with a
/// [`ConfigError::InvalidWorkflowsConfig`] (classified as a conflict, not a
/// transient, by the sync watcher) naming every offending path — so a malformed
/// config refuses everywhere rather than half-working.
fn validate_config_document(yaml: &YamlValue) -> Result<(), ConfigError> {
    use std::fmt::Write;

    // `serde_yaml::Value` is `Serialize`, so this reuses serde's own YAML→JSON
    // conversion rather than a hand-rolled walker. A YAML document JSON cannot
    // represent (e.g. a non-string mapping key) is itself an invalid config, so
    // route the conversion failure into the same variant rather than letting it
    // escape as an opaque JSON error.
    let document: Value = serde_json::to_value(yaml).map_err(|err| {
        ConfigError::InvalidWorkflowsConfig(format!(
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
        Err(ConfigError::InvalidWorkflowsConfig(format!(
            "workflows/config.yml does not satisfy the workflows config schema:{message}"
        )))
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

/// The declared object URIs of the schemas a workflow references, for building
/// "Open in catalog" links in the commit dialog. Each is `None` when the
/// workflow declares no such schema.
///
/// These are the raw URIs declared in the config's `schemas` section — read
/// from the retained YAML with no I/O and no version resolution, so a link
/// points at the current object rather than a version pinned at fetch time.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WorkflowSchemaUris {
    pub metadata_schema: Option<S3Uri>,
    pub entries_schema: Option<S3Uri>,
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
    pub fn from_yaml(yaml: &YamlValue) -> Result<WorkflowsConfig, ConfigError> {
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

    /// Whether the config declares a workflow with this id. The pure counterpart
    /// of the old `workflow_entry(id).is_some()` check, exposed so quilt-rs can
    /// gate on presence without reaching the retained raw YAML.
    pub(crate) fn has_workflow(&self, workflow_id: &str) -> bool {
        self.workflow_entry(workflow_id).is_some()
    }

    /// The `handle_pattern` regex declared by a workflow, if any. Lenient:
    /// a non-string value degrades to `None`, matching the parser's stance.
    pub(crate) fn handle_pattern(&self, workflow_id: &str) -> Option<String> {
        self.workflow_entry(workflow_id)?
            .get("handle_pattern")
            .and_then(YamlValue::as_str)
            .map(String::from)
    }

    /// A workflow's `is_message_required` flag; defaults to `false` (matches quilt3).
    pub(crate) fn is_message_required(&self, workflow_id: &str) -> bool {
        self.workflow_entry(workflow_id)
            .and_then(|workflow| workflow.get("is_message_required"))
            .and_then(YamlValue::as_bool)
            .unwrap_or(false)
    }

    /// The schema id a workflow declares under `key` (`metadata_schema` or
    /// `entries_schema`), mirroring the legacy lazy lookup (including its
    /// error variants) exactly.
    pub(crate) fn schema_id(
        &self,
        workflow_id: &str,
        key: &str,
    ) -> Result<Option<String>, ConfigError> {
        match self.raw.get("workflows") {
            Some(YamlValue::Mapping(workflows)) => match workflows.get(workflow_id) {
                Some(YamlValue::Mapping(workflow)) => match workflow.get(key) {
                    Some(YamlValue::String(schema_id)) => Ok(Some(schema_id.clone())),
                    // Absent key: the workflow simply declares no such schema.
                    None => Ok(None),
                    // Present but not a string (explicit null, mapping, list):
                    // a misconfiguration, reported as such — not as "not found".
                    Some(_) => Err(ConfigError::Workflow(format!(
                        "`{key}` for workflow ID {workflow_id} must be a string"
                    ))),
                },
                _ => Err(ConfigError::Workflow(format!(
                    "Workflow {workflow_id} not found in workflows/config.yaml"
                ))),
            },
            _ => Err(ConfigError::Workflow(
                "Workflows not found in workflows/config.yaml".to_string(),
            )),
        }
    }

    /// The declared (unresolved) URL of a schema by its id, read straight from
    /// the `schemas` section — no I/O, no version resolution. `workflow_id` is
    /// used only for error context.
    pub(crate) fn declared_schema_url(
        &self,
        workflow_id: &str,
        schema_id: &str,
    ) -> Result<S3Uri, ConfigError> {
        match self.raw.get("schemas") {
            Some(YamlValue::Mapping(schemas)) => match schemas.get(schema_id) {
                Some(YamlValue::Mapping(schema)) => match schema.get("url") {
                    Some(YamlValue::String(url)) => Ok(url.parse()?),
                    _ => Err(ConfigError::Workflow(format!(
                        "Schema {schema_id} doesn't have URL"
                    ))),
                },
                _ => Err(ConfigError::Workflow(format!(
                    "Schema {schema_id}, referenced by workflow {workflow_id} not found in workflows/config.yaml",
                ))),
            },
            _ => Err(ConfigError::Workflow(
                "Schemas not found in workflows/config.yaml".to_string(),
            )),
        }
    }

    /// The declared object URI of the schema a workflow references under `key`
    /// (`metadata_schema` / `entries_schema`), or `None` when the workflow
    /// declares no such schema. Pure: resolved from the retained raw config.
    fn declared_schema_uri(
        &self,
        workflow_id: &str,
        key: &str,
    ) -> Result<Option<S3Uri>, ConfigError> {
        match self.schema_id(workflow_id, key)? {
            Some(schema_id) => Ok(Some(self.declared_schema_url(workflow_id, &schema_id)?)),
            None => Ok(None),
        }
    }

    /// The declared object URIs of a workflow's `metadata_schema` and
    /// `entries_schema`, for building "Open in catalog" links in the commit
    /// dialog.
    ///
    /// Each key resolves **independently**: a dangling reference (a declared
    /// schema id with no matching `schemas` entry, a missing `schemas` section)
    /// or an unknown `workflow_id` yields `None` for THAT key only, never
    /// suppressing the other. This is a lenient, display-only accessor — a
    /// broken link should drop that one link, not the whole dialog, so it never
    /// errors. Gate paths must NOT use it: they need misconfiguration to surface
    /// loudly (see `declared_schema_uri`).
    #[must_use]
    pub fn schema_uris(&self, workflow_id: &str) -> WorkflowSchemaUris {
        WorkflowSchemaUris {
            metadata_schema: self
                .declared_schema_uri(workflow_id, "metadata_schema")
                .ok()
                .flatten(),
            entries_schema: self
                .declared_schema_uri(workflow_id, "entries_schema")
                .ok()
                .flatten(),
        }
    }

    /// Interpret the top-level `default_workflow` key for the bucket-default intent.
    ///
    /// - key absent → `Ok(None)`: caller produces a null-id record.
    /// - string → `Ok(Some(id))`: caller resolves it like a named workflow.
    /// - anything else (including explicit null) → `Err`: misconfiguration.
    pub(crate) fn bucket_default_id(&self) -> Result<Option<String>, ConfigError> {
        match self.raw.get("default_workflow") {
            None => Ok(None),
            Some(YamlValue::String(id)) => Ok(Some(id.clone())),
            Some(_) => Err(ConfigError::Workflow(
                "`default_workflow` in workflows/config.yaml must be a string".to_string(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_log::test;

    type Res<T = ()> = Result<T, Box<dyn std::error::Error>>;

    #[test]
    fn test_non_string_mapping_key_is_invalid_config() -> Res<()> {
        // A YAML mapping whose key is not a string cannot be represented as
        // JSON, so the schema validator's serde YAML→JSON conversion fails.
        // This used to escape as an opaque JSON error; it now surfaces as the
        // same `InvalidWorkflowsConfig` variant a schema violation does, so both
        // malformed-config shapes classify identically everywhere.
        let yaml: YamlValue = serde_yaml::from_str(
            r"
1: not-a-string-key
version: '1'
",
        )?;

        let err = WorkflowsConfig::from_yaml(&yaml).unwrap_err();
        assert!(
            matches!(err, ConfigError::InvalidWorkflowsConfig(_)),
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
        assert!(matches!(err, ConfigError::InvalidWorkflowsConfig(_)));
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
        assert!(matches!(err, ConfigError::InvalidWorkflowsConfig(_)));
        let message = err.to_string();
        assert!(
            message.contains("/workflows/foo/handle_pattern"),
            "violation must name the offending path, got: {message}"
        );
    }

    #[test]
    fn test_schema_uris_both_one_none_and_unknown() -> Res<()> {
        // A config with three workflows: one declaring both schemas, one
        // declaring only the metadata schema, and one declaring neither.
        let yaml: YamlValue = serde_yaml::from_str(
            r#"
version: "1"
workflows:
  both:
    name: Both
    metadata_schema: meta
    entries_schema: entries
  meta-only:
    name: Meta only
    metadata_schema: meta
  none:
    name: None
schemas:
  meta:
    url: s3://schemas-bucket/meta.json
  entries:
    url: s3://schemas-bucket/entries.json
"#,
        )?;
        let config = WorkflowsConfig::from_yaml(&yaml)?;

        // Both declared → both URIs resolved from the `schemas` section.
        assert_eq!(
            config.schema_uris("both"),
            WorkflowSchemaUris {
                metadata_schema: Some("s3://schemas-bucket/meta.json".parse()?),
                entries_schema: Some("s3://schemas-bucket/entries.json".parse()?),
            }
        );
        // Only the metadata schema declared → entries is `None`.
        assert_eq!(
            config.schema_uris("meta-only"),
            WorkflowSchemaUris {
                metadata_schema: Some("s3://schemas-bucket/meta.json".parse()?),
                entries_schema: None,
            }
        );
        // Neither declared → both `None`, the type's default.
        assert_eq!(config.schema_uris("none"), WorkflowSchemaUris::default());
        // An unknown workflow id is not a link we can build: the lenient,
        // display-only accessor yields both `None` rather than erroring.
        assert_eq!(config.schema_uris("ghost"), WorkflowSchemaUris::default());

        Ok(())
    }

    #[test]
    fn test_schema_uris_resolve_independently() -> Res<()> {
        // A workflow with a resolvable `metadata_schema` but an `entries_schema`
        // whose id dangles (no matching `schemas` entry): the metadata link
        // still resolves, the dangling entries link degrades to `None` on its
        // own — a broken reference must not suppress the sibling link.
        let yaml: YamlValue = serde_yaml::from_str(
            r#"
version: "1"
workflows:
  partial:
    name: Partial
    metadata_schema: meta
    entries_schema: ghost
schemas:
  meta:
    url: s3://schemas-bucket/meta.json
"#,
        )?;
        let config = WorkflowsConfig::from_yaml(&yaml)?;

        assert_eq!(
            config.schema_uris("partial"),
            WorkflowSchemaUris {
                metadata_schema: Some("s3://schemas-bucket/meta.json".parse()?),
                entries_schema: None,
            }
        );

        Ok(())
    }

    #[test]
    fn test_schema_uris_missing_schemas_section_degrades_to_none() -> Res<()> {
        // A workflow declaring a schema id, but no `schemas` section to resolve
        // it against → the dangling reference degrades to `None` (display-only
        // leniency) rather than the hard error the async resolver raises.
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

        assert_eq!(config.schema_uris("foo"), WorkflowSchemaUris::default());

        Ok(())
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
