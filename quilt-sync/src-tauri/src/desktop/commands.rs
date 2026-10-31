//! What the desktop gives the engine's commands: the folder picker, the
//! clipboard, the updater and quitting.

use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use rfd::FileDialog;
use tauri::Manager;
use tauri::Runtime;
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_updater::UpdaterExt;

use crate::engine::Error;
use crate::engine::desktop::Desktop;
use crate::engine::error::DesktopError;

impl From<tauri::Error> for Error {
    fn from(e: tauri::Error) -> Self {
        Error::Desktop(DesktopError::Tauri(e.to_string()))
    }
}

/// The desktop features, over the Tauri app.
struct TauriDesktop<R: Runtime> {
    app: tauri::AppHandle<R>,
}

#[async_trait]
impl<R: Runtime> Desktop for TauriDesktop<R> {
    async fn pick_folder(&self, start: &Path) -> crate::engine::Result<Option<PathBuf>> {
        let window = self
            .app
            .get_webview_window(super::MAIN)
            .ok_or(DesktopError::Window)?;
        Ok(FileDialog::new()
            .set_directory(start)
            .set_parent(&window)
            .pick_folder())
    }

    fn copy_to_clipboard(&self, text: &str) -> crate::engine::Result {
        self.app
            .clipboard()
            .write_text(text)
            .map_err(|err| DesktopError::Clipboard(err.to_string()))?;
        Ok(())
    }

    async fn check_for_update(&self) -> Result<Option<String>, String> {
        let updater = self.app.updater().map_err(|e| e.to_string())?;
        let update = updater.check().await.map_err(|e| e.to_string())?;
        Ok(update.map(|update| update.version))
    }

    async fn install_update(&self) -> Result<(), String> {
        let updater = self.app.updater().map_err(|e| e.to_string())?;
        let update = updater
            .check()
            .await
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "No update available".to_string())?;
        update
            .download_and_install(|_, _| {}, || {})
            .await
            .map_err(|e| e.to_string())?;
        self.app.restart();
    }

    fn quit(&self) {
        self.app.exit(0);
    }
}

/// The desktop features over `app`, for the engine's server to hand its
/// commands.
pub fn desktop<R: Runtime>(app: &tauri::AppHandle<R>) -> Arc<dyn Desktop> {
    Arc::new(TauriDesktop { app: app.clone() })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::engine::dispatch::COMMANDS;

    const UI_COMMANDS: &str = include_str!("../../../ui/src/commands.rs");
    const UI_PANIC_REPORT: &str = include_str!("../../../ui/src/panic_report.rs");

    /// Each command the UI calls, with the fields of the arguments it sends.
    ///
    /// Reads the calls as the UI writes them: `invoke("name", &Args { a, b })`
    /// or `invoke_unit("name")`, `Args` serialized with camelCase names.
    fn ui_calls() -> BTreeMap<String, Vec<String>> {
        let call = regex::Regex::new(
            r#"invoke(?:_unit|_and_forget)?\(\s*"(\w+)"\s*(?:,\s*&\w+\s*\{([^}]*)\})?"#,
        )
        .unwrap();
        let mut calls = BTreeMap::new();
        for source in [UI_COMMANDS, UI_PANIC_REPORT] {
            for found in call.captures_iter(source) {
                let fields = found.get(2).map_or("", |m| m.as_str());
                let mut fields: Vec<String> = fields
                    .split(',')
                    .map(|field| field.split(':').next().unwrap_or_default().trim())
                    .filter(|field| !field.is_empty())
                    .map(str::to_owned)
                    .collect();
                fields.sort_unstable();
                calls.insert(found[1].to_owned(), fields);
            }
        }
        calls
    }

    /// The table and the UI agree on every command's name and arguments, so
    /// what the UI sends is what the command reads.
    #[test]
    fn every_command_the_ui_calls_is_listed_with_the_arguments_it_sends() {
        let listed: BTreeMap<String, Vec<String>> = COMMANDS
            .iter()
            .map(|(name, args)| {
                let mut args: Vec<String> = args.iter().map(|&arg| arg.to_owned()).collect();
                args.sort_unstable();
                ((*name).to_owned(), args)
            })
            .collect();
        let called = ui_calls();
        assert!(called.len() > 80, "found only {} UI calls", called.len());
        assert_eq!(called, listed);
    }
}
