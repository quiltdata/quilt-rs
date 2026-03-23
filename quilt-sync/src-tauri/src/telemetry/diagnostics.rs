use std::path::PathBuf;

use tauri::Manager;

use crate::app::App;
use crate::app::AppAssets;
use crate::error::Error;
use crate::model::Model;
use crate::model::QuiltModel;
use crate::quilt;

/// Collected diagnostic metadata shared by crash reports and diagnostic log exports.
pub struct DiagnosticInfo {
    pub version: String,
    pub os: String,
    pub data_dir: PathBuf,
    pub home_dir: String,
    pub logs_dir: PathBuf,
    pub auth_hosts: Vec<String>,
}

/// Gather diagnostic info from the app state, model, and filesystem.
pub async fn collect(
    app_handle: &tauri::AppHandle,
    m: &Model,
    app: &App,
) -> Result<DiagnosticInfo, Error> {
    let globals = app.globals();
    let local_data_dir = app_handle.path().app_local_data_dir()?;
    let auth_dir = local_data_dir.join(quilt::paths::AUTH_DIR);

    let mut auth_hosts: Vec<String> = Vec::new();
    if auth_dir.exists() {
        if let Ok(entries) = std::fs::read_dir(&auth_dir) {
            for entry in entries.flatten() {
                if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    if let Some(name) = entry.file_name().to_str() {
                        auth_hosts.push(name.to_string());
                    }
                }
            }
        }
    }

    let home_dir = m
        .get_quilt()
        .lock()
        .await
        .get_home()
        .await
        .ok()
        .map(|h| h.as_ref().display().to_string())
        .unwrap_or_default();

    Ok(DiagnosticInfo {
        version: globals.version.to_string(),
        os: format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
        data_dir: local_data_dir,
        home_dir,
        logs_dir: globals.logs_dir,
        auth_hosts,
    })
}
