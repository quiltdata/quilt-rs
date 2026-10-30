//! Everything `QuiltSync` does that isn't the desktop window: the model, the
//! commands, autosync, the file watcher, settings and telemetry.
//!
//! Nothing here names Tauri or reaches outside this folder (`just
//! check-engine`), so it can become its own crate by moving the folder.

pub mod app;
pub mod autopull;
pub mod changelog;
pub mod commands;
pub mod commit_message;
pub mod design_settings;
pub mod desktop;
pub mod dispatch;
pub mod env;
pub mod error;
pub mod events;
pub mod experimental_settings;
pub mod fswatcher;
pub mod lock;
pub mod log_settings;
pub mod model;
pub mod navigation;
pub mod notify;
pub mod oauth;
pub mod publish_settings;
pub mod quilt;
pub mod quit;
pub mod routes;
pub mod server;
pub mod services;
pub mod start;
pub mod tasks;
pub mod telemetry;
pub mod toast;

pub use error::Error;

pub type Result<T = ()> = std::result::Result<T, Error>;
