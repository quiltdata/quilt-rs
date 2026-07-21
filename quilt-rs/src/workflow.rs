//! Pure, I/O-free workflow logic for Quilt packages.
//!
//! This module owns the workflow **domain**, with no dependency on `aws-sdk`,
//! `tokio`, or the `Remote` trait, so it can be lifted into a standalone
//! `quilt-workflow` crate (it is written to compile to `wasm32` for live
//! client-side validation in the UI). It has three parts:
//!
//! - the **gate** ([`validate_package`], [`validate_candidate_fields`]) — given
//!   a workflow's rules and a candidate package, decide whether the package is
//!   admissible;
//! - the **config model** ([`WorkflowsConfig`]) — a typed, schema-validated view
//!   of `.quilt/workflows/config.yml`, plus [`WorkflowIntent`] and the
//!   display-facing [`WorkflowInfo`] / [`WorkflowSchemaUris`];
//! - the **header stamp** ([`Workflow`], [`WorkflowId`]) — the workflow reference
//!   recorded in a manifest header.
//!
//! Fetching config and schema documents from a remote, and version-resolving
//! schema URLs, live in `crate::io::remote`; this module only consumes
//! already-fetched documents.

mod config;
mod stamp;
mod validate;

pub use config::ConfigError;
pub use config::WORKFLOWS_CONFIG_KEY;
pub use config::WorkflowInfo;
pub use config::WorkflowIntent;
pub use config::WorkflowSchemaUris;
pub use config::WorkflowsConfig;
pub use stamp::Workflow;
pub use stamp::WorkflowId;
pub use validate::EntryView;
pub use validate::PackageCandidate;
pub use validate::RuleViolation;
pub use validate::SchemaKind;
pub use validate::WorkflowRules;
pub use validate::WorkflowValidationError;
pub use validate::validate_candidate_fields;
pub use validate::validate_package;
