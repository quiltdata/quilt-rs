//! What only the desktop app can do, as the engine asks for it.
//!
//! The desktop app implements [`Desktop`] over Tauri and hands it to every
//! command call. A caller without a desktop passes none, and the commands that
//! need one answer [`NOT_AVAILABLE`].

use std::path::Path;
use std::path::PathBuf;

use async_trait::async_trait;

use crate::engine::Result;

/// What a command that needs the desktop app answers without one.
pub const NOT_AVAILABLE: &str = "not available in the browser";

#[async_trait]
pub trait Desktop: Send + Sync {
    /// Show the native folder picker, starting in `start`. `None` when the
    /// user cancels.
    async fn pick_folder(&self, start: &Path) -> Result<Option<PathBuf>>;

    fn copy_to_clipboard(&self, text: &str) -> Result;

    /// The version the update feed offers, if it is newer than this build.
    async fn check_for_update(&self) -> std::result::Result<Option<String>, String>;

    /// Download and install the update, then restart.
    async fn install_update(&self) -> std::result::Result<(), String>;

    fn quit(&self);
}
