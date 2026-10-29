//! The window's Tauri commands.
//!
//! Most are thin wrappers over the plain functions in
//! [`crate::engine::commands`], generated from one list. The rest need the
//! desktop itself: the folder picker, the clipboard, the updater and quitting.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use quilt_rs::io::remote::WorkflowIntent;
use quilt_uri::S3PackageUri;
use rfd::FileDialog;
use serde::Serialize;
use tauri::Manager;
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_updater::UpdaterExt;

use crate::engine::Error;
use crate::engine::commands::{
    AccountHost, AutosyncPatch, AutosyncSettingsData, CommitData, CommitViolation, CommitWorkflows,
    FreedSpace, InstalledPackageData, InstalledPackagesListData, KeepingScope, LoginData,
    LoginErrorData, MainPageAccounts, MainPagePackageRefresh, MainPagePackages,
    MainPageRecentFiles, MainPageWatcher, MergeData, NotDownloaded, PackagePageData,
    RefreshedPackageStatus, RemotePackageResult, RevisionHistoryData, RolesData, SetRemoteResponse,
    SettingsData, SetupData, StopIgnoringPreview, StopIgnoringScope, StorageSize,
};
use crate::engine::error::DesktopError;
use crate::engine::log_settings::LogLevel;
use crate::engine::notify::Notify;
use crate::engine::quilt;
use crate::engine::services::Services;
use crate::engine::telemetry::{MixpanelEvent, prelude::*};
use crate::engine::toast::Toast;

/// One `#[tauri::command]` per engine command, under the same name and with
/// the same arguments.
macro_rules! wrappers {
    (
        stateless { $( $sname:ident ( $( $sarg:ident : $sty:ty ),* ) -> $sret:ty ; )* }
        shared { $( $name:ident ( $( $arg:ident : $ty:ty ),* ) -> $ret:ty ; )* }
    ) => {
        $(
            #[tauri::command]
            pub async fn $sname($( $sarg: $sty ),*) -> Result<$sret, String> {
                crate::engine::commands::$sname($( $sarg ),*).await
            }
        )*
        $(
            #[tauri::command]
            pub async fn $name(
                s: tauri::State<'_, Arc<Services>>,
                $( $arg: $ty ),*
            ) -> Result<$ret, String> {
                Box::pin(crate::engine::commands::$name(&s, $( $arg ),*)).await
            }
        )*
    };
}

wrappers! {
    stateless {
        get_setup_data() -> SetupData;
        test_quiltignore_pattern(pattern: String, path: String) -> bool;
        get_login_error_data(host: String, title: Option<String>, error: String) -> LoginErrorData;
        get_login_data(host: String, back: String) -> LoginData;
    }
    shared {
        erase_auth(host: String) -> String;
        get_roles(host: String) -> RolesData;
        switch_role(host: String, role: String) -> RolesData;
        login(host: String, code: String) -> String;
        login_oauth(host: String, back: Option<String>) -> String;
        get_merge_data(namespace: String) -> MergeData;
        get_commit_data(namespace: String) -> CommitData;
        get_bucket_workflows(host: Option<String>, bucket: String) -> CommitWorkflows;
        load_workflow_rules(namespace: String, workflow_id: String, refresh: bool) -> bool;
        validate_commit_candidate(namespace: String, workflow_id: String, message: String, user_meta: String, name: String) -> Vec<CommitViolation>;
        get_main_page_packages() -> MainPagePackages;
        get_main_page_accounts() -> MainPageAccounts;
        refresh_main_page_account(host: String) -> AccountHost;
        refresh_main_page_package(namespace: String) -> MainPagePackageRefresh;
        get_main_page_watcher() -> MainPageWatcher;
        get_main_page_recent_files() -> MainPageRecentFiles;
        get_installed_package_data(namespace: String, filter: Option<String>) -> InstalledPackageData;
        get_installed_packages_list_data() -> InstalledPackagesListData;
        refresh_package_status(namespace: String) -> RefreshedPackageStatus;
        package_commit(namespace: String, message: String, metadata: String, workflow: WorkflowIntent, uri: Option<S3PackageUri>) -> String;
        certify_latest(namespace: String, uri: Option<S3PackageUri>) -> String;
        reset_local(namespace: String, uri: Option<S3PackageUri>) -> String;
        package_push(namespace: String, uri: Option<S3PackageUri>) -> String;
        package_publish(namespace: String, uri: Option<S3PackageUri>) -> String;
        package_commit_and_push(namespace: String, message: String, metadata: String, workflow: WorkflowIntent, uri: Option<S3PackageUri>) -> String;
        package_set_sync_scope(namespace: String, scope: KeepingScope, folders: Vec<String>) -> ();
        package_pull(namespace: String, uri: Option<S3PackageUri>) -> String;
        package_pull_outcome(namespace: String) -> quilt::flow::PullPreview;
        package_uninstall(namespace: String, uri: Option<S3PackageUri>, prune: bool) -> String;
        undo_commit(namespace: String) -> String;
        set_remote(namespace: String, origin: String, bucket: String, workflow: WorkflowIntent) -> SetRemoteResponse;
        change_catalog(namespace: String, origin: String) -> String;
        package_create(namespace: String, source: Option<String>, message: Option<String>) -> String;
        package_install_paths(uri: String, paths: Vec<String>) -> String;
        add_to_quiltignore(namespace: String, pattern: String, uri: Option<S3PackageUri>) -> String;
        stop_ignoring_preview(namespace: String, path: String, folder: bool) -> StopIgnoringPreview;
        stop_ignoring(namespace: String, path: String, folder: bool, scope: StopIgnoringScope, uri: Option<S3PackageUri>) -> String;
        handle_remote_package(uri: String) -> RemotePackageResult;
        get_revision_message(bucket: String, namespace: String, hash: String, catalog: Option<String>) -> Option<String>;
        get_package_page_data(namespace: String) -> PackagePageData;
        get_revision_history(namespace: String) -> RevisionHistoryData;
        remove_revisions(namespace: String, hashes: Vec<String>) -> String;
        package_download_backlog(namespace: String, paths: Vec<String>) -> NotDownloaded;
        get_settings_data() -> SettingsData;
        set_default_ignore_rules(off: Vec<String>) -> ();
        open_user_ignore_rules() -> ();
        update_publish_settings(message_template: String, default_workflow: String, default_metadata: String) -> ();
        update_autosync_settings(settings: AutosyncSettingsData) -> ();
        set_autosync_direction(pull: Option<bool>, push: Option<bool>) -> ();
        patch_autosync_settings(patch: AutosyncPatch) -> ();
        get_autosync_snapshot() -> crate::engine::autopull::reporter::WatcherSnapshot;
        get_autopull_activity() -> Option<crate::engine::autopull::activity::AutopullActivity>;
        update_fswatcher_settings(enabled: bool) -> ();
        update_log_settings(level: LogLevel) -> ();
        update_experimental_settings(entire_package_sync: Option<bool>, agent_tasks: Option<bool>) -> ();
        update_task_settings(settings: crate::engine::tasks::settings::TaskSettings) -> ();
        update_design_settings(design: crate::engine::design_settings::Design) -> ();
        debug_dot_quilt() -> String;
        debug_logs() -> String;
        open_home_dir() -> String;
        open_data_dir() -> String;
        run_gc() -> FreedSpace;
        measure_storage() -> StorageSize;
        collect_diagnostic_logs() -> String;
        send_crash_report(zip_path: String, message: Option<String>) -> String;
        reveal_in_file_browser(namespace: String, path: String, uri: Option<S3PackageUri>) -> String;
        open_in_file_browser(namespace: String, uri: Option<S3PackageUri>) -> String;
        open_in_default_application(namespace: String, path: String, uri: Option<S3PackageUri>) -> String;
        package_file_path(namespace: String, path: String) -> String;
        open_in_web_browser(url: String) -> String;
        setup(directory: String) -> String;
        get_toasts() -> Vec<Toast>;
        dismiss_toast(id: u64) -> ();
        report_ui_panic(message: String) -> ();
        quit_prompt_shown(generation: u64) -> ();
        quit_cancel() -> ();
    }
}

impl From<tauri::Error> for Error {
    fn from(e: tauri::Error) -> Self {
        Error::Desktop(DesktopError::Tauri(e.to_string()))
    }
}

fn open_directory_picker_command(app_handle: &tauri::AppHandle) -> Result<PathBuf, Error> {
    let home_dir = app_handle.path().home_dir()?;

    let canonical_home = home_dir.join(quilt::DEFAULT_HOME_DIR_NAME);
    let canonical_home_already_exists = canonical_home.exists();

    if !canonical_home_already_exists && let Err(e) = fs::create_dir_all(&canonical_home) {
        return Err(Error::from(e));
    }

    let window = app_handle
        .get_webview_window("main")
        .ok_or(DesktopError::Window)?;

    let result = if let Some(path) = FileDialog::new()
        .set_directory(&canonical_home)
        .set_parent(&window)
        .pick_folder()
    {
        debug!("Successfully selected {}", path.display());
        Ok(path)
    } else {
        debug!("User cancelled directory selection");
        Err(Error::Desktop(DesktopError::UserCancelled))
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
#[allow(
    clippy::unused_async,
    reason = "async so Tauri runs the dialog off the main thread, as it always has"
)]
pub async fn open_directory_picker(
    app_handle: tauri::AppHandle,
    s: tauri::State<'_, Arc<Services>>,
) -> Result<PathBuf, String> {
    // Deliberately reported before the outcome, and the only site that is. The
    // criterion is whether the event's name stays true when the call returns
    // `Err`, and here it does: this command models the user *cancelling* as an
    // error, so waiting for success would silently narrow "the picker was shown"
    // to "a folder was chosen". Those are different questions and this event
    // answers the first. If the second is ever wanted, it is a second event.
    s.telemetry.track(MixpanelEvent::DirectoryPickerOpened);

    match open_directory_picker_command(&app_handle) {
        Ok(path) => Ok(path),
        Err(err) => {
            error!("Failed to open directory picker: {}", err);
            Err(err.to_string())
        }
    }
}

fn copy_to_clipboard_command(app: &tauri::AppHandle, text: &str) -> Result<(), Error> {
    app.clipboard()
        .write_text(text)
        .map_err(|err| DesktopError::Clipboard(err.to_string()))?;
    Ok(())
}

/// Put `text` on the system clipboard.
///
/// The text, not the thing it describes: the caller decides what a copy *means*
/// — the `quilt+s3` address of a file or of a package — and this command decides
/// nothing about it. `uri` is for the event alone and never reaches the
/// clipboard, which is why the two are separate arguments rather than one that
/// gets rendered here: a `uri` with no path is a package's, and counts apart
/// from files.
///
/// Through the backend rather than `navigator.clipboard`, so it does not depend
/// on a webview setting that differs per platform: on Linux and Windows the DOM
/// API is gated behind wry's `clipboard` attribute, which Tauri leaves off and
/// exposes only on a builder this app does not call — its window comes from
/// `tauri.conf.json`.
#[tauri::command]
#[allow(
    clippy::unused_async,
    reason = "async so Tauri runs it off the main thread, as it always has"
)]
pub async fn copy_to_clipboard(
    app: tauri::AppHandle,
    s: tauri::State<'_, Arc<Services>>,
    text: String,
    uri: Option<S3PackageUri>,
) -> Result<String, String> {
    let msg_init = format!("Copying to clipboard: {text}");
    let msg_ok = "Successfully copied to clipboard".to_string();
    let msg_err = |err: &Error| format!("Failed to copy to clipboard: {err}");

    Notify::new(msg_init)
        .on_success(
            &s.telemetry,
            crate::engine::commands::copied_event(uri.as_ref()),
        )
        .map(copy_to_clipboard_command(&app, &text), msg_ok, msg_err)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub version: String,
}

#[tauri::command]
pub async fn check_for_update(app: tauri::AppHandle) -> Result<Option<UpdateInfo>, String> {
    // The UI ignores a failed check so users are not nagged on every launch,
    // which leaves the log as the only place the failure shows.
    let checked = match app.updater() {
        Ok(updater) => updater.check().await,
        Err(e) => Err(e),
    };
    match checked {
        Ok(Some(update)) => Ok(Some(UpdateInfo {
            version: update.version.clone(),
        })),
        Ok(None) => Ok(None),
        Err(e) => {
            warn!("update check failed: {e}");
            Err(e.to_string())
        }
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

/// *Quit anyway* — the user accepts interrupting the apply.
#[tauri::command]
#[allow(
    clippy::needless_pass_by_value,
    reason = "Tauri's command macro dictates the signature: the app handle is injected by value"
)]
pub fn quit_confirm(app: tauri::AppHandle) {
    app.exit(0);
}

/// The Tauri invoke handler for every command.
pub fn invoke_handler() -> impl Fn(tauri::ipc::Invoke) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        certify_latest,
        erase_auth,
        get_bucket_workflows,
        get_commit_data,
        get_installed_package_data,
        get_installed_packages_list_data,
        get_login_data,
        get_login_error_data,
        get_merge_data,
        get_roles,
        switch_role,
        load_workflow_rules,
        validate_commit_candidate,
        get_settings_data,
        get_setup_data,
        debug_dot_quilt,
        debug_logs,
        open_data_dir,
        open_home_dir,
        run_gc,
        measure_storage,
        collect_diagnostic_logs,
        send_crash_report,
        login,
        login_oauth,
        open_directory_picker,
        open_in_default_application,
        open_in_file_browser,
        copy_to_clipboard,
        package_file_path,
        open_in_web_browser,
        package_commit,
        package_commit_and_push,
        package_install_paths,
        package_set_sync_scope,
        package_publish,
        package_pull,
        package_pull_outcome,
        package_push,
        update_publish_settings,
        update_autosync_settings,
        set_autosync_direction,
        patch_autosync_settings,
        get_autosync_snapshot,
        get_autopull_activity,
        get_toasts,
        dismiss_toast,
        update_fswatcher_settings,
        set_default_ignore_rules,
        open_user_ignore_rules,
        update_log_settings,
        update_experimental_settings,
        update_task_settings,
        update_design_settings,
        get_main_page_packages,
        get_main_page_recent_files,
        get_main_page_accounts,
        refresh_main_page_account,
        refresh_main_page_package,
        get_main_page_watcher,
        get_package_page_data,
        get_revision_history,
        remove_revisions,
        package_download_backlog,
        refresh_package_status,
        package_uninstall,
        reset_local,
        undo_commit,
        reveal_in_file_browser,
        set_remote,
        change_catalog,
        package_create,
        setup,
        add_to_quiltignore,
        test_quiltignore_pattern,
        stop_ignoring_preview,
        stop_ignoring,
        handle_remote_package,
        get_revision_message,
        check_for_update,
        download_and_install_update,
        report_ui_panic,
        quit_prompt_shown,
        quit_confirm,
        quit_cancel,
    ]
}
