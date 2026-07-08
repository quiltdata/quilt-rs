//! The pure workflow gate: given a workflow's rules and the candidate
//! package projected to what the gate inspects, decide whether the package
//! is admissible.
//!
//! This module performs **no I/O**. Schema documents arrive already-fetched
//! as [`serde_json::Value`]; fetching lives in `crate::io::remote`. The only
//! dependencies are `serde_json`, the `jsonschema` validator, and `regex`
//! (for `handle_pattern`) — so the module is a candidate for extraction into a
//! standalone `quilt-workflow` crate that also compiles to `wasm32`.
//!
//! Semantics mirror quilt3's client-side gate
//! (`quilt3.workflows.WorkflowValidator`):
//!
//! - metadata / entries schemas are **Draft-7** JSON Schema;
//! - a schema may **not** use `$ref` (each schema must be self-contained);
//! - `handle_pattern` is a **substring** match (the regex is not anchored);
//! - entry *bytes* are never validated — only the logical key, size, and
//!   metadata of each entry.

use std::fmt;

use regex::Regex;
use serde_json::Value;
use serde_json::json;
use thiserror::Error;

/// Which schema in a workflow a configuration problem refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemaKind {
    Metadata,
    Entries,
}

impl fmt::Display for SchemaKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SchemaKind::Metadata => f.write_str("metadata_schema"),
            SchemaKind::Entries => f.write_str("entries_schema"),
        }
    }
}

/// A single reason a candidate package fails its workflow gate. Several may
/// apply to one package; they are reported together in
/// [`WorkflowValidationError::Rejected`].
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RuleViolation {
    #[error("a workflow is required by this bucket, but none was selected")]
    WorkflowRequired,

    #[error("a commit message is required by this workflow, but none was provided")]
    MessageRequired,

    #[error("package name {name:?} does not match the required handle_pattern {pattern:?}")]
    HandleMismatch { name: String, pattern: String },

    #[error("package metadata does not satisfy the workflow's metadata_schema: {0}")]
    MetadataInvalid(String),

    #[error("package entries do not satisfy the workflow's entries_schema: {0}")]
    EntriesInvalid(String),
}

/// The outcome of running the gate against a candidate package.
///
/// A [`WorkflowValidationError::Rejected`] means the package is well-formed
/// but breaks one or more rules — the caller should surface the violations to
/// the user. The other variants mean the *gate itself* is misconfigured (a
/// schema is not valid Draft-7, uses `$ref`, or `handle_pattern` is not a
/// valid regex) and are hard errors distinct from a rule failure.
#[derive(Debug, Error)]
pub enum WorkflowValidationError {
    #[error("workflow {kind} is not a valid Draft-7 JSON Schema: {reason}")]
    InvalidSchema { kind: SchemaKind, reason: String },

    #[error("workflow {kind} uses `$ref`, which is not supported")]
    UnsupportedRef { kind: SchemaKind },

    #[error("workflow handle_pattern {pattern:?} is not a valid regular expression: {reason}")]
    InvalidHandlePattern { pattern: String, reason: String },

    #[error("package does not satisfy the workflow:{}", render_violations(.0))]
    Rejected(Vec<RuleViolation>),
}

fn render_violations(violations: &[RuleViolation]) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    for violation in violations {
        let _ = write!(out, "\n  - {violation}");
    }
    out
}

/// The rules a single workflow imposes, with any referenced schema documents
/// already fetched. Built by `crate::io::remote` and handed to
/// [`validate_package`].
#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowRules {
    /// Unanchored regex the package name must match (substring semantics).
    pub handle_pattern: Option<String>,
    /// Whether a non-empty commit message is required.
    pub is_message_required: bool,
    /// Draft-7 schema constraining the package-level user metadata.
    pub metadata_schema: Option<Value>,
    /// Draft-7 schema constraining the list of projected entries.
    pub entries_schema: Option<Value>,
}

/// A single package entry projected to exactly what the gate inspects:
/// logical key, size, and metadata. Entry *bytes* are never inspected.
#[derive(Debug, Clone, PartialEq)]
pub struct EntryView<'a> {
    pub logical_key: &'a str,
    pub size: u64,
    pub meta: Option<&'a Value>,
}

/// A candidate package projected to the surface the gate validates.
#[derive(Debug, Clone, PartialEq)]
pub struct PackageCandidate<'a> {
    /// The package name (handle), e.g. `team/dataset`.
    pub name: &'a str,
    /// The commit message, if any.
    pub message: Option<&'a str>,
    /// Package-level user metadata, if any.
    pub user_meta: Option<&'a Value>,
    /// The package entries, projected to logical key / size / metadata.
    pub entries: &'a [EntryView<'a>],
}

/// Run the workflow gate against a candidate package.
///
/// `rules` is the selected workflow's rules, or `None` when the caller
/// selected no workflow. `is_workflow_required` is the bucket config's
/// required-or-not flag: with no workflow selected and the flag set, the
/// package is rejected.
///
/// Returns `Ok(())` when the package is admissible. Rule failures are
/// collected and returned together as [`WorkflowValidationError::Rejected`];
/// a misconfigured gate (bad schema or regex) short-circuits with the
/// corresponding hard-error variant.
pub fn validate_package(
    rules: Option<&WorkflowRules>,
    is_workflow_required: bool,
    package: &PackageCandidate<'_>,
) -> Result<(), WorkflowValidationError> {
    let Some(rules) = rules else {
        return if is_workflow_required {
            Err(WorkflowValidationError::Rejected(vec![
                RuleViolation::WorkflowRequired,
            ]))
        } else {
            Ok(())
        };
    };

    let mut violations = Vec::new();

    if rules.is_message_required && package.message.is_none_or(str::is_empty) {
        violations.push(RuleViolation::MessageRequired);
    }

    if let Some(pattern) = &rules.handle_pattern {
        let regex =
            Regex::new(pattern).map_err(|err| WorkflowValidationError::InvalidHandlePattern {
                pattern: pattern.clone(),
                reason: err.to_string(),
            })?;
        if !regex.is_match(package.name) {
            violations.push(RuleViolation::HandleMismatch {
                name: package.name.to_string(),
                pattern: pattern.clone(),
            });
        }
    }

    if let Some(schema) = &rules.metadata_schema {
        let validator = compile_schema(schema, SchemaKind::Metadata)?;
        let empty = json!({});
        let meta = package.user_meta.unwrap_or(&empty);
        if let Some(reason) = collect_errors(&validator, meta) {
            violations.push(RuleViolation::MetadataInvalid(reason));
        }
    }

    if let Some(schema) = &rules.entries_schema {
        let validator = compile_schema(schema, SchemaKind::Entries)?;
        let entries = project_entries(package.entries);
        if let Some(reason) = collect_errors(&validator, &entries) {
            violations.push(RuleViolation::EntriesInvalid(reason));
        }
    }

    if violations.is_empty() {
        Ok(())
    } else {
        Err(WorkflowValidationError::Rejected(violations))
    }
}

/// Project entries to the JSON array quilt3 validates: one
/// `{logical_key, size, meta}` object per entry, with an empty object for a
/// missing metadata.
fn project_entries(entries: &[EntryView<'_>]) -> Value {
    Value::Array(
        entries
            .iter()
            .map(|entry| {
                json!({
                    "logical_key": entry.logical_key,
                    "size": entry.size,
                    "meta": entry.meta.cloned().unwrap_or_else(|| json!({})),
                })
            })
            .collect(),
    )
}

/// Compile a schema document as Draft-7, rejecting any use of `$ref` first
/// (quilt3 forbids it; each schema must be self-contained).
fn compile_schema(
    schema: &Value,
    kind: SchemaKind,
) -> Result<jsonschema::Validator, WorkflowValidationError> {
    if contains_ref(schema) {
        return Err(WorkflowValidationError::UnsupportedRef { kind });
    }
    jsonschema::draft7::new(schema).map_err(|err| WorkflowValidationError::InvalidSchema {
        kind,
        reason: err.to_string(),
    })
}

/// Whether the schema uses `$ref` anywhere (as an object key at any depth).
fn contains_ref(value: &Value) -> bool {
    match value {
        Value::Object(map) => map.contains_key("$ref") || map.values().any(contains_ref),
        Value::Array(items) => items.iter().any(contains_ref),
        _ => false,
    }
}

/// Validate `instance` against `validator`, returning `None` when valid or a
/// single joined string of all failures when not.
fn collect_errors(validator: &jsonschema::Validator, instance: &Value) -> Option<String> {
    let messages: Vec<String> = validator
        .iter_errors(instance)
        .map(|err| format!("{err} (at {})", err.instance_path()))
        .collect();
    if messages.is_empty() {
        None
    } else {
        Some(messages.join("; "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_log::test;

    /// A workflow that requires: a message, a `team/...` handle, an object
    /// metadata with a required `owner` string, and entries each carrying a
    /// non-negative integer `size` and an object `meta`.
    fn strict_rules() -> WorkflowRules {
        WorkflowRules {
            handle_pattern: Some("^team/".to_string()),
            is_message_required: true,
            metadata_schema: Some(json!({
                "type": "object",
                "required": ["owner"],
                "properties": { "owner": { "type": "string" } }
            })),
            entries_schema: Some(json!({
                "type": "array",
                "items": {
                    "type": "object",
                    "required": ["logical_key", "size", "meta"],
                    "properties": {
                        "logical_key": { "type": "string" },
                        "size": { "type": "integer", "maximum": 100 },
                        "meta": { "type": "object" }
                    }
                }
            })),
        }
    }

    fn valid_meta() -> Value {
        json!({ "owner": "alice" })
    }

    fn candidate<'a>(
        name: &'a str,
        message: Option<&'a str>,
        user_meta: Option<&'a Value>,
        entries: &'a [EntryView<'a>],
    ) -> PackageCandidate<'a> {
        PackageCandidate {
            name,
            message,
            user_meta,
            entries,
        }
    }

    #[test]
    fn no_workflow_not_required_passes() {
        let entries = [];
        let pkg = candidate("anything/goes", None, None, &entries);
        assert!(validate_package(None, false, &pkg).is_ok());
    }

    #[test]
    fn no_workflow_required_is_rejected() {
        let entries = [];
        let pkg = candidate("anything/goes", None, None, &entries);
        let err = validate_package(None, true, &pkg).unwrap_err();
        assert!(matches!(
            err,
            WorkflowValidationError::Rejected(v) if v == vec![RuleViolation::WorkflowRequired]
        ));
    }

    #[test]
    fn fully_valid_package_passes_cleanly() {
        let rules = strict_rules();
        let meta = valid_meta();
        let entry_meta = json!({ "k": "v" });
        let entries = [EntryView {
            logical_key: "data/a.csv",
            size: 42,
            meta: Some(&entry_meta),
        }];
        let pkg = candidate(
            "team/dataset",
            Some("initial commit"),
            Some(&meta),
            &entries,
        );
        assert!(validate_package(Some(&rules), true, &pkg).is_ok());
    }

    #[test]
    fn metadata_pass_and_fail() {
        let rules = WorkflowRules {
            handle_pattern: None,
            is_message_required: false,
            metadata_schema: Some(json!({
                "type": "object",
                "required": ["owner"],
                "properties": { "owner": { "type": "string" } }
            })),
            entries_schema: None,
        };
        let entries = [];

        let ok_meta = json!({ "owner": "bob" });
        let pkg = candidate("p", None, Some(&ok_meta), &entries);
        assert!(validate_package(Some(&rules), false, &pkg).is_ok());

        let bad_meta = json!({ "owner": 7 });
        let pkg = candidate("p", None, Some(&bad_meta), &entries);
        let err = validate_package(Some(&rules), false, &pkg).unwrap_err();
        assert!(matches!(
            err,
            WorkflowValidationError::Rejected(v)
                if matches!(v.as_slice(), [RuleViolation::MetadataInvalid(_)])
        ));
    }

    #[test]
    fn absent_metadata_validated_as_empty_object() {
        // quilt3 validates the package metadata as `{}` when none is set, so a
        // schema requiring a field must reject a package with no user_meta.
        let rules = WorkflowRules {
            handle_pattern: None,
            is_message_required: false,
            metadata_schema: Some(json!({
                "type": "object",
                "required": ["owner"]
            })),
            entries_schema: None,
        };
        let entries = [];
        let pkg = candidate("p", None, None, &entries);
        let err = validate_package(Some(&rules), false, &pkg).unwrap_err();
        assert!(matches!(
            err,
            WorkflowValidationError::Rejected(v)
                if matches!(v.as_slice(), [RuleViolation::MetadataInvalid(_)])
        ));
    }

    #[test]
    fn entries_pass_and_fail() {
        let rules = WorkflowRules {
            handle_pattern: None,
            is_message_required: false,
            metadata_schema: None,
            entries_schema: Some(json!({
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": { "size": { "type": "integer", "maximum": 100 } }
                }
            })),
        };

        let small = [EntryView {
            logical_key: "a.txt",
            size: 10,
            meta: None,
        }];
        let pkg = candidate("p", None, None, &small);
        assert!(validate_package(Some(&rules), false, &pkg).is_ok());

        let big = [EntryView {
            logical_key: "a.txt",
            size: 999,
            meta: None,
        }];
        let pkg = candidate("p", None, None, &big);
        let err = validate_package(Some(&rules), false, &pkg).unwrap_err();
        assert!(matches!(
            err,
            WorkflowValidationError::Rejected(v)
                if matches!(v.as_slice(), [RuleViolation::EntriesInvalid(_)])
        ));
    }

    #[test]
    fn handle_pattern_match_and_miss() {
        let rules = WorkflowRules {
            handle_pattern: Some("^team/".to_string()),
            is_message_required: false,
            metadata_schema: None,
            entries_schema: None,
        };
        let entries = [];

        let pkg = candidate("team/data", None, None, &entries);
        assert!(validate_package(Some(&rules), false, &pkg).is_ok());

        let pkg = candidate("other/data", None, None, &entries);
        let err = validate_package(Some(&rules), false, &pkg).unwrap_err();
        assert!(matches!(
            err,
            WorkflowValidationError::Rejected(v)
                if matches!(v.as_slice(), [RuleViolation::HandleMismatch { .. }])
        ));
    }

    #[test]
    fn handle_pattern_is_a_substring_match() {
        // The pattern is unanchored, so a fragment matches anywhere in the
        // name — the non-obvious quilt3 quirk. `staging` admits
        // `team/staging-2024` even though it is neither a prefix nor the whole
        // name.
        let rules = WorkflowRules {
            handle_pattern: Some("staging".to_string()),
            is_message_required: false,
            metadata_schema: None,
            entries_schema: None,
        };
        let entries = [];

        let pkg = candidate("team/staging-2024", None, None, &entries);
        assert!(validate_package(Some(&rules), false, &pkg).is_ok());

        let pkg = candidate("team/prod", None, None, &entries);
        assert!(validate_package(Some(&rules), false, &pkg).is_err());
    }

    #[test]
    fn message_required() {
        let rules = WorkflowRules {
            handle_pattern: None,
            is_message_required: true,
            metadata_schema: None,
            entries_schema: None,
        };
        let entries = [];

        let pkg = candidate("p", Some("has a message"), None, &entries);
        assert!(validate_package(Some(&rules), false, &pkg).is_ok());

        for message in [None, Some("")] {
            let pkg = candidate("p", message, None, &entries);
            let err = validate_package(Some(&rules), false, &pkg).unwrap_err();
            assert!(matches!(
                err,
                WorkflowValidationError::Rejected(v)
                    if v == vec![RuleViolation::MessageRequired]
            ));
        }
    }

    #[test]
    fn multiple_violations_are_collected() {
        let rules = strict_rules();
        let entries = []; // entries_schema allows an empty array
        // Missing message, wrong handle, and metadata missing `owner`.
        let bad_meta = json!({});
        let pkg = candidate("nope/data", None, Some(&bad_meta), &entries);
        let err = validate_package(Some(&rules), true, &pkg).unwrap_err();
        let WorkflowValidationError::Rejected(violations) = err else {
            panic!("expected Rejected, got {err:?}");
        };
        assert!(violations.contains(&RuleViolation::MessageRequired));
        assert!(
            violations
                .iter()
                .any(|v| matches!(v, RuleViolation::HandleMismatch { .. }))
        );
        assert!(
            violations
                .iter()
                .any(|v| matches!(v, RuleViolation::MetadataInvalid(_)))
        );
    }

    #[test]
    fn ref_in_schema_is_unsupported() {
        let rules = WorkflowRules {
            handle_pattern: None,
            is_message_required: false,
            metadata_schema: Some(json!({
                "type": "object",
                "properties": { "owner": { "$ref": "#/definitions/x" } }
            })),
            entries_schema: None,
        };
        let entries = [];
        let meta = valid_meta();
        let pkg = candidate("p", None, Some(&meta), &entries);
        let err = validate_package(Some(&rules), false, &pkg).unwrap_err();
        assert!(matches!(
            err,
            WorkflowValidationError::UnsupportedRef {
                kind: SchemaKind::Metadata
            }
        ));
    }

    #[test]
    fn invalid_schema_is_a_hard_error() {
        let rules = WorkflowRules {
            handle_pattern: None,
            is_message_required: false,
            metadata_schema: None,
            entries_schema: Some(json!({ "type": 123 })),
        };
        let entries = [];
        let pkg = candidate("p", None, None, &entries);
        let err = validate_package(Some(&rules), false, &pkg).unwrap_err();
        assert!(matches!(
            err,
            WorkflowValidationError::InvalidSchema {
                kind: SchemaKind::Entries,
                ..
            }
        ));
    }

    #[test]
    fn invalid_handle_pattern_is_a_hard_error() {
        let rules = WorkflowRules {
            handle_pattern: Some("(unclosed".to_string()),
            is_message_required: false,
            metadata_schema: None,
            entries_schema: None,
        };
        let entries = [];
        let pkg = candidate("p", None, None, &entries);
        let err = validate_package(Some(&rules), false, &pkg).unwrap_err();
        assert!(matches!(
            err,
            WorkflowValidationError::InvalidHandlePattern { .. }
        ));
    }
}
