//! Tauri IPC commands, grouped by domain. Everything is re-exported at
//! `commands::*` so `main.rs`'s `generate_handler!` list and other callers
//! keep the flat `commands::<name>` paths.

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
mod test_support;
