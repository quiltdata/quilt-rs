use std::io::Write;
use std::path::Path;
use std::path::PathBuf;

use sentry::protocol::Attachment;
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

/// Typed shape of `metadata.json` stored inside a diagnostic zip.
#[derive(serde::Serialize, serde::Deserialize, Debug, PartialEq, Eq)]
struct DiagnosticMetadata {
    version: String,
    os: String,
    data_dir: String,
    home_dir: String,
    authenticated_hosts: Vec<String>,
}

impl DiagnosticMetadata {
    fn from_info(info: &DiagnosticInfo) -> Self {
        Self {
            version: info.version.clone(),
            os: info.os.clone(),
            data_dir: info.data_dir.display().to_string(),
            home_dir: info.home_dir.clone(),
            authenticated_hosts: info.auth_hosts.clone(),
        }
    }
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

/// Send a Sentry crash report with the diagnostic zip attached.
///
/// Reads the pre-built zip file at `zip_path`, attaches it to the Sentry
/// event, and sets metadata extras from `metadata.json` inside the zip.
///
/// Returns an error if the Sentry client is not initialized (e.g. DSN not
/// configured or offline), so callers can inform the user instead of
/// silently pretending the report was sent.
pub fn send_crash_report(zip_path: &Path) -> Result<(), Error> {
    if sentry::Hub::current().client().is_none() {
        return Err(Error::General(
            "Sentry is not initialized — crash report was not sent".to_string(),
        ));
    }

    let zip_bytes = std::fs::read(zip_path)?;
    let metadata = read_metadata_from_zip(&zip_bytes);

    sentry::with_scope(
        |scope| {
            if let Some(ref m) = metadata {
                scope.set_extra("app_version", m.version.clone().into());
                scope.set_extra("os", m.os.clone().into());
                scope.set_extra("data_dir", m.data_dir.clone().into());
                scope.set_extra("home_dir", m.home_dir.clone().into());
                if let Ok(hosts) = serde_json::to_value(&m.authenticated_hosts) {
                    scope.set_extra("authenticated_hosts", hosts);
                }
            }

            scope.add_attachment(Attachment {
                buffer: zip_bytes,
                filename: "quiltsync-diagnostic.zip".to_string(),
                content_type: Some("application/zip".to_string()),
                ty: None,
            });
        },
        || {
            sentry::capture_message("User crash report", sentry::Level::Error);
        },
    );

    Ok(())
}

/// Try to read `metadata.json` from inside a zip archive.
fn read_metadata_from_zip(zip_bytes: &[u8]) -> Option<DiagnosticMetadata> {
    let cursor = std::io::Cursor::new(zip_bytes);
    let mut archive = zip::ZipArchive::new(cursor).ok()?;
    let file = archive.by_name("metadata.json").ok()?;
    serde_json::from_reader(file).ok()
}

/// Bundle diagnostic info, logs, and config files into a zip and reveal it.
pub fn save_diagnostic_zip(info: &DiagnosticInfo) -> Result<PathBuf, Error> {
    let auth_dir = info.data_dir.join(quilt::paths::AUTH_DIR);

    let zip_path = info.data_dir.join("quiltsync-diagnostic.zip");
    let file = std::fs::File::create(&zip_path)?;
    let mut zip_writer = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    // Write metadata
    let metadata = DiagnosticMetadata::from_info(info);
    zip_writer.start_file("metadata.json", options)?;
    zip_writer.write_all(serde_json::to_string_pretty(&metadata)?.as_bytes())?;

    // Add log files
    if info.logs_dir.exists()
        && let Ok(entries) = std::fs::read_dir(&info.logs_dir)
    {
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
            let name = format!("auth/{host}/client.json");
            zip_writer.start_file(name, options)?;
            let contents = std::fs::read(&client_json)?;
            zip_writer.write_all(&contents)?;
        }
    }

    zip_writer.finish()?;

    Ok(zip_path)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::io::Read;

    use tempfile::TempDir;

    use super::*;

    fn write_file(path: &Path, contents: &[u8]) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create parent dir");
        }
        std::fs::write(path, contents).expect("write file");
    }

    /// Drain every entry in the archive into a name → bytes map for byte-exact
    /// assertions independent of the on-disk entry order.
    fn read_zip_entries(zip_path: &Path) -> BTreeMap<String, Vec<u8>> {
        let bytes = std::fs::read(zip_path).expect("read zip file");
        let mut archive =
            zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("open zip archive");
        let mut entries = BTreeMap::new();
        for i in 0..archive.len() {
            let mut file = archive.by_index(i).expect("entry by index");
            let name = file.name().to_string();
            let mut buf = Vec::new();
            file.read_to_end(&mut buf).expect("read entry");
            entries.insert(name, buf);
        }
        entries
    }

    fn make_info(data_dir: PathBuf, logs_dir: PathBuf, auth_hosts: Vec<String>) -> DiagnosticInfo {
        DiagnosticInfo {
            version: "0.17.1-test".to_string(),
            os: "linux x86_64".to_string(),
            data_dir,
            home_dir: "/home/tester".to_string(),
            logs_dir,
            auth_hosts,
        }
    }

    #[test]
    fn save_diagnostic_zip_round_trip() {
        let data_tmp = TempDir::new().expect("data tempdir");
        let logs_tmp = TempDir::new().expect("logs tempdir");

        let current_log = b"2026-05-07T12:00:00Z INFO quiltsync starting up\n";
        let rotated_log = b"2026-05-06T23:59:59Z WARN quiltsync rotated log line\n";
        write_file(&logs_tmp.path().join("quiltsync.log"), current_log);
        write_file(&logs_tmp.path().join("quiltsync.log.1"), rotated_log);

        let data_json = br#"{"version":1,"packages":[]}"#;
        write_file(&data_tmp.path().join(".quilt").join("data.json"), data_json);

        let demo_host = "demo.quiltdata.com";
        let open_host = "open.quiltdata.com";
        let demo_client = br#"{"client_id":"abc"}"#;
        let open_client = br#"{"client_id":"def"}"#;
        write_file(
            &data_tmp
                .path()
                .join(quilt::paths::AUTH_DIR)
                .join(demo_host)
                .join(quilt::paths::AUTH_CLIENT),
            demo_client,
        );
        write_file(
            &data_tmp
                .path()
                .join(quilt::paths::AUTH_DIR)
                .join(open_host)
                .join(quilt::paths::AUTH_CLIENT),
            open_client,
        );

        let info = make_info(
            data_tmp.path().to_path_buf(),
            logs_tmp.path().to_path_buf(),
            vec![demo_host.to_string(), open_host.to_string()],
        );

        let zip_path = save_diagnostic_zip(&info).expect("save zip");
        assert_eq!(zip_path, data_tmp.path().join("quiltsync-diagnostic.zip"));
        assert!(zip_path.exists(), "zip file should exist on disk");

        let entries = read_zip_entries(&zip_path);

        let demo_entry = format!("auth/{demo_host}/client.json");
        let open_entry = format!("auth/{open_host}/client.json");
        let expected_names: Vec<&str> = vec![
            "metadata.json",
            "logs/quiltsync.log",
            "logs/quiltsync.log.1",
            "data.json",
            &demo_entry,
            &open_entry,
        ];
        let actual_names: Vec<&str> = entries.keys().map(String::as_str).collect();
        for name in &expected_names {
            assert!(
                actual_names.contains(name),
                "missing entry {name}; got {actual_names:?}",
            );
        }
        assert_eq!(
            entries.len(),
            expected_names.len(),
            "unexpected entry count: {actual_names:?}",
        );

        assert_eq!(entries["logs/quiltsync.log"], current_log);
        assert_eq!(entries["logs/quiltsync.log.1"], rotated_log);
        assert_eq!(entries["data.json"], data_json);
        assert_eq!(entries[&demo_entry], demo_client);
        assert_eq!(entries[&open_entry], open_client);

        let parsed: DiagnosticMetadata =
            serde_json::from_slice(&entries["metadata.json"]).expect("parse metadata.json");
        assert_eq!(parsed, DiagnosticMetadata::from_info(&info));

        let bytes = std::fs::read(&zip_path).expect("re-read zip bytes");
        assert_eq!(
            read_metadata_from_zip(&bytes),
            Some(DiagnosticMetadata::from_info(&info)),
        );
    }

    #[test]
    fn save_diagnostic_zip_minimal_inputs() {
        let data_tmp = TempDir::new().expect("data tempdir");
        let logs_tmp = TempDir::new().expect("logs tempdir");

        let info = make_info(
            data_tmp.path().to_path_buf(),
            logs_tmp.path().to_path_buf(),
            Vec::new(),
        );

        let zip_path = save_diagnostic_zip(&info).expect("save zip");
        let entries = read_zip_entries(&zip_path);

        let names: Vec<&str> = entries.keys().map(String::as_str).collect();
        assert_eq!(names, vec!["metadata.json"]);

        let parsed: DiagnosticMetadata =
            serde_json::from_slice(&entries["metadata.json"]).expect("parse metadata.json");
        assert!(parsed.authenticated_hosts.is_empty());
        assert_eq!(parsed, DiagnosticMetadata::from_info(&info));
    }

    #[test]
    fn read_metadata_from_zip_returns_none_on_bad_input() {
        // 1. Random bytes — not a valid zip archive.
        let garbage = b"this is definitely not a zip file";
        assert!(read_metadata_from_zip(garbage).is_none());

        // 2. Valid zip with no metadata.json entry.
        let mut buf = Vec::new();
        {
            let mut writer = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            writer.start_file("other.txt", options).expect("start_file");
            writer.write_all(b"not metadata").expect("write_all");
            writer.finish().expect("finish");
        }
        assert!(read_metadata_from_zip(&buf).is_none());

        // 3. Valid zip whose metadata.json is missing a required field.
        let mut buf = Vec::new();
        {
            let mut writer = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            writer
                .start_file("metadata.json", options)
                .expect("start_file");
            // Missing `authenticated_hosts`.
            writer
                .write_all(br#"{"version":"x","os":"y","data_dir":"d","home_dir":"h"}"#)
                .expect("write_all");
            writer.finish().expect("finish");
        }
        assert!(read_metadata_from_zip(&buf).is_none());
    }
}
