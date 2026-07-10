//! The workflow quality gate: a pure, I/O-free validator that decides whether
//! a candidate package satisfies a bucket's workflow rules.
//!
//! This module deliberately avoids depending on `aws-sdk`, `tokio`, or the
//! `Remote` trait so it can later be lifted into a standalone `quilt-workflow`
//! crate (which would also serve live client-side validation in the UI).
//! Fetching a workflow's schema documents lives in `crate::io::remote`; here we
//! only consume the already-fetched documents.

mod validate;

pub use validate::EntryView;
pub use validate::PackageCandidate;
pub use validate::RuleViolation;
pub use validate::SchemaKind;
pub use validate::WorkflowRules;
pub use validate::WorkflowValidationError;
pub(crate) use validate::compile_config_schema;
pub use validate::validate_candidate_fields;
pub use validate::validate_package;
