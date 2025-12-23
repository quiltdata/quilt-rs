//!
//! Namespace contains helpers to work with manifest and its content (rows).

#[allow(clippy::module_inception)]
mod manifest;
mod row;
mod table;

pub use row::Header;
pub use row::Row;
pub use row::RowDisplay;
pub use table::Table;
pub use table::TopHasher;

pub use manifest::Manifest;
pub use manifest::ManifestHeader;
pub use manifest::ManifestRow;
pub use manifest::MetadataSchema;
pub use manifest::Workflow;
pub use manifest::WorkflowId;
