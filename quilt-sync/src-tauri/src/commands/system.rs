//! OS-integration commands: setup, directory pickers, file browser,
//! diagnostics, local storage cleanup, and auto-update.

use std::fs;
use std::path::PathBuf;

use quilt_uri::S3PackageUri;
use rfd::FileDialog;
use serde::Serialize;
use tauri::Manager;
use tauri_plugin_clipboard_manager::ClipboardExt;
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
use crate::telemetry::event::PackageFileEvent;
use crate::telemetry::{MixpanelEvent, prelude::*};

fn get_default_home_dir(app_handle: &tauri::AppHandle) -> Result<PathBuf, Error> {
    let path_resolver = app_handle.path();
    let user_home = path_resolver.home_dir()?;
    Ok(user_home.join(quilt::DEFAULT_HOME_DIR_NAME))
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

fn open_directory_picker_command(app_handle: &tauri::AppHandle) -> Result<PathBuf, Error> {
    let paths = app_handle.path();
    let home_dir = paths.home_dir()?;

    let canonical_home = home_dir.join(quilt::DEFAULT_HOME_DIR_NAME);
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
    // Deliberately reported before the outcome, and the only site that is. The
    // criterion is whether the event's name stays true when the call returns
    // `Err`, and here it does: this command models the user *cancelling* as an
    // error, so waiting for success would silently narrow "the picker was shown"
    // to "a folder was chosen". Those are different questions and this event
    // answers the first. If the second is ever wanted, it is a second event.
    tracing.track(MixpanelEvent::DirectoryPickerOpened);

    let app_handle = &app_handle.lock().await;

    match open_directory_picker_command(app_handle) {
        Ok(path) => Ok(path),
        Err(err) => {
            error!("Failed to open directory picker: {}", err);
            Err(err.to_string())
        }
    }
}

fn debug_dot_quilt_command(app_handle: &tauri::AppHandle) -> Result<(), Error> {
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
    let app_handle = app_handle.lock().await;

    let msg_init = "Opening .quilt directory".to_string();
    let msg_ok = "Successfully opened .quilt directory".to_string();
    let msg_err = |err: &Error| format!("Failed to open directory: {err}");

    Notify::new(msg_init)
        .on_success(&tracing, MixpanelEvent::DebugDotQuiltOpened)
        .map(debug_dot_quilt_command(&app_handle), msg_ok, msg_err)
}

fn debug_logs_command(app: &app::App) -> Result<(), Error> {
    let logs_dir = &app.logging.dir;
    opener::open_browser(logs_dir.path())?;
    Ok(())
}

#[tauri::command]
pub async fn debug_logs(
    app: tauri::State<'_, app::App>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
) -> Result<String, String> {
    let app: &app::App = &app;

    let msg_init = "Opening logs directory".to_string();
    let msg_ok = "Successfully opened logs directory".to_string();
    let msg_err = |err: &Error| format!("Failed to open logs directory: {err}");

    Notify::new(msg_init)
        .on_success(&tracing, MixpanelEvent::DebugLogsOpened)
        .map(debug_logs_command(app), msg_ok, msg_err)
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
    let msg_init = "Opening home directory".to_string();
    let msg_ok = "Successfully opened home directory".to_string();
    let msg_err = |err: &Error| format!("Failed to open home directory: {err}");

    Notify::new(msg_init)
        .on_success(&tracing, MixpanelEvent::HomeDirOpened)
        .map(open_home_dir_command(&m).await, msg_ok, msg_err)
}

fn open_data_dir_command(app_handle: &tauri::AppHandle) -> Result<(), Error> {
    let local_data_dir = app_handle.path().app_local_data_dir()?;
    opener::open_browser(&local_data_dir)?;
    Ok(())
}

#[tauri::command]
pub async fn open_data_dir(
    app_handle: tauri::State<'_, sync::Mutex<tauri::AppHandle>>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
) -> Result<String, String> {
    let app_handle = app_handle.lock().await;

    let msg_init = "Opening data directory".to_string();
    let msg_ok = "Successfully opened data directory".to_string();
    let msg_err = |err: &Error| format!("Failed to open data directory: {err}");

    Notify::new(msg_init)
        .on_success(&tracing, MixpanelEvent::DataDirOpened)
        .map(open_data_dir_command(&app_handle), msg_ok, msg_err)
}

async fn run_gc_command(m: &impl QuiltModel) -> Result<String, String> {
    // Made before the sweep so its line logs first, as everywhere else; the
    // success message is the report's own sentence, known only once it ends.
    let notify = Notify::new("Freeing up space".to_string());
    let result = m.gc().await;
    let msg_ok = result.as_ref().map(ToString::to_string).unwrap_or_default();
    let msg_err = |err: &Error| format!("Failed to free up space: {}", err.user_facing());

    notify.map(result, msg_ok, msg_err)
}

/// Delete what the domain holds for nothing, answering with what was freed.
#[tauri::command]
pub async fn run_gc(m: tauri::State<'_, model::Model>) -> Result<String, String> {
    run_gc_command(&*m).await
}

/// How much local storage holds, and how much Free up space would free.
#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageSize {
    pub total_bytes: u64,
    /// An estimate: Free up space's own report is the exact answer.
    pub freeable_bytes: u64,
}

async fn measure_storage_command(m: &impl QuiltModel) -> Result<StorageSize, String> {
    debug!("Measuring local storage");
    let size = m
        .measure_storage()
        .await
        .map_err(|err| format!("Failed to measure local storage: {}", err.user_facing()))?;
    Ok(StorageSize {
        total_bytes: size.total_bytes,
        freeable_bytes: size.freeable.bytes,
    })
}

/// Measure local storage without deleting anything or taking package locks;
/// run only when the user asks.
#[tauri::command]
pub async fn measure_storage(m: tauri::State<'_, model::Model>) -> Result<StorageSize, String> {
    measure_storage_command(&*m).await
}

async fn collect_diagnostic_logs_command(
    app_handle: &tauri::AppHandle,
    m: &model::Model,
    app: &app::App,
    tracing: &crate::telemetry::Telemetry,
) -> Result<PathBuf, Error> {
    let info = diagnostics::collect(app_handle, m, app, tracing.install_id()).await?;
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
    let app_handle = app_handle.lock().await;
    let app: &app::App = &app;

    match collect_diagnostic_logs_command(&app_handle, &m, app, &tracing).await {
        Ok(zip_path) => {
            // Reported here rather than through `Notify::on_success`, because this
            // command returns the archive's path and so does not route its outcome
            // through `map`. Same rule, hand-applied: nothing was saved unless this
            // arm was reached.
            tracing.track(MixpanelEvent::DiagnosticLogsSaved);
            Ok(zip_path.display().to_string())
        }
        Err(err) => Err(err.to_string()),
    }
}

#[tauri::command]
pub async fn send_crash_report(
    zip_path: String,
    message: Option<String>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
) -> Result<String, String> {
    let zip_path = PathBuf::from(zip_path);
    if zip_path.file_name() != Some("quiltsync-diagnostic.zip".as_ref()) {
        return Err("Invalid diagnostic zip filename".to_string());
    }

    let msg_init = "Sending crash report".to_string();
    let msg_ok = "Successfully sent crash report".to_string();
    let msg_err = |err: &Error| format!("Failed to send crash report: {err}");

    let result = tokio::task::spawn_blocking(move || {
        diagnostics::send_crash_report(zip_path.as_path(), message.as_deref())
    })
    .await
    .map_err(|e| Error::General(e.to_string()))
    .and_then(|r| r);

    Notify::new(msg_init)
        .on_success(&tracing, MixpanelEvent::CrashReportSent)
        .map(result, msg_ok, msg_err)
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
    uri: Option<S3PackageUri>,
) -> Result<String, String> {
    let msg_init = format!("Revealing {path} in file browser for {namespace}");
    let msg_ok = format!("Successfully opened {path} in file browser");
    let msg_err = |err: &Error| format!("Failed to open directory: {err}");

    Notify::new(msg_init)
        .on_success(
            &tracing,
            MixpanelEvent::FileRevealed(PackageFileEvent::for_uri(uri.as_ref())),
        )
        .map(
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
    uri: Option<S3PackageUri>,
) -> Result<String, String> {
    let msg_init = format!("Opening file manager for {namespace}");
    let msg_ok = format!("Successfully opened file manager for {namespace}");
    let msg_err = |err: &Error| format!("Failed to open file manager: {err}");

    Notify::new(msg_init)
        .on_success(
            &tracing,
            MixpanelEvent::PackageDirOpened(PackageFileEvent::for_uri(uri.as_ref())),
        )
        .map(
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
    uri: Option<S3PackageUri>,
) -> Result<String, String> {
    let msg_init = format!("Opening {path} with default application for {namespace}");
    let msg_ok = format!("Successfully opened {path} with default application");
    let msg_err = |err: &Error| format!("Failed to open application: {err}");

    Notify::new(msg_init)
        .on_success(
            &tracing,
            MixpanelEvent::DefaultApplicationOpened(PackageFileEvent::for_uri(uri.as_ref())),
        )
        .map(
            open_in_default_application_command(&m, &namespace, &path).await,
            msg_ok,
            msg_err,
        )
}

/// Where a package's file is on disk: the package's home joined with its
/// path inside the package, as opening the file resolves it, so the two cannot
/// disagree. Fails, as opening does, for a file that is not there.
async fn package_file_path_command(
    m: &impl QuiltModel,
    namespace: &str,
    path: &str,
) -> Result<PathBuf, Error> {
    let namespace = quilt_uri::Namespace::try_from(namespace)?;
    m.file_path(&namespace, &PathBuf::from(path)).await
}

/// The absolute path of one of a package's files, for the file pane's *Copy
/// path*: a `New` file has no `quilt+s3` address that resolves, so the reader
/// gets where it is on this computer instead. Reads nothing but the package's
/// home, and reports nothing: the copy that follows is the event.
#[tauri::command]
pub async fn package_file_path(
    m: tauri::State<'_, model::Model>,
    namespace: String,
    path: String,
) -> Result<String, String> {
    package_file_path_command(&*m, &namespace, &path)
        .await
        .map(|p| p.display().to_string())
        .map_err(|err| err.to_string())
}

fn open_in_web_browser_command(url: &str) -> Result<(), Error> {
    model::open_in_web_browser(url)?;
    Ok(())
}

#[tauri::command]
pub async fn open_in_web_browser(
    url: String,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
) -> Result<String, String> {
    // Hostless on purpose, even though a URL has a host: this one command opens
    // catalog links, documentation, a local directory and `mailto:` alike, so its
    // URL's host is not necessarily a Quilt deployment — and `host` means the
    // deployment an event concerns, never a hostname that appeared nearby.
    let msg_init = format!("Opening URL {url}");
    let msg_ok = format!("Successfully opened {url}");
    let msg_err = |err: &Error| format!("Failed to open URL: {err}");

    Notify::new(msg_init)
        .on_success(&tracing, MixpanelEvent::WebBrowserOpened)
        .map(open_in_web_browser_command(&url), msg_ok, msg_err)
}

fn copy_to_clipboard_command(app: &tauri::AppHandle, text: &str) -> Result<(), Error> {
    app.clipboard()
        .write_text(text)
        .map_err(crate::error::TauriUiError::from)?;
    Ok(())
}

/// Put `text` on the system clipboard.
///
/// The text, not the thing it describes: the caller decides what a copy *means*
/// — today the `quilt+s3` address of a file — and this command decides nothing
/// about it. `uri` is for the event alone and never reaches the clipboard, which
/// is why the two are separate arguments rather than one that gets rendered here.
///
/// Through the backend rather than `navigator.clipboard`, so it does not depend
/// on a webview setting that differs per platform: on Linux and Windows the DOM
/// API is gated behind wry's `clipboard` attribute, which Tauri leaves off and
/// exposes only on a builder this app does not call — its window comes from
/// `tauri.conf.json`.
#[tauri::command]
pub async fn copy_to_clipboard(
    app: tauri::AppHandle,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
    text: String,
    uri: Option<S3PackageUri>,
) -> Result<String, String> {
    let msg_init = format!("Copying to clipboard: {text}");
    let msg_ok = "Successfully copied to clipboard".to_string();
    let msg_err = |err: &Error| format!("Failed to copy to clipboard: {err}");

    Notify::new(msg_init)
        .on_success(
            &tracing,
            MixpanelEvent::FileUriCopied(PackageFileEvent::for_uri(uri.as_ref())),
        )
        .map(copy_to_clipboard_command(&app, &text), msg_ok, msg_err)
}

/// Stores `directory` as the home. quilt-rs creates the folder.
async fn setup_command(m: &model::Model, directory: &str) -> Result<quilt::lineage::Home, Error> {
    m.set_home(&directory).await
}

#[tauri::command]
pub async fn setup(
    m: tauri::State<'_, model::Model>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
    directory: String,
) -> Result<String, String> {
    let msg_init = format!("Setup with directory {directory}");
    let msg_ok = format!("Successfully set up directory: {directory}");
    let msg_err = |err: &Error| format!("Failed to create QuiltSync directory: {err}");
    Notify::new(msg_init)
        .on_success(&tracing, MixpanelEvent::SetupCompleted)
        .map(setup_command(&m, &directory).await, msg_ok, msg_err)
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

/// How much of a panic message reaches the crash reporter.
///
/// A payload is whatever was formatted into `panic!` — plausibly a `Debug` dump of
/// a large value — and it becomes an issue's title, so it is bounded here rather
/// than at the sink. Generous enough that the message and its location survive.
const MAX_PANIC_MESSAGE: usize = 1024;

/// Bound the message at a character boundary, marking that it was cut.
fn bounded(message: &str) -> String {
    if message.chars().count() <= MAX_PANIC_MESSAGE {
        return message.to_owned();
    }
    // By characters, not bytes: slicing a multi-byte character in half would panic
    // while reporting a panic.
    let kept: String = message.chars().take(MAX_PANIC_MESSAGE).collect();
    format!("{kept}… (truncated)")
}

/// Report a panic the frontend caught in its own hook.
///
/// The frontend is a WASM module in a webview with no crash client of its own, so
/// its panic hook could reach the browser console and nothing else. Bridging it
/// here is the only way a UI panic reaches anybody.
///
/// Both sinks on purpose, at two levels. `warn!` puts it in the log file and leaves
/// a breadcrumb; the explicit report is what becomes an issue. Logging it at `error!`
/// instead would file a *second* issue through the crash-sink layer, since that is
/// what an error-level event does — one panic, two reports.
#[tauri::command]
#[allow(
    clippy::needless_pass_by_value,
    reason = "Tauri's command macro dictates the signature: an argument is deserialized \
              into an owned value and state is injected by value. Sync rather than async \
              because there is nothing to await, and an async command taking state would \
              have to return a Result it could never populate."
)]
pub fn report_ui_panic(message: String, tracing: tauri::State<'_, crate::telemetry::Telemetry>) {
    let message = bounded(&message);
    warn!("UI panic: {message}");
    tracing.report_ui_panic(&message);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The first-run pick creates the folder it picks and stores it as the
    /// home, so the first install has somewhere to go.
    #[tokio::test]
    async fn setup_creates_the_picked_folder_and_stores_it() {
        let domain = tempfile::tempdir().expect("temp domain");
        let parent = tempfile::tempdir().expect("temp parent");
        let picked = parent.path().join("not/yet/there");
        let m = model::Model::create(domain.path());

        let home = setup_command(&m, picked.to_str().expect("utf-8 path"))
            .await
            .expect("setup succeeds");

        assert_eq!(home.as_ref(), &picked);
        assert!(picked.is_dir());
    }

    /// *Copy path* asks where the file is, and gets the path opening the file
    /// would open: the package's home joined with its path inside it.
    #[tokio::test]
    async fn a_package_file_s_path_is_where_opening_it_would_look() {
        let mut m = model::MockQuiltModel::new();
        m.expect_file_path()
            .withf(|ns, path| ns.to_string() == "team/plate" && path == &PathBuf::from("raw/a.csv"))
            .returning(|_, path| Ok(PathBuf::from("/Users/me/QuiltSync/team/plate").join(path)));

        let found = package_file_path_command(&m, "team/plate", "raw/a.csv")
            .await
            .expect("a file that is there has a path");

        assert_eq!(
            found,
            PathBuf::from("/Users/me/QuiltSync/team/plate/raw/a.csv")
        );
    }

    /// A file that is not there has no path to copy, and says why, as opening
    /// it would.
    #[tokio::test]
    async fn a_missing_file_has_no_path_to_copy() {
        let mut m = model::MockQuiltModel::new();
        m.expect_file_path().returning(|_, path| {
            Err(Error::FsOpen(crate::error::FsOpenError::PathNotFound(
                path.clone(),
            )))
        });

        assert!(
            package_file_path_command(&m, "team/plate", "raw/gone.csv")
                .await
                .is_err()
        );
    }

    /// The toast says what the sweep freed, in the report's own words.
    #[tokio::test]
    async fn freeing_up_space_answers_with_what_it_freed() {
        let mut m = model::MockQuiltModel::new();
        m.expect_gc().returning(|| {
            Ok(quilt::flow::GcReport {
                objects: 6,
                cached_manifests: 2,
                staging: 0,
                bytes: 630_200,
            })
        });

        assert_eq!(
            run_gc_command(&m).await,
            Ok("Freed 630.2 kB: 6 objects, 2 cached manifests".to_string())
        );
    }

    /// A package busy elsewhere stops the sweep, and the message names it.
    #[tokio::test]
    async fn freeing_up_space_while_a_package_is_busy_says_which() {
        let mut m = model::MockQuiltModel::new();
        m.expect_gc()
            .returning(|| Err(quilt::Error::PackageBusy(("acme", "demo").into()).into()));

        assert_eq!(
            run_gc_command(&m).await,
            Err(
                "Failed to free up space: acme/demo is busy in another quilt process; \
                 try again once it finishes"
                    .to_string()
            )
        );
    }

    /// Measuring answers with the total and the estimate's bytes.
    #[tokio::test]
    async fn measuring_storage_answers_with_the_total_and_what_can_be_freed() {
        let mut m = model::MockQuiltModel::new();
        m.expect_measure_storage().returning(|| {
            Ok(quilt::flow::StorageSize {
                total_bytes: 3_200_000,
                freeable: quilt::flow::GcReport {
                    objects: 6,
                    cached_manifests: 2,
                    staging: 0,
                    bytes: 1_100_000,
                },
            })
        });

        assert_eq!(
            measure_storage_command(&m).await,
            Ok(StorageSize {
                total_bytes: 3_200_000,
                freeable_bytes: 1_100_000,
            })
        );
    }

    /// A failed walk says what failed, in the user's words.
    #[tokio::test]
    async fn measuring_storage_that_fails_says_so() {
        let mut m = model::MockQuiltModel::new();
        m.expect_measure_storage()
            .returning(|| Err(quilt::Error::Io(std::io::Error::other("disk gone")).into()));

        let Err(message) = measure_storage_command(&m).await else {
            panic!("a failed walk is an error");
        };
        assert!(
            message.starts_with("Failed to measure local storage: "),
            "{message}"
        );
    }

    /// A short message is passed through untouched — the common case, and the one a
    /// reader of the issue title needs to be exact.
    #[test]
    fn a_short_panic_message_is_untouched() {
        let message = "panicked at ui/src/pages.rs:12:5:\nassertion failed";
        assert_eq!(bounded(message), message);
    }

    /// A long one is cut and *says* it was cut, so nobody reads a truncated payload
    /// as the whole story.
    #[test]
    fn a_long_panic_message_is_cut_and_marked() {
        let bounded = bounded(&"x".repeat(MAX_PANIC_MESSAGE * 3));

        assert!(bounded.ends_with("… (truncated)"));
        assert_eq!(
            bounded.chars().count(),
            MAX_PANIC_MESSAGE + "… (truncated)".chars().count()
        );
    }

    /// Cutting by characters rather than bytes, because slicing a multi-byte
    /// character in half would panic *while reporting a panic* — the one failure
    /// this path must not have.
    #[test]
    fn cutting_a_multibyte_message_does_not_panic() {
        // Four bytes per character, so a byte-indexed cut would land mid-character.
        let message = "🙀".repeat(MAX_PANIC_MESSAGE * 2);

        let bounded = bounded(&message);

        assert!(bounded.starts_with('🙀'));
        assert!(bounded.ends_with("… (truncated)"));
    }
}
