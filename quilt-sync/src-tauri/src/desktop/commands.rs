//! The window's one Tauri command, [`engine_call`], and what the desktop
//! gives the engine's commands: the folder picker, the clipboard, the updater
//! and quitting.

use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use rfd::FileDialog;
use serde_json::Value;
use tauri::Manager;
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_updater::UpdaterExt;

use crate::engine::Error;
use crate::engine::desktop::Desktop;
use crate::engine::dispatch::dispatch;
use crate::engine::error::DesktopError;
use crate::engine::services::Services;

impl From<tauri::Error> for Error {
    fn from(e: tauri::Error) -> Self {
        Error::Desktop(DesktopError::Tauri(e.to_string()))
    }
}

/// The desktop features, over the Tauri app.
struct TauriDesktop {
    app: tauri::AppHandle,
}

#[async_trait]
impl Desktop for TauriDesktop {
    async fn pick_folder(&self, start: &Path) -> crate::engine::Result<Option<PathBuf>> {
        let window = self
            .app
            .get_webview_window("main")
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

/// Run the engine command `cmd` with `args`, as the UI names them.
#[tauri::command]
pub async fn engine_call(
    app: tauri::AppHandle,
    s: tauri::State<'_, Arc<Services>>,
    cmd: String,
    args: Value,
) -> Result<Value, String> {
    let desktop = TauriDesktop { app };
    dispatch(&s, Some(&desktop), &cmd, args).await
}

/// The Tauri invoke handler: every command goes through [`engine_call`].
pub fn invoke_handler() -> impl Fn(tauri::ipc::Invoke) -> bool + Send + Sync + 'static {
    tauri::generate_handler![engine_call]
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
