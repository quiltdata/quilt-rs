//! One list of the commands, two ways in: the desktop window's Tauri IPC and
//! [`dispatch`] for a front end with no Tauri (a browser over a socket).
//!
//! `dispatch` takes the same JSON object Tauri's `invoke` receives (each
//! argument under its camelCase name) and answers with what Tauri would send
//! back: the command's value on success, its error string on failure.

use std::sync::Arc;

use quilt_rs::io::remote::WorkflowIntent;
use quilt_uri::S3PackageUri;
use serde::Deserialize;
use serde_json::Value;

use crate::commands::{
    AccountHost, AutosyncPatch, AutosyncSettingsData, CommitData, CommitViolation, CommitWorkflows,
    FolderListing, FreedSpace, InstalledPackageData, InstalledPackagesListData, LoginData,
    LoginErrorData, MainPageAccounts, MainPagePackageRefresh, MainPagePackages,
    MainPageRecentFiles, MainPageWatcher, MergeData, PackagePageData, RefreshedPackageStatus,
    RemotePackageResult, RevisionHistoryData, RolesData, SetRemoteResponse, SettingsData,
    SetupData, StopIgnoringPreview, StopIgnoringScope, StorageSize,
};
use crate::log_settings::LogLevel;
use crate::quilt;
use crate::services::Services;
use crate::toast::Toast;

/// What a desktop-only command answers when called without a window.
pub const NOT_IN_BROWSER: &str = "not available in the browser";

macro_rules! commands {
    (
        stateless { $( $sname:ident ( $( $sarg:ident : $sty:ty ),* ) -> $sret:ty ; )* }
        shared { $( $name:ident ( $( $arg:ident : $ty:ty ),* ) -> $ret:ty ; )* }
        desktop_plain { $( $dname:ident ( $( $darg:ident : $dty:ty ),* ) -> $dret:ty ; )* }
        desktop_native { $( $native:ident ),* $(,)? }
    ) => {
        /// The desktop window's commands: thin wrappers over the plain
        /// functions in [`crate::commands`].
        pub mod tauri_ipc {
            use super::*;
            $(
                #[tauri::command]
                pub async fn $sname($( $sarg: $sty ),*) -> Result<$sret, String> {
                    crate::commands::$sname($( $sarg ),*).await
                }
            )*
            $(
                #[tauri::command]
                pub async fn $name(
                    s: tauri::State<'_, Arc<Services>>,
                    $( $arg: $ty ),*
                ) -> Result<$ret, String> {
                    Box::pin(crate::commands::$name(&s, $( $arg ),*)).await
                }
            )*
            $(
                #[tauri::command]
                pub async fn $dname(
                    s: tauri::State<'_, Arc<Services>>,
                    $( $darg: $dty ),*
                ) -> Result<$dret, String> {
                    Box::pin(crate::commands::$dname(&s, $( $darg ),*)).await
                }
            )*
        }

        /// The Tauri invoke handler for every command.
        pub fn invoke_handler() -> impl Fn(tauri::ipc::Invoke) -> bool + Send + Sync + 'static {
            tauri::generate_handler![
                $( tauri_ipc::$sname, )*
                $( tauri_ipc::$name, )*
                $( tauri_ipc::$dname, )*
                $( crate::commands::$native, )*
            ]
        }

        /// Commands that work without a window.
        #[allow(dead_code, reason = "the tests use it")]
        pub const SHARED: &[&str] = &[$( stringify!($sname), )* $( stringify!($name) ),*];
        /// Commands that need the desktop window or the desktop app.
        pub const DESKTOP_ONLY: &[&str] = &[$( stringify!($dname), )* $( stringify!($native) ),*];

        /// A shared command with its arguments read.
        #[allow(non_camel_case_types, reason = "one variant per command, named as the command")]
        enum Call {
            $( $sname { $( $sarg: $sty ),* }, )*
            $( $name { $( $arg: $ty ),* }, )*
        }

        fn parse(cmd: &str, args: Value) -> Result<Call, Value> {
            match cmd {
                $(
                    stringify!($sname) => {
                        #[derive(Deserialize)]
                        #[serde(rename_all = "camelCase")]
                        struct Args { $( $sarg: $sty ),* }
                        let Args { $( $sarg ),* } = serde_json::from_value(args).map_err(|err| {
                            Value::String(format!("invalid args for command `{cmd}`: {err}"))
                        })?;
                        Ok(Call::$sname { $( $sarg ),* })
                    }
                )*
                $(
                    stringify!($name) => {
                        #[derive(Deserialize)]
                        #[serde(rename_all = "camelCase")]
                        struct Args { $( $arg: $ty ),* }
                        let Args { $( $arg ),* } = serde_json::from_value(args).map_err(|err| {
                            Value::String(format!("invalid args for command `{cmd}`: {err}"))
                        })?;
                        Ok(Call::$name { $( $arg ),* })
                    }
                )*
                _ => Err(Value::String(format!("command {cmd} not found"))),
            }
        }

        async fn run(s: &Services, call: Call) -> Result<Value, Value> {
            match call {
                $(
                    Call::$sname { $( $sarg ),* } => {
                        finish(crate::commands::$sname($( $sarg ),*).await)
                    }
                )*
                $(
                    Call::$name { $( $arg ),* } => {
                        finish(Box::pin(crate::commands::$name(s, $( $arg ),*)).await)
                    }
                )*
            }
        }
    };
}

commands! {
    stateless {
        get_setup_data() -> SetupData;
        test_quiltignore_pattern(pattern: String, path: String) -> bool;
        get_login_error_data(host: String, title: Option<String>, error: String) -> LoginErrorData;
        get_login_data(host: String, back: String) -> LoginData;
        list_folders(path: Option<String>) -> FolderListing;
    }
    shared {
        erase_auth(host: String) -> String;
        get_roles(host: String) -> RolesData;
        switch_role(host: String, role: String) -> RolesData;
        login(host: String, code: String) -> String;
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
        package_set_sync_scope(namespace: String, scope: crate::commands::KeepingScope, folders: Vec<String>) -> ();
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
        package_download_backlog(namespace: String, paths: Vec<String>) -> crate::commands::NotDownloaded;
        get_settings_data() -> SettingsData;
        set_default_ignore_rules(off: Vec<String>) -> ();
        open_user_ignore_rules() -> ();
        update_publish_settings(message_template: String, default_workflow: String, default_metadata: String) -> ();
        update_autosync_settings(settings: AutosyncSettingsData) -> ();
        set_autosync_direction(pull: Option<bool>, push: Option<bool>) -> ();
        patch_autosync_settings(patch: AutosyncPatch) -> ();
        get_autosync_snapshot() -> crate::autopull::reporter::WatcherSnapshot;
        get_autopull_activity() -> Option<crate::autopull::activity::AutopullActivity>;
        update_fswatcher_settings(enabled: bool) -> ();
        update_log_settings(level: LogLevel) -> ();
        update_experimental_settings(entire_package_sync: Option<bool>, agent_tasks: Option<bool>) -> ();
        update_task_settings(settings: crate::tasks::settings::TaskSettings) -> ();
        update_design_settings(design: crate::design_settings::Design) -> ();
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
        bench_report(text: String) -> ();
        reset_web_token() -> ();
    }
    desktop_plain {
        login_oauth(host: String, back: Option<String>) -> String;
    }
    desktop_native {
        copy_to_clipboard,
        open_directory_picker,
        check_for_update,
        download_and_install_update,
        quit_confirm,
    }
}

fn finish<T: serde::Serialize>(out: Result<T, String>) -> Result<Value, Value> {
    match out {
        Ok(value) => serde_json::to_value(value).map_err(|err| Value::String(err.to_string())),
        Err(err) => Err(Value::String(err)),
    }
}

/// What a desktop-only command answers without the desktop app, `None` for
/// any other.
fn desktop_answer(cmd: &str) -> Option<Result<Value, Value>> {
    if cmd == "check_for_update" {
        return None;
    }
    DESKTOP_ONLY
        .contains(&cmd)
        .then(|| Err(Value::String(NOT_IN_BROWSER.to_owned())))
}

fn read_args<T: serde::de::DeserializeOwned>(cmd: &str, args: Value) -> Result<T, Value> {
    serde_json::from_value(args)
        .map_err(|err| Value::String(format!("invalid args for command `{cmd}`: {err}")))
}

/// Run a desktop-only command with the desktop app's handle, as its Tauri
/// command would. `None` for any other command.
async fn run_desktop(
    app: &tauri::AppHandle,
    s: &Services,
    cmd: &str,
    args: Value,
) -> Option<Result<Value, Value>> {
    use crate::commands as system;
    Some(match cmd {
        "login_oauth" => {
            #[derive(Deserialize)]
            struct Args {
                host: String,
                back: Option<String>,
            }
            match read_args::<Args>(cmd, args) {
                Ok(a) => finish(crate::commands::login_oauth(s, a.host, a.back).await),
                Err(err) => Err(err),
            }
        }
        "copy_to_clipboard" => {
            #[derive(Deserialize)]
            struct Args {
                text: String,
                uri: Option<S3PackageUri>,
            }
            match read_args::<Args>(cmd, args) {
                Ok(a) => finish(system::copy_to_clipboard_with(
                    app,
                    s,
                    &a.text,
                    a.uri.as_ref(),
                )),
                Err(err) => Err(err),
            }
        }
        "open_directory_picker" => finish(system::open_directory_picker_with(app, s)),
        "check_for_update" => finish(system::check_for_update(app.clone()).await),
        "download_and_install_update" => {
            finish(system::download_and_install_update(app.clone()).await)
        }
        "quit_confirm" => {
            app.exit(0);
            Ok(Value::Null)
        }
        _ => return None,
    })
}

/// Run `cmd` with `args` (the object Tauri's `invoke` would receive).
///
/// # Errors
///
/// The command's error, serialized as Tauri rejects with it, or a message
/// for an unknown command, unreadable arguments or a desktop-only command
/// without the desktop app.
pub async fn dispatch(s: &Arc<Services>, cmd: &str, args: Value) -> Result<Value, Value> {
    // Tauri's `invoke` without arguments sends an empty object.
    let args = if args.is_null() {
        Value::Object(serde_json::Map::new())
    } else {
        args
    };
    if DESKTOP_ONLY.contains(&cmd) {
        if let Some(app) = s.desktop.get()
            && let Some(answer) = Box::pin(run_desktop(app, s, cmd, args)).await
        {
            return answer;
        }
        if cmd == "check_for_update" {
            return finish(crate::update_feed::check(&s.app.version).await);
        }
        return desktop_answer(cmd).expect("a desktop-only command");
    }
    let call = parse(cmd, args)?;
    run(s, call).await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every command the window can invoke reaches an arm here: a shared one
    /// reads its arguments (and rejects these), a desktop one answers without
    /// a window.
    #[test]
    fn every_command_has_an_arm() {
        assert_eq!(SHARED.len() + DESKTOP_ONLY.len(), 88);
        for name in SHARED {
            let Err(Value::String(err)) = parse(name, Value::String("not args".into())) else {
                panic!("{name}: arguments of the wrong shape were accepted");
            };
            assert!(err.starts_with("invalid args"), "{name}: {err}");
        }
        for name in DESKTOP_ONLY {
            // The update check reads the feed itself; see `dispatch`.
            if *name != "check_for_update" {
                assert!(desktop_answer(name).is_some(), "{name}");
            }
        }
        assert!(desktop_answer("nope").is_none());
        let Err(Value::String(err)) = parse("nope", Value::Null) else {
            panic!("an unknown command was accepted");
        };
        assert_eq!(err, "command nope not found");
    }

    #[test]
    fn arguments_are_read_by_their_camel_case_names() {
        let call = parse(
            "update_experimental_settings",
            serde_json::json!({"entirePackageSync": true}),
        );
        assert!(matches!(
            call,
            Ok(Call::update_experimental_settings {
                entire_package_sync: Some(true),
                ..
            })
        ));
        // A missing optional argument is `None`, as with Tauri.
        let call = parse("package_push", serde_json::json!({"namespace": "a/b"}));
        assert!(matches!(call, Ok(Call::package_push { uri: None, .. })));
    }
}
