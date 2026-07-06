//! OS-integration commands: setup, directory pickers, file browser,
//! diagnostics, and auto-update.

use std::fs;
use std::path::PathBuf;

use rfd::FileDialog;
use serde::Serialize;
use tauri::Manager;
use tauri_plugin_updater::UpdaterExt;
use tokio::sync;

use crate::Error;
use crate::app;
use crate::model;
use crate::model::QuiltModel;
use crate::notify::Notify;
use crate::quilt;
use crate::quilt::paths::DomainPaths;
use crate::telemetry::diagnostics;
use crate::telemetry::{MixpanelEvent, prelude::*};

fn get_default_home_dir(app_handle: &tauri::AppHandle) -> Result<PathBuf, Error> {
    let path_resolver = app_handle.path();
    let user_home = path_resolver.home_dir()?;
    Ok(user_home.join("QuiltSync"))
}

// ── Setup data for Leptos UI ──

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupData {
    pub default_home: String,
}

#[tauri::command]
pub async fn get_setup_data(
    app_handle: tauri::State<'_, sync::Mutex<tauri::AppHandle>>,
) -> Result<SetupData, String> {
    let app_handle = app_handle.lock().await;
    let home = get_default_home_dir(&app_handle).map_err(|e| e.to_string())?;
    Ok(SetupData {
        default_home: home.display().to_string(),
    })
}

async fn open_directory_picker_command(app_handle: &tauri::AppHandle) -> Result<PathBuf, Error> {
    let paths = app_handle.path();
    let home_dir = paths.home_dir()?;

    let canonical_home = home_dir.join("QuiltSync");
    let canonical_home_already_exists = canonical_home.exists();

    if !canonical_home_already_exists && let Err(e) = fs::create_dir_all(&canonical_home) {
        return Err(Error::from(e));
    }

    let window = app_handle
        .get_webview_window("main")
        .ok_or(crate::error::TauriUiError::Window)?;

    let result = if let Some(path) = FileDialog::new()
        .set_directory(&canonical_home)
        .set_parent(&window)
        .pick_folder()
    {
        debug!("Successfully selected {}", path.display());
        Ok(path)
    } else {
        debug!("User cancelled directory selection");
        Err(Error::TauriUi(crate::error::TauriUiError::UserCancelled))
    };

    // Cleanup logic: remove temporary canonical directory if needed
    let should_delete_canonical = match &result {
        Ok(path) => path != &canonical_home,
        Err(_) => true,
    };

    if !canonical_home_already_exists
        && should_delete_canonical
        && let Err(e) = fs::remove_dir(&canonical_home)
    {
        error!("Failed to remove temporary QuiltSync directory: {}", e);
    }

    result
}

#[tauri::command]
pub async fn open_directory_picker(
    app_handle: tauri::State<'_, sync::Mutex<tauri::AppHandle>>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
) -> Result<PathBuf, String> {
    tracing.track(MixpanelEvent::DirectoryPickerOpened).await;

    let app_handle = &app_handle.lock().await;

    match open_directory_picker_command(app_handle).await {
        Ok(path) => Ok(path),
        Err(err) => {
            error!("Failed to open directory picker: {}", err);
            Err(err.to_string())
        }
    }
}

async fn debug_dot_quilt_command(app_handle: &tauri::AppHandle) -> Result<(), Error> {
    let local_data_dir = app_handle.path().app_local_data_dir()?;
    let dot_quilt_dir = DomainPaths::new(local_data_dir).dot_quilt_dir();

    opener::open_browser(&dot_quilt_dir)?;
    Ok(())
}

#[tauri::command]
pub async fn debug_dot_quilt(
    app_handle: tauri::State<'_, sync::Mutex<tauri::AppHandle>>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
) -> Result<String, String> {
    tracing.track(MixpanelEvent::DebugDotQuiltOpened).await;
    let app_handle = app_handle.lock().await;

    let msg_init = "Opening .quilt directory".to_string();
    let msg_ok = "Successfully opened .quilt directory".to_string();
    let msg_err = |err: &Error| format!("Failed to open directory: {err}");

    Notify::new(msg_init).map(debug_dot_quilt_command(&app_handle).await, msg_ok, msg_err)
}

async fn debug_logs_command(app: &app::App) -> Result<(), Error> {
    let logs_dir = &app.logs_dir;
    opener::open_browser(logs_dir.path())?;
    Ok(())
}

#[tauri::command]
pub async fn debug_logs(
    app: tauri::State<'_, app::App>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
) -> Result<String, String> {
    tracing.track(MixpanelEvent::DebugLogsOpened).await;
    let app: &app::App = &app;

    let msg_init = "Opening logs directory".to_string();
    let msg_ok = "Successfully opened logs directory".to_string();
    let msg_err = |err: &Error| format!("Failed to open logs directory: {err}");

    Notify::new(msg_init).map(debug_logs_command(app).await, msg_ok, msg_err)
}

async fn open_home_dir_command(m: &model::Model) -> Result<(), Error> {
    let home = m.get_quilt().lock().await.get_home().await?;
    let home_path: &std::path::PathBuf = home.as_ref();
    opener::open_browser(home_path)?;
    Ok(())
}

#[tauri::command]
pub async fn open_home_dir(
    m: tauri::State<'_, model::Model>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
) -> Result<String, String> {
    tracing.track(MixpanelEvent::FileBrowserOpened).await;

    let msg_init = "Opening home directory".to_string();
    let msg_ok = "Successfully opened home directory".to_string();
    let msg_err = |err: &Error| format!("Failed to open home directory: {err}");

    Notify::new(msg_init).map(open_home_dir_command(&m).await, msg_ok, msg_err)
}

async fn open_data_dir_command(app_handle: &tauri::AppHandle) -> Result<(), Error> {
    let local_data_dir = app_handle.path().app_local_data_dir()?;
    opener::open_browser(&local_data_dir)?;
    Ok(())
}

#[tauri::command]
pub async fn open_data_dir(
    app_handle: tauri::State<'_, sync::Mutex<tauri::AppHandle>>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
) -> Result<String, String> {
    tracing.track(MixpanelEvent::FileBrowserOpened).await;
    let app_handle = app_handle.lock().await;

    let msg_init = "Opening data directory".to_string();
    let msg_ok = "Successfully opened data directory".to_string();
    let msg_err = |err: &Error| format!("Failed to open data directory: {err}");

    Notify::new(msg_init).map(open_data_dir_command(&app_handle).await, msg_ok, msg_err)
}

async fn collect_diagnostic_logs_command(
    app_handle: &tauri::AppHandle,
    m: &model::Model,
    app: &app::App,
) -> Result<PathBuf, Error> {
    let info = diagnostics::collect(app_handle, m, app).await?;
    tokio::task::spawn_blocking(move || diagnostics::save_diagnostic_zip(&info))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn collect_diagnostic_logs(
    app_handle: tauri::State<'_, sync::Mutex<tauri::AppHandle>>,
    m: tauri::State<'_, model::Model>,
    app: tauri::State<'_, app::App>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
) -> Result<String, String> {
    tracing.track(MixpanelEvent::DiagnosticLogsSaved).await;
    let app_handle = app_handle.lock().await;
    let app: &app::App = &app;

    match collect_diagnostic_logs_command(&app_handle, &m, app).await {
        Ok(zip_path) => Ok(zip_path.display().to_string()),
        Err(err) => Err(err.to_string()),
    }
}

#[tauri::command]
pub async fn send_crash_report(
    zip_path: String,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
) -> Result<String, String> {
    tracing.track(MixpanelEvent::CrashReportSent).await;

    let zip_path = PathBuf::from(zip_path);
    if zip_path.file_name() != Some("quiltsync-diagnostic.zip".as_ref()) {
        return Err("Invalid diagnostic zip filename".to_string());
    }

    let msg_init = "Sending crash report".to_string();
    let msg_ok = "Successfully sent crash report".to_string();
    let msg_err = |err: &Error| format!("Failed to send crash report: {err}");

    let result =
        tokio::task::spawn_blocking(move || diagnostics::send_crash_report(zip_path.as_path()))
            .await
            .map_err(|e| Error::General(e.to_string()))
            .and_then(|r| r);

    Notify::new(msg_init).map(result, msg_ok, msg_err)
}

async fn reveal_in_file_browser_command(
    m: &model::Model,
    namespace: &str,
    path: &str,
) -> Result<(), Error> {
    let namespace = quilt_uri::Namespace::try_from(namespace)?;
    m.reveal_in_file_browser(&namespace, &PathBuf::from(path))
        .await?;
    Ok(())
}

#[tauri::command]
pub async fn reveal_in_file_browser(
    m: tauri::State<'_, model::Model>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
    namespace: String,
    path: String,
) -> Result<String, String> {
    tracing.track(MixpanelEvent::FileRevealed).await;

    let msg_init = format!("Revealing {path} in file browser for {namespace}");
    let msg_ok = format!("Successfully opened {path} in file browser");
    let msg_err = |err: &Error| format!("Failed to open directory: {err}");

    Notify::new(msg_init).map(
        reveal_in_file_browser_command(&m, &namespace, &path).await,
        msg_ok,
        msg_err,
    )
}

async fn open_in_file_browser_command(m: &model::Model, namespace: &str) -> Result<(), Error> {
    let namespace = quilt_uri::Namespace::try_from(namespace)?;
    m.open_in_file_browser(&namespace).await?;
    Ok(())
}

#[tauri::command]
pub async fn open_in_file_browser(
    m: tauri::State<'_, model::Model>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
    namespace: String,
) -> Result<String, String> {
    tracing.track(MixpanelEvent::FileBrowserOpened).await;

    let msg_init = format!("Opening file manager for {namespace}");
    let msg_ok = format!("Successfully opened file manager for {namespace}");
    let msg_err = |err: &Error| format!("Failed to open file manager: {err}");

    Notify::new(msg_init).map(
        open_in_file_browser_command(&m, &namespace).await,
        msg_ok,
        msg_err,
    )
}

async fn open_in_default_application_command(
    m: &model::Model,
    namespace: &str,
    path: &str,
) -> Result<(), Error> {
    let namespace = quilt_uri::Namespace::try_from(namespace)?;
    m.open_in_default_application(&namespace, &PathBuf::from(path))
        .await?;
    Ok(())
}

#[tauri::command]
pub async fn open_in_default_application(
    m: tauri::State<'_, model::Model>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
    namespace: String,
    path: String,
) -> Result<String, String> {
    tracing.track(MixpanelEvent::DefaultApplicationOpened).await;

    let msg_init = format!("Opening {path} with default application for {namespace}");
    let msg_ok = format!("Successfully opened {path} with default application");
    let msg_err = |err: &Error| format!("Failed to open application: {err}");

    Notify::new(msg_init).map(
        open_in_default_application_command(&m, &namespace, &path).await,
        msg_ok,
        msg_err,
    )
}

async fn open_in_web_browser_command(url: &str) -> Result<(), Error> {
    model::open_in_web_browser(url)?;
    Ok(())
}

#[tauri::command]
pub async fn open_in_web_browser(
    url: String,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
) -> Result<String, String> {
    tracing.track(MixpanelEvent::WebBrowserOpened).await;
    let msg_init = format!("Opening URL {url}");
    let msg_ok = format!("Successfully opened {url}");
    let msg_err = |err: &Error| format!("Failed to open URL: {err}");

    Notify::new(msg_init).map(open_in_web_browser_command(&url).await, msg_ok, msg_err)
}

async fn setup_command(m: &model::Model, directory: &str) -> Result<quilt::lineage::Home, Error> {
    if let Err(err) = fs::create_dir_all(directory)
        && err.kind() != std::io::ErrorKind::AlreadyExists
    {
        return Err(Error::from(err));
    }

    m.set_home(&directory).await
}

#[tauri::command]
pub async fn setup(
    m: tauri::State<'_, model::Model>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
    directory: String,
) -> Result<String, String> {
    tracing.track(MixpanelEvent::SetupCompleted).await;
    let msg_init = format!("Setup with directory {directory}");
    let msg_ok = format!("Successfully set up directory: {directory}");
    let msg_err = |err: &Error| format!("Failed to create QuiltSync directory: {err}");
    Notify::new(msg_init).map(setup_command(&m, &directory).await, msg_ok, msg_err)
}

// ── Auto-update ────────────────────────────────────────────

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub version: String,
}

#[tauri::command]
pub async fn check_for_update(app: tauri::AppHandle) -> Result<Option<UpdateInfo>, String> {
    let updater = app.updater().map_err(|e| e.to_string())?;
    match updater.check().await {
        Ok(Some(update)) => Ok(Some(UpdateInfo {
            version: update.version.clone(),
        })),
        Ok(None) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

#[tauri::command]
pub async fn download_and_install_update(app: tauri::AppHandle) -> Result<(), String> {
    let updater = app.updater().map_err(|e| e.to_string())?;
    let update = updater
        .check()
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "No update available".to_string())?;
    update
        .download_and_install(|_, _| {}, || {})
        .await
        .map_err(|e| e.to_string())?;
    app.restart();
}
