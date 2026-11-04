//! What only the desktop app can do, as the engine asks for it.
//!
//! The desktop app implements [`Desktop`] over Tauri and hands it to every
//! command call. A caller without a desktop passes none, and the commands that
//! need one answer [`NOT_AVAILABLE`].

use std::path::Path;
use std::path::PathBuf;

use async_trait::async_trait;
use serde::Serialize;

use crate::engine::Result;

/// A link type the OS opens with another program, or with none.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForeignHandler {
    pub scheme: String,
    /// The program the OS opens these links with; `None` when there is none.
    pub program: Option<String>,
}

/// What a command that needs the desktop app answers without one.
pub const NOT_AVAILABLE: &str = "not available in the browser";

#[async_trait]
pub trait Desktop: Send + Sync {
    /// Show the native folder picker, starting in `start`. `None` when the
    /// user cancels.
    async fn pick_folder(&self, start: &Path) -> Result<Option<PathBuf>>;

    fn copy_to_clipboard(&self, text: &str) -> Result;

    /// Download and install the update, then restart.
    async fn install_update(&self) -> std::result::Result<(), String>;

    fn quit(&self);

    /// The link types the OS opens with another program, found at startup.
    /// Empty when every one opens this copy of the app.
    fn foreign_link_handlers(&self) -> Vec<ForeignHandler>;
}
