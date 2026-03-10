use std::fs;
use std::path::PathBuf;
use std::str::FromStr;

use rfd::FileDialog;
use tauri::Manager;
use tokio::sync;

use crate::app;
use crate::model;
use crate::oauth::OAuthState;
use crate::pages;
use crate::quilt;
use crate::routes;
use crate::Error;

use crate::model::QuiltModel;
use crate::telemetry::{prelude::*, MixpanelEvent};
use crate::ui::notify::TmplNotify;

fn get_default_home_dir(app_handle: &tauri::AppHandle) -> Result<PathBuf, Error> {
    let path_resolver = app_handle.path();
    let user_home = path_resolver.home_dir()?;
    Ok(user_home.join("QuiltSync"))
}

async fn load_page_command(
    m: &model::Model,
    app: &app::App,
    app_handle: &tauri::AppHandle,
    tracing: &crate::telemetry::Telemetry,
    location: &str,
) -> Result<String, Error> {
    let home = get_default_home_dir(app_handle)?;

    let path = location.parse::<routes::Paths>()?;
    let page_result = pages::load(m, app, &home, tracing, &path).await;

    match page_result {
        Ok(output) => {
            debug!("Page loaded successfully, URL: {}", location);
            tracing
                .track(MixpanelEvent::PageLoaded {
                    pathname: path.pathname(),
                    error: None,
                })
                .await;
            Ok(output)
        }
        Err(Error::Quilt(quilt::Error::LineageMissing | quilt::Error::LineageMissingHome)) => {
            let err = "Lineage file is required";
            error!("{}", err);
            let setup_page = pages::ViewSetup::create(app, &home).await?;
            tracing
                .track(MixpanelEvent::PageLoaded {
                    pathname: path.pathname(),
                    error: Some(err.to_string()),
                })
                .await;
            Ok(setup_page.render()?)
        }
        Err(Error::Quilt(quilt::Error::LoginRequired(Some(host)))) => {
            let warn = "Login is required";
            warn!("{}", warn);
            let login_page =
                pages::ViewLogin::create(app, tracing, host.clone(), Some(location.to_string()))
                    .await?;
            tracing
                .track(MixpanelEvent::PageLoaded {
                    pathname: path.pathname(),
                    error: Some(warn.to_string()),
                })
                .await;
            Ok(login_page.render()?)
        }
        Err(err) => {
            error!("{}", err);
            let error = Some(err.to_string());
            let error_page = pages::ViewError::create(app, err).await?;
            tracing
                .track(MixpanelEvent::PageLoaded {
                    pathname: path.pathname(),
                    error,
                })
                .await;
            Ok(error_page.render()?)
        }
    }
}

#[tauri::command]
pub async fn load_page(
    m: tauri::State<'_, model::Model>,
    app: tauri::State<'_, app::App>,
    app_handle: tauri::State<'_, sync::Mutex<tauri::AppHandle>>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
    location: String,
) -> Result<String, String> {
    let m: &model::Model = &m;
    let app: &app::App = &app;
    let tracing: &crate::telemetry::Telemetry = &tracing;

    let app_handle = &app_handle.lock().await;

    match load_page_command(m, app, app_handle, tracing, &location).await {
        Ok(result) => Ok(result),
        Err(err) => {
            error!("Failed to load page: {}", err);
            match pages::ViewError::create(app, err).await {
                Ok(error_page) => match error_page.render() {
                    Ok(rendered) => Ok(rendered),
                    Err(render_err) => Ok(format!("Critical error: {}", render_err)),
                },
                Err(create_err) => Ok(format!("Critical error: {}", create_err)),
            }
        }
    }
}

async fn package_commit_command(
    m: &model::Model,
    namespace: &str,
    message: &str,
    metadata: &str,
    workflow: Option<String>,
) -> Result<(), Error> {
    let namespace = quilt::uri::Namespace::try_from(namespace)?;
    if message.is_empty() {
        return Err(Error::Commit("Message is required".to_string()));
    }

    model::package_commit(m, namespace.clone(), message, metadata, workflow, None).await?;
    Ok(())
}

#[tauri::command]
pub async fn package_commit(
    m: tauri::State<'_, model::Model>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
    namespace: String,
    message: String,
    metadata: String,
    workflow: Option<String>,
) -> Result<String, String> {
    let m: &model::Model = &m;

    tracing.track(MixpanelEvent::PackageCommitted).await;

    let msg_init = format!("Committing package {namespace}");
    let msg_ok = format!("Successfully committed {namespace}");
    let msg_err = |err: &Error| format!("Failed to commit: {err}");

    TmplNotify::new(msg_init).map(
        package_commit_command(m, &namespace, &message, &metadata, workflow).await,
        msg_ok,
        msg_err,
    )
}

async fn open_directory_picker_command(app_handle: &tauri::AppHandle) -> Result<PathBuf, Error> {
    let paths = app_handle.path();
    let home_dir = paths.home_dir()?;

    let canonical_home = home_dir.join("QuiltSync");
    let canonical_home_already_exists = canonical_home.exists();

    if !canonical_home_already_exists {
        if let Err(e) = fs::create_dir_all(&canonical_home) {
            return Err(Error::from(e));
        }
    }

    let window = app_handle.get_webview_window("main").ok_or(Error::Window)?;

    let result = match FileDialog::new()
        .set_directory(&canonical_home)
        .set_parent(&window)
        .pick_folder()
    {
        Some(path) => {
            debug!("Successfully selected {}", path.display());
            Ok(path)
        }
        None => {
            debug!("User cancelled directory selection");
            Err(Error::UserCancelled)
        }
    };

    // Cleanup logic: remove temporary canonical directory if needed
    let should_delete_canonical = match &result {
        Ok(path) => path != &canonical_home,
        Err(_) => true,
    };

    if !canonical_home_already_exists && should_delete_canonical {
        if let Err(e) = fs::remove_dir(&canonical_home) {
            error!("Failed to remove temporary QuiltSync directory: {}", e);
        }
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

async fn erase_auth_command(app_handle: &tauri::AppHandle) -> Result<(), Error> {
    let local_data_dir = app_handle.path().app_local_data_dir()?;
    let auth_dir = local_data_dir.join(quilt::paths::AUTH_DIR);

    if auth_dir.exists() {
        std::fs::remove_dir_all(&auth_dir)?;
    }
    Ok(())
}

#[tauri::command]
pub async fn erase_auth(
    app_handle: tauri::State<'_, sync::Mutex<tauri::AppHandle>>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
) -> Result<String, String> {
    tracing.track(MixpanelEvent::AuthErased).await;

    let app_handle = app_handle.lock().await;

    let msg_init = "Erasing auth directory".to_string();
    let msg_ok = "Successfully erased auth directory".to_string();
    let msg_err = |err: &Error| format!("Failed to erase auth directory: {err}");

    TmplNotify::new(msg_init).map(erase_auth_command(&app_handle).await, msg_ok, msg_err)
}

async fn debug_dot_quilt_command(app_handle: &tauri::AppHandle) -> Result<(), Error> {
    let local_data_dir = app_handle.path().app_local_data_dir()?;
    let dot_quilt_dir = local_data_dir.join(".quilt");

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

    TmplNotify::new(msg_init).map(debug_dot_quilt_command(&app_handle).await, msg_ok, msg_err)
}

async fn debug_logs_command(app: &app::App) -> Result<(), Error> {
    let logs_dir = app.logs_dir();
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

    TmplNotify::new(msg_init).map(debug_logs_command(app).await, msg_ok, msg_err)
}

async fn reveal_in_file_browser_command(
    m: &model::Model,
    namespace: &str,
    path: &str,
) -> Result<(), Error> {
    let namespace = quilt::uri::Namespace::try_from(namespace)?;
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
    let m: &model::Model = &m;

    let msg_init = format!("Revealing {path} in file browser for {namespace}");
    let msg_ok = format!("Successfully opened {path} in file browser");
    let msg_err = |err: &Error| format!("Failed to open directory: {err}");

    TmplNotify::new(msg_init).map(
        reveal_in_file_browser_command(m, &namespace, &path).await,
        msg_ok,
        msg_err,
    )
}

#[tauri::command]
pub async fn load_empty() -> Result<String, String> {
    Ok("".to_string())
}

async fn open_in_file_browser_command(m: &model::Model, namespace: &str) -> Result<(), Error> {
    let namespace = quilt::uri::Namespace::try_from(namespace)?;
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
    let m: &model::Model = &m;

    let msg_init = format!("Opening file manager for {namespace}");
    let msg_ok = format!("Successfully opened file manager for {namespace}");
    let msg_err = |err: &Error| format!("Failed to open file manager: {err}");

    TmplNotify::new(msg_init).map(
        open_in_file_browser_command(m, &namespace).await,
        msg_ok,
        msg_err,
    )
}

async fn open_in_default_application_command(
    m: &model::Model,
    namespace: &str,
    path: &str,
) -> Result<(), Error> {
    let namespace = quilt::uri::Namespace::try_from(namespace)?;
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
    let m: &model::Model = &m;

    let msg_init = format!("Opening {path} with default application for {namespace}");
    let msg_ok = format!("Successfully opened {path} with default application");
    let msg_err = |err: &Error| format!("Failed to open application: {err}");

    TmplNotify::new(msg_init).map(
        open_in_default_application_command(m, &namespace, &path).await,
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

    TmplNotify::new(msg_init).map(open_in_web_browser_command(&url).await, msg_ok, msg_err)
}

async fn certify_latest_command(m: &model::Model, namespace: &str) -> Result<(), Error> {
    let namespace = quilt::uri::Namespace::try_from(namespace)?;
    model::package_revision_certify_latest(m, namespace.clone()).await?;
    Ok(())
}

#[tauri::command]
pub async fn certify_latest(
    m: tauri::State<'_, model::Model>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
    namespace: String,
) -> Result<String, String> {
    tracing.track(MixpanelEvent::LatestCertified).await;
    let m: &model::Model = &m;

    let msg_init = format!("Certifying latest for {namespace}");
    let msg_ok = format!("Successfully certified latest for {namespace}");
    let msg_err = |err: &Error| format!("Failed to certify latest: {err}");

    TmplNotify::new(msg_init).map(certify_latest_command(m, &namespace).await, msg_ok, msg_err)
}

async fn reset_local_command(m: &model::Model, namespace: &str) -> Result<(), Error> {
    let namespace = quilt::uri::Namespace::try_from(namespace)?;
    model::package_revision_reset_local(m, namespace.clone()).await?;
    Ok(())
}

#[tauri::command]
pub async fn reset_local(
    m: tauri::State<'_, model::Model>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
    namespace: String,
) -> Result<String, String> {
    tracing.track(MixpanelEvent::LocalReset).await;
    let m: &model::Model = &m;

    let msg_init = format!("Resetting local for {namespace}");
    let msg_ok = format!("Successfully reset local for {namespace}");
    let msg_err = |err: &Error| format!("Failed to reset local: {err}");

    TmplNotify::new(msg_init).map(reset_local_command(m, &namespace).await, msg_ok, msg_err)
}

async fn package_push_command(m: &model::Model, namespace: &str) -> Result<(), Error> {
    let namespace = quilt::uri::Namespace::try_from(namespace)?;
    model::package_push(m, &namespace, None).await?;
    Ok(())
}

#[tauri::command]
pub async fn package_push(
    m: tauri::State<'_, model::Model>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
    namespace: String,
) -> Result<String, String> {
    tracing.track(MixpanelEvent::PackagePushed).await;
    let m: &model::Model = &m;

    let msg_init = format!("Pushing package {namespace}");
    let msg_ok = format!("Successfully pushed package {namespace}");
    let msg_err = |err: &Error| format!("Failed to push package: {err}");

    TmplNotify::new(msg_init).map(package_push_command(m, &namespace).await, msg_ok, msg_err)
}

async fn package_pull_command(m: &model::Model, namespace: &str) -> Result<(), Error> {
    let namespace = quilt::uri::Namespace::try_from(namespace)?;
    model::package_pull(m, &namespace, None).await?;
    Ok(())
}

#[tauri::command]
pub async fn package_pull(
    m: tauri::State<'_, model::Model>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
    namespace: String,
) -> Result<String, String> {
    tracing.track(MixpanelEvent::PackagePulled).await;
    let m: &model::Model = &m;

    let msg_init = format!("Pulling package {namespace}");
    let msg_ok = format!("Successfully pulled package {namespace}");
    let msg_err = |err: &Error| format!("Failed to pull package: {err}");

    TmplNotify::new(msg_init).map(package_pull_command(m, &namespace).await, msg_ok, msg_err)
}

async fn package_uninstall_command(m: &model::Model, namespace: &str) -> Result<(), Error> {
    let namespace = quilt::uri::Namespace::try_from(namespace)?;
    model::package_uninstall(m, namespace.clone()).await?;
    Ok(())
}

#[tauri::command]
pub async fn package_uninstall(
    m: tauri::State<'_, model::Model>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
    namespace: String,
) -> Result<String, String> {
    tracing.track(MixpanelEvent::PackageUninstalled).await;
    let m: &model::Model = &m;

    let msg_init = format!("Uninstalling package {namespace}");
    let msg_ok = format!("Successfully uninstalled package {namespace}");
    let msg_err = |err: &Error| format!("Failed to uninstall package: {err}");

    TmplNotify::new(msg_init).map(
        package_uninstall_command(m, &namespace).await,
        msg_ok,
        msg_err,
    )
}

async fn set_origin_command(m: &model::Model, namespace: &str, origin: &str) -> Result<(), Error> {
    let namespace = quilt::uri::Namespace::try_from(namespace)?;
    let origin = quilt::uri::Host::from_str(origin)?;
    model::set_origin(m, &namespace, origin).await?;
    Ok(())
}

#[tauri::command]
pub async fn set_origin(
    m: tauri::State<'_, model::Model>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
    namespace: String,
    origin: String,
) -> Result<String, String> {
    tracing.track(MixpanelEvent::OriginSet).await;
    let m: &model::Model = &m;

    let msg_init = format!("Setting origin for {namespace}");
    let msg_ok = format!("Successfully set origin for {namespace}");
    let msg_err = |err: &Error| format!("Failed to set origin: {err}");

    TmplNotify::new(msg_init).map(
        set_origin_command(m, &namespace, &origin).await,
        msg_ok,
        msg_err,
    )
}

/// Navigate to a location after successful login.
pub(crate) fn navigate_after_login(
    app_handle: &tauri::AppHandle,
    location: &str,
) -> Result<(), Error> {
    debug!("Attempting to redirect after login to: {}", location);
    match app_handle.get_webview_window("main") {
        Some(win) => match location.parse::<routes::Paths>() {
            Ok(page_path) => match win.url() {
                Ok(win_url) => {
                    let redirect_url = routes::from_url(page_path, win_url);
                    debug!("Redirecting to: {}", redirect_url);
                    if let Err(e) = win.navigate(redirect_url) {
                        error!("Failed to navigate after login: {}", e);
                        return Err(e.into());
                    }
                    Ok(())
                }
                Err(e) => {
                    error!("Failed to get window URL for redirect: {}", e);
                    Err(e.into())
                }
            },
            Err(e) => {
                error!(
                    "Failed to parse location '{}' for redirect: {}",
                    location, e
                );
                Err(e)
            }
        },
        None => {
            error!("Main window not found for post-login redirect");
            Ok(())
        }
    }
}

pub(crate) async fn login_command(
    m: &model::Model,
    host: &str,
    code: String,
    location: Option<String>,
    app_handle: &tauri::AppHandle,
) -> Result<(), Error> {
    let host = quilt::uri::Host::from_str(host)?;
    model::login(m, &host, code).await?;

    if let Some(location) = location {
        navigate_after_login(app_handle, &location)?;
    }

    Ok(())
}

#[tauri::command]
pub async fn login(
    m: tauri::State<'_, model::Model>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
    host: String,
    code: String,
    location: Option<String>,
    app_handle: tauri::State<'_, sync::Mutex<tauri::AppHandle>>,
) -> Result<String, String> {
    tracing
        .track(MixpanelEvent::UserLoggedIn { host: host.clone() })
        .await;
    let msg_init = format!("Login with code for host {host}");
    let msg_ok = format!("Successfully logged in to {host}");
    let msg_err = |err: &Error| format!("Failed to login: {err}");

    let app_handle = app_handle.lock().await;
    TmplNotify::new(msg_init).map(
        login_command(&m, &host, code, location, &app_handle).await,
        msg_ok,
        msg_err,
    )
}

/// Initiate OAuth 2.1 login: register client via DCR if needed,
/// generate PKCE, store verifier, open browser.
#[tauri::command]
pub async fn login_oauth(
    m: tauri::State<'_, model::Model>,
    oauth_state: tauri::State<'_, OAuthState>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
    host: String,
) -> Result<String, String> {
    let host_parsed = quilt::uri::Host::from_str(&host).map_err(|e| e.to_string())?;

    tracing
        .track(MixpanelEvent::UserLoggedIn { host: host.clone() })
        .await;

    let redirect_uri = crate::oauth::redirect_uri(&host_parsed);
    let client_id = model::get_or_register_client(&*m, &host_parsed, &redirect_uri)
        .await
        .map_err(|e| e.to_string())?;

    let request = oauth_state.start_login(&host_parsed, &client_id).await;

    model::open_in_web_browser(&request.authorize_url).map_err(|e| e.to_string())?;

    Ok(format!("Opening browser for OAuth login to {host}"))
}

async fn setup_command(m: &model::Model, directory: &str) -> Result<quilt::lineage::Home, Error> {
    if let Err(err) = fs::create_dir_all(directory) {
        if err.kind() != std::io::ErrorKind::AlreadyExists {
            return Err(Error::from(err));
        }
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
    TmplNotify::new(msg_init).map(setup_command(&m, &directory).await, msg_ok, msg_err)
}

async fn package_install_paths_command(
    m: &model::Model,
    uri: &str,
    paths: &[String],
) -> Result<(), Error> {
    let uri = quilt::uri::S3PackageUri::try_from(uri)?;
    let paths: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
    model::install_paths_only(m, &uri.namespace, paths).await?;
    Ok(())
}

#[tauri::command]
pub async fn package_install_paths(
    m: tauri::State<'_, model::Model>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
    uri: String,
    paths: Vec<String>,
) -> Result<String, String> {
    tracing.track(MixpanelEvent::PackageInstalled).await;
    let m: &model::Model = &m;

    let msg_init = format!("Installing paths from {uri}");
    let msg_ok = format!("Successfully installed {} paths", paths.len());
    let msg_err = |err: &Error| format!("Failed to install paths: {err}");

    TmplNotify::new(msg_init).map(
        package_install_paths_command(m, &uri, &paths).await,
        msg_ok,
        msg_err,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_load_empty() -> Result<(), String> {
        let result = load_empty().await?;
        assert_eq!(result, "");
        Ok(())
    }
}
