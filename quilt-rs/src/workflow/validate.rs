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
//! - a schema may declare `$schema` only as the Draft-7 meta-schema URI
//!   (`http://json-schema.org/draft-07/schema#` — the sole entry in quilt3's
//!   `SUPPORTED_META_SCHEMAS`); any other value, or a non-string one, is a
//!   hard configuration error rather than a schema silently compiled as
//!   Draft-7; an absent `$schema` defaults to Draft-7, as in quilt3;
//! - `format` keywords are **annotation-only**: quilt3's `Draft7Validator`
//!   carries no `FormatChecker`, so a value like `{"format": "date"}` never
//!   asserts, matching this gate;
//! - `handle_pattern` is a **substring** match (the regex is not anchored),
//!   and is compiled with the Rust `regex` crate rather than Python's `re`:
//!   patterns using Python-only features (look-arounds like `(?!...)`,
//!   backreferences like `\1`) fail loudly here as `InvalidHandlePattern`
//!   instead of matching — an accepted divergence, since such patterns are
//!   rare and the failure is explicit rather than silent drift;
//! - entry *bytes* are never validated — only the logical key, size, and
//!   metadata of each entry.

use std::borrow::Cow;

use jsonschema::Validator;
use regex::Regex;
use serde_json::Value;
use serde_json::json;

use super::error::RuleViolation;
use super::error::SUPPORTED_META_SCHEMA;
use super::error::SchemaKind;
use super::error::Violations;
use super::error::WorkflowValidationError;

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
///
/// `logical_key` is a [`Cow`] because manifest logical keys are paths that may
/// not be valid UTF-8: a valid key borrows, a non-UTF-8 key is projected
/// lossily into an owned string.
#[derive(Debug, Clone, PartialEq)]
pub struct EntryView<'a> {
    pub logical_key: Cow<'a, str>,
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
            Err(WorkflowValidationError::Rejected(
                RuleViolation::WorkflowRequired.into(),
            ))
        } else {
            Ok(())
        };
    };

    let mut violations = Vec::new();
    check_field_rules(rules, package, &mut violations)?;
    check_entries_rule(rules, package, &mut violations)?;
    finish(violations)
}

/// Validate a candidate against a workflow's **field-level** rules —
/// message-required, `handle_pattern`, and `metadata_schema` — while
/// deliberately skipping the `entries_schema`.
///
/// This is the live-validation counterpart to [`validate_package`], for the
/// commit dialog's advisory feedback as the user types. A workflow is always
/// selected on that path, so `rules` is passed by value (never the "no
/// workflow / required" case [`validate_package`] handles with its
/// `is_workflow_required` argument).
///
/// The `entries_schema` is intentionally not checked here: projecting a
/// candidate's entries requires materialising the built manifest's rows (the
/// heavier flow machinery), so entries validation stays the commit gate's
/// responsibility — the authoritative check that still runs at commit time.
/// Field checks run in the same order [`validate_package`] uses, and failures
/// are collected together as [`WorkflowValidationError::Rejected`]; a
/// misconfigured gate (bad `metadata_schema` or `handle_pattern`)
/// short-circuits with the corresponding hard-error variant, exactly as the
/// full gate does.
pub fn validate_candidate_fields(
    rules: &WorkflowRules,
    package: &PackageCandidate<'_>,
) -> Result<(), WorkflowValidationError> {
    let mut violations = Vec::new();
    check_field_rules(rules, package, &mut violations)?;
    finish(violations)
}

/// The message / `handle_pattern` / `metadata_schema` checks shared by
/// [`validate_package`] and [`validate_candidate_fields`]. Appends any rule
/// failures to `violations`; a misconfigured schema or regex short-circuits
/// with a hard error.
fn check_field_rules(
    rules: &WorkflowRules,
    package: &PackageCandidate<'_>,
    violations: &mut Vec<RuleViolation>,
) -> Result<(), WorkflowValidationError> {
    if rules.is_message_required && package.message.is_none_or(str::is_empty) {
        violations.push(RuleViolation::MessageRequired);
    }

    if let Some(pattern) = &rules.handle_pattern {
        // quilt3 compiles handle_pattern with Python's `re`, which supports
        // look-around assertions (e.g. `(?!...)`) and backreferences (e.g.
        // `\1`). The Rust `regex` crate rejects those by design, so such a
        // pattern surfaces here as a loud `InvalidHandlePattern` rather than
        // matching. This is an accepted divergence: such patterns are rare,
        // and an explicit error is preferable to silent behavioral drift.
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

    Ok(())
}

/// The `entries_schema` check, run only by the full [`validate_package`] gate.
fn check_entries_rule(
    rules: &WorkflowRules,
    package: &PackageCandidate<'_>,
    violations: &mut Vec<RuleViolation>,
) -> Result<(), WorkflowValidationError> {
    if let Some(schema) = &rules.entries_schema {
        let validator = compile_schema(schema, SchemaKind::Entries)?;
        let entries = project_entries(package.entries);
        if let Some(reason) = collect_errors(&validator, &entries) {
            violations.push(RuleViolation::EntriesInvalid(reason));
        }
    }
    Ok(())
}

/// Turn a collected violation list into the gate's result: `Ok` when empty,
/// [`WorkflowValidationError::Rejected`] otherwise.
fn finish(violations: Vec<RuleViolation>) -> Result<(), WorkflowValidationError> {
    match Violations::from_nonempty(violations) {
        Some(violations) => Err(WorkflowValidationError::Rejected(violations)),
        None => Ok(()),
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
                    "logical_key": entry.logical_key.as_ref(),
                    "size": entry.size,
                    "meta": entry.meta.cloned().unwrap_or_else(|| json!({})),
                })
            })
            .collect(),
    )
}

/// Compile a schema document as Draft-7, rejecting any use of `$ref` first
/// (quilt3 forbids it; each schema must be self-contained), then any `$schema`
/// declaration other than the Draft-7 meta-schema (quilt3's
/// `SUPPORTED_META_SCHEMAS` accepts only that one URI; compiling e.g. a
/// 2020-12 schema as Draft-7 would silently mis-validate it).
fn compile_schema(schema: &Value, kind: SchemaKind) -> Result<Validator, WorkflowValidationError> {
    if contains_ref(schema) {
        return Err(WorkflowValidationError::UnsupportedRef { kind });
    }
    // Like quilt3, only an *object* schema is inspected for `$schema` (a bare
    // boolean schema has no meta-schema to declare). Absent `$schema` means
    // Draft-7, matching quilt3's default validator class.
    if let Value::Object(map) = schema
        && let Some(meta_schema) = map.get("$schema")
        && meta_schema.as_str() != Some(SUPPORTED_META_SCHEMA)
    {
        return Err(WorkflowValidationError::UnsupportedMetaSchema {
            kind,
            value: meta_schema.to_string(),
        });
    }
    build_annotation_only_draft7(schema).map_err(|err| WorkflowValidationError::InvalidSchema {
        kind,
        reason: err.to_string(),
    })
}

/// Compile the vendored workflows-config JSON Schema itself into a validator.
///
/// Uses the same annotation-only Draft-7 compile as [`compile_schema`], but
/// *without* its `$ref` / `$schema` guards: those guard **untrusted user**
/// metadata/entries schemas, whereas the config schema is quilt3's own trusted
/// document, which legitimately declares `$ref` (`#/definitions/Version`) and
/// `$schema`. This mirrors quilt3, whose config validator is a plain
/// `Draft7Validator(config-1.schema.json)` with no `FormatChecker` — so its
/// `format: regex` / `format: uri` annotations never assert, exactly as here.
///
/// Panics if the schema does not compile: the only caller feeds it the vendored
/// constant, so a failure is a build-time defect, not a runtime input error.
pub(crate) fn compile_config_schema(schema: &Value) -> Validator {
    build_annotation_only_draft7(schema)
        .expect("vendored workflows-config schema must compile as Draft-7")
}

/// Build a Draft-7 validator with `format` assertions disabled.
///
/// quilt3's `Draft7Validator` is built without a `FormatChecker`, so `format`
/// is annotation-only there. jsonschema 0.47 asserts `format` for Draft 7 by
/// default, so we disable it — otherwise we would falsely reject inputs (e.g. a
/// non-RFC-3339 `{"format": "date"}` value) that quilt3 accepts, which would
/// pause autosync. Shared by the user-schema gate and config-format validation
/// so there is a single quilt3-parity compile path.
fn build_annotation_only_draft7(
    schema: &Value,
) -> Result<Validator, jsonschema::ValidationError<'static>> {
    jsonschema::draft7::options()
        .should_validate_formats(false)
        .build(schema)
}

/// Whether the schema uses `$ref` anywhere (as an object key at any depth).
///
/// Mirrors quilt3's `_schema_load_object_hook`, which rejects `$ref` as an
/// object key at any depth — so rejecting a data property literally named
/// `$ref` is exact parity, not a bug.
fn contains_ref(value: &Value) -> bool {
    match value {
        Value::Object(map) => map.contains_key("$ref") || map.values().any(contains_ref),
        Value::Array(items) => items.iter().any(contains_ref),
        _ => false,
    }
}

/// Validate `instance` against `validator`, returning `None` when valid or a
/// single joined string of all failures when not.
fn collect_errors(validator: &Validator, instance: &Value) -> Option<String> {
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
            WorkflowValidationError::Rejected(v) if matches!(v.as_slice(), [RuleViolation::WorkflowRequired])
        ));
    }

    #[test]
    fn fully_valid_package_passes_cleanly() {
        let rules = strict_rules();
        let meta = valid_meta();
        let entry_meta = json!({ "k": "v" });
        let entries = [EntryView {
            logical_key: "data/a.csv".into(),
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
            logical_key: "a.txt".into(),
            size: 10,
            meta: None,
        }];
        let pkg = candidate("p", None, None, &small);
        assert!(validate_package(Some(&rules), false, &pkg).is_ok());

        let big = [EntryView {
            logical_key: "a.txt".into(),
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
                    if matches!(v.as_slice(), [RuleViolation::MessageRequired])
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
    fn format_keyword_is_annotation_only() {
        // quilt3 builds its Draft7Validator without a FormatChecker, so
        // `format` is annotation-only there — a value that violates the format
        // (but satisfies the base type) must still PASS our gate. Asserting
        // `format` would falsely reject packages quilt3 accepts and pause
        // autosync.
        let rules = WorkflowRules {
            handle_pattern: None,
            is_message_required: false,
            metadata_schema: Some(json!({
                "type": "object",
                "properties": { "Date": { "type": "string", "format": "date" } }
            })),
            entries_schema: None,
        };
        let entries = [];

        // "July 8, 2026" is not an RFC 3339 date, but it is a string, so with
        // format as annotation-only the package is admissible.
        let meta = json!({ "Date": "July 8, 2026" });
        let pkg = candidate("p", None, Some(&meta), &entries);
        assert!(validate_package(Some(&rules), false, &pkg).is_ok());

        // Guard against the test passing because validation was skipped: a
        // genuine type violation on the same schema must still reject.
        let bad = json!({ "Date": 7 });
        let pkg = candidate("p", None, Some(&bad), &entries);
        let err = validate_package(Some(&rules), false, &pkg).unwrap_err();
        assert!(matches!(
            err,
            WorkflowValidationError::Rejected(v)
                if matches!(v.as_slice(), [RuleViolation::MetadataInvalid(_)])
        ));
    }

    #[test]
    fn draft7_meta_schema_declaration_is_accepted() {
        // The one URI in quilt3's SUPPORTED_META_SCHEMAS.
        let rules = WorkflowRules {
            handle_pattern: None,
            is_message_required: false,
            metadata_schema: Some(json!({
                "$schema": "http://json-schema.org/draft-07/schema#",
                "type": "object",
                "required": ["owner"]
            })),
            entries_schema: None,
        };
        let entries = [];
        let meta = valid_meta();
        let pkg = candidate("p", None, Some(&meta), &entries);
        assert!(validate_package(Some(&rules), false, &pkg).is_ok());
    }

    #[test]
    fn unsupported_meta_schema_is_a_hard_error() {
        // quilt3 refuses any meta-schema outside SUPPORTED_META_SCHEMAS
        // ("Unsupported meta-schema: ..."). Compiling a 2020-12 schema as
        // Draft-7 here would silently mis-validate it instead.
        let rules = WorkflowRules {
            handle_pattern: None,
            is_message_required: false,
            metadata_schema: Some(json!({
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "type": "object"
            })),
            entries_schema: None,
        };
        let entries = [];
        let meta = valid_meta();
        let pkg = candidate("p", None, Some(&meta), &entries);
        let err = validate_package(Some(&rules), false, &pkg).unwrap_err();
        assert!(matches!(
            err,
            WorkflowValidationError::UnsupportedMetaSchema {
                kind: SchemaKind::Metadata,
                ..
            }
        ));
        // The message must name the offending value.
        assert!(err.to_string().contains("2020-12"));
    }

    #[test]
    fn non_string_meta_schema_is_a_hard_error() {
        // quilt3: "$schema must be a string." — also a hard config error here.
        let rules = WorkflowRules {
            handle_pattern: None,
            is_message_required: false,
            metadata_schema: None,
            entries_schema: Some(json!({
                "$schema": 42,
                "type": "array"
            })),
        };
        let entries = [];
        let pkg = candidate("p", None, None, &entries);
        let err = validate_package(Some(&rules), false, &pkg).unwrap_err();
        assert!(matches!(
            err,
            WorkflowValidationError::UnsupportedMetaSchema {
                kind: SchemaKind::Entries,
                ..
            }
        ));
        assert!(err.to_string().contains("42"));
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

    // ── Field-level candidate validation (live commit-dialog path) ──

    #[test]
    fn candidate_fields_pass_when_message_meta_and_name_satisfy_rules() {
        // message present, handle matches, metadata valid → no violations, and
        // the entries_schema is NOT consulted (empty entries would violate a
        // strict entries_schema under the full gate — see the next test).
        let rules = strict_rules();
        let meta = valid_meta();
        let entries = [];
        let pkg = candidate("team/dataset", Some("msg"), Some(&meta), &entries);
        assert!(validate_candidate_fields(&rules, &pkg).is_ok());
    }

    #[test]
    fn candidate_fields_skip_entries_schema() {
        // An entries_schema requiring a non-empty array would reject empty
        // entries under the full gate, but the field-level path never checks
        // it — so the same empty-entries candidate passes here.
        let rules = WorkflowRules {
            handle_pattern: None,
            is_message_required: false,
            metadata_schema: None,
            entries_schema: Some(json!({ "type": "array", "minItems": 1 })),
        };
        let entries = [];
        let pkg = candidate("p", None, None, &entries);
        // Full gate rejects (empty array violates minItems)…
        assert!(validate_package(Some(&rules), false, &pkg).is_err());
        // …but the field-level path ignores the entries_schema entirely.
        assert!(validate_candidate_fields(&rules, &pkg).is_ok());
    }

    #[test]
    fn candidate_fields_flag_missing_message() {
        let rules = WorkflowRules {
            handle_pattern: None,
            is_message_required: true,
            metadata_schema: None,
            entries_schema: None,
        };
        let entries = [];
        // Both an absent and an empty message are rejected.
        for message in [None, Some("")] {
            let pkg = candidate("p", message, None, &entries);
            let err = validate_candidate_fields(&rules, &pkg).unwrap_err();
            assert!(matches!(
                err,
                WorkflowValidationError::Rejected(v) if matches!(v.as_slice(), [RuleViolation::MessageRequired])
            ));
        }
    }

    #[test]
    fn candidate_fields_flag_metadata_and_handle_together() {
        // A name that misses the handle_pattern and metadata missing a required
        // field are collected together, exactly as the full gate collects them.
        let rules = strict_rules();
        let bad_meta = json!({});
        let entries = [];
        let pkg = candidate("nope/data", Some("msg"), Some(&bad_meta), &entries);
        let WorkflowValidationError::Rejected(violations) =
            validate_candidate_fields(&rules, &pkg).unwrap_err()
        else {
            panic!("expected Rejected");
        };
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
    fn candidate_fields_absent_metadata_validated_as_empty_object() {
        // Mirrors the full gate: no user_meta validates as `{}`, so a schema
        // requiring a field rejects.
        let rules = WorkflowRules {
            handle_pattern: None,
            is_message_required: false,
            metadata_schema: Some(json!({ "type": "object", "required": ["owner"] })),
            entries_schema: None,
        };
        let entries = [];
        let pkg = candidate("p", None, None, &entries);
        let err = validate_candidate_fields(&rules, &pkg).unwrap_err();
        assert!(matches!(
            err,
            WorkflowValidationError::Rejected(v)
                if matches!(v.as_slice(), [RuleViolation::MetadataInvalid(_)])
        ));
    }

    #[test]
    fn candidate_fields_surface_metadata_schema_misconfig_as_hard_error() {
        // A metadata_schema using `$ref` is a gate misconfiguration; the
        // field-level path short-circuits with the same hard error the full
        // gate raises, so the dialog can still tell the user something is wrong.
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
        let pkg = candidate("p", Some("m"), Some(&meta), &entries);
        let err = validate_candidate_fields(&rules, &pkg).unwrap_err();
        assert!(matches!(
            err,
            WorkflowValidationError::UnsupportedRef {
                kind: SchemaKind::Metadata
            }
        ));
    }
}
