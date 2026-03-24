use std::io::Write;
use std::path::PathBuf;

use tauri::Manager;

use crate::app::App;
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
    let local_data_dir = app_handle.path().app_local_data_dir()?;
    let auth_hosts = quilt::paths::list_auth_hosts(&local_data_dir);

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
        version: app.version.to_string(),
        os: format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
        data_dir: local_data_dir,
        home_dir,
        logs_dir: app.logs_dir.path().to_path_buf(),
        auth_hosts,
    })
}

/// Send a Sentry crash report with diagnostic metadata.
pub fn send_crash_report(info: DiagnosticInfo) {
    sentry::configure_scope(|scope| {
        scope.set_extra("app_version", info.version.clone().into());
        scope.set_extra("os", info.os.clone().into());
        scope.set_extra("data_dir", info.data_dir.display().to_string().into());
        scope.set_extra("home_dir", info.home_dir.clone().into());
        scope.set_extra(
            "authenticated_hosts",
            serde_json::Value::from(info.auth_hosts),
        );
    });

    sentry::capture_message("User crash report", sentry::Level::Error);
}

/// Bundle diagnostic info, logs, and config files into a zip and reveal it.
pub fn save_diagnostic_zip(info: DiagnosticInfo) -> Result<PathBuf, Error> {
    let auth_dir = info.data_dir.join(quilt::paths::AUTH_DIR);

    let zip_path = info.data_dir.join("quiltsync-diagnostic.zip");
    let file = std::fs::File::create(&zip_path)?;
    let mut zip_writer = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    // Write metadata
    let metadata = serde_json::json!({
        "version": info.version,
        "os": info.os,
        "data_dir": info.data_dir.display().to_string(),
        "home_dir": info.home_dir,
        "authenticated_hosts": info.auth_hosts,
    });
    zip_writer.start_file("metadata.json", options)?;
    zip_writer.write_all(serde_json::to_string_pretty(&metadata)?.as_bytes())?;

    // Add log files
    if info.logs_dir.exists() {
        if let Ok(entries) = std::fs::read_dir(&info.logs_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    let name = format!("logs/{}", entry.file_name().to_string_lossy());
                    zip_writer.start_file(name, options)?;
                    let contents = std::fs::read(&path)?;
                    zip_writer.write_all(&contents)?;
                }
            }
        }
    }

    // Add data.json (lineage file)
    let data_json = info.data_dir.join(".quilt").join("data.json");
    if data_json.exists() {
        zip_writer.start_file("data.json", options)?;
        let contents = std::fs::read(&data_json)?;
        zip_writer.write_all(&contents)?;
    }

    // Add client.json per host (OAuth client registration — client ID only)
    for host in &info.auth_hosts {
        let client_json = auth_dir.join(host).join(quilt::paths::AUTH_CLIENT);
        if client_json.exists() {
            let name = format!("auth/{}/client.json", host);
            zip_writer.start_file(name, options)?;
            let contents = std::fs::read(&client_json)?;
            zip_writer.write_all(&contents)?;
        }
    }

    zip_writer.finish()?;

    Ok(zip_path)
}
