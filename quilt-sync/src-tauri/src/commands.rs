//! Tauri IPC commands, grouped by domain. Everything is re-exported at
//! `commands::*` so `main.rs`'s `generate_handler!` list and other callers
//! keep the flat `commands::<name>` paths.
//!
//! # The `uri` argument some commands take but never use
//!
//! A package operation is addressed by its `namespace`; several also accept the
//! package `uri` the calling surface rendered from, and do nothing with it but
//! read its catalog for [telemetry attribution](crate::telemetry::Telemetry::track).
//! It is passed rather than resolved from local lineage on purpose: the catalog
//! the user acted on is what the analytics question asks about, and deriving it
//! here would add I/O and a failure path to every tracked action — including one
//! that would have to run *before* an uninstall destroys the lineage it reads.
//! See [`crate::telemetry::EventContext::for_uri`].

mod auth;
mod commit_data;
mod package_data;
mod package_list;
mod package_ops;
mod settings;
mod system;

pub use auth::*;
pub use commit_data::*;
pub use package_data::*;
pub use package_list::*;
pub use package_ops::*;
pub use settings::*;
pub use system::*;

#[cfg(test)]
mod role_switch_chain;
#[cfg(test)]
mod test_support;
