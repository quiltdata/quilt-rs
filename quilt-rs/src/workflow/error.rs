//! Errors produced by the workflow gate and the workflows-config model.

use std::fmt;
use std::ops::Deref;

use thiserror::Error;

/// The only `$schema` meta-schema quilt3 accepts (its `SUPPORTED_META_SCHEMAS`
/// maps exactly this URI to `Draft7Validator`).
pub(super) const SUPPORTED_META_SCHEMA: &str = "http://json-schema.org/draft-07/schema#";

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

/// The reasons a candidate package failed its workflow gate — always at least
/// one. The inner list is private, so a `Violations` can only be built non-empty
/// (via [`Violations::from_nonempty`] or `From<RuleViolation>`); read access is
/// through the slice `Deref` (`.iter()`, `.contains()`, `&v[..]`), and the
/// multi-line rendering lives here as `Display`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violations(Vec<RuleViolation>);

impl Violations {
    /// Build from a list, or `None` when it is empty — a rejection must carry a
    /// reason, so an empty `Violations` is unrepresentable.
    #[must_use]
    pub fn from_nonempty(list: Vec<RuleViolation>) -> Option<Self> {
        (!list.is_empty()).then_some(Self(list))
    }
}

impl Deref for Violations {
    type Target = [RuleViolation];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl fmt::Display for Violations {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for violation in self.iter() {
            write!(f, "\n  - {violation}")?;
        }
        Ok(())
    }
}

impl From<RuleViolation> for Violations {
    fn from(violation: RuleViolation) -> Self {
        Violations(vec![violation])
    }
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

    #[error(
        "workflow {kind} declares `$schema`: {value}, which is not supported \
         (only the Draft-7 meta-schema {SUPPORTED_META_SCHEMA:?} is supported)"
    )]
    UnsupportedMetaSchema { kind: SchemaKind, value: String },

    #[error("workflow handle_pattern {pattern:?} is not a valid regular expression: {reason}")]
    InvalidHandlePattern { pattern: String, reason: String },

    #[error("package does not satisfy the workflow:{0}")]
    Rejected(Violations),
}

/// Errors from parsing / validating a workflows config, or resolving the
/// declared (unfetched) schema URLs within it.
///
/// The variants mirror the `quilt_rs::RemoteCatalogError` variants they map
/// onto, so a config error surfaced through quilt-rs keeps its exact `Display`.
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("Workflow error: {0}")]
    Workflow(String),

    /// The `.quilt/workflows/config.yml` is malformed — it violates the vendored
    /// quilt3 config schema, or its YAML could not be converted for validation.
    #[error("Invalid workflows config: {0}")]
    InvalidWorkflowsConfig(String),

    #[error(transparent)]
    Uri(#[from] quilt_uri::UriError),
}
