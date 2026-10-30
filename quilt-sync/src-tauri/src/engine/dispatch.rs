//! Every command, listed once, and [`dispatch`], which runs one by name.
//!
//! `dispatch` takes the arguments as the window sends them, one JSON object
//! with each argument under its camelCase name, and answers with the
//! command's value as JSON, or its error.

use std::path::PathBuf;

use quilt_rs::io::remote::WorkflowIntent;
use quilt_uri::S3PackageUri;
use serde::Deserialize;
use serde_json::Value;

use crate::engine::autopull::activity::AutopullActivity;
use crate::engine::autopull::reporter::WatcherSnapshot;
use crate::engine::commands::{
    AccountHost, AutosyncPatch, AutosyncSettingsData, CommitData, CommitViolation, CommitWorkflows,
    FreedSpace, InstalledPackageData, InstalledPackagesListData, KeepingScope, LoginData,
    LoginErrorData, MainPageAccounts, MainPagePackageRefresh, MainPagePackages,
    MainPageRecentFiles, MainPageWatcher, MergeData, NotDownloaded, PackagePageData,
    RefreshedPackageStatus, RemotePackageResult, RevisionHistoryData, RolesData, SetRemoteResponse,
    SettingsData, SetupData, StopIgnoringPreview, StopIgnoringScope, StorageSize, UpdateInfo,
};
use crate::engine::design_settings::Design;
use crate::engine::desktop::{Desktop, NOT_AVAILABLE};
use crate::engine::log_settings::LogLevel;
use crate::engine::quilt::flow::PullPreview;
use crate::engine::services::Services;
use crate::engine::tasks::settings::TaskSettings;
use crate::engine::toast::Toast;

/// Builds [`dispatch`] from the table below. Each group calls its functions in
/// [`crate::engine::commands`] differently:
///
/// - `stateless`: with the arguments only;
/// - `shared`: with the services first;
/// - `desktop`: with the services and the [`Desktop`], and answers
///   [`NOT_AVAILABLE`] without one.
macro_rules! commands {
    (
        stateless { $( $sname:ident ( $( $sarg:ident : $sty:ty ),* ) -> $sret:ty ; )* }
        shared { $( $name:ident ( $( $arg:ident : $ty:ty ),* ) -> $ret:ty ; )* }
        desktop { $( $dname:ident ( $( $darg:ident : $dty:ty ),* ) -> $dret:ty ; )* }
    ) => {
        /// Each command's name and its arguments' names, as listed.
        #[cfg(test)]
        pub const COMMANDS: &[(&str, &[&str])] = &[
            $( (stringify!($sname), &[$( stringify!($sarg) ),*]), )*
            $( (stringify!($name), &[$( stringify!($arg) ),*]), )*
            $( (stringify!($dname), &[$( stringify!($darg) ),*]), )*
        ];

        /// A command with its arguments read.
        #[allow(non_camel_case_types, reason = "one variant per command, named as the command")]
        enum Call {
            $( $sname { $( $sarg: $sty ),* }, )*
            $( $name { $( $arg: $ty ),* }, )*
            $( $dname { $( $darg: $dty ),* }, )*
        }

        /// Read `cmd`'s arguments, and refuse a desktop command without a
        /// desktop.
        fn prepare(cmd: &str, args: Value, has_desktop: bool) -> Result<Call, String> {
            // A call without arguments may send nothing at all.
            let args = if args.is_null() {
                Value::Object(serde_json::Map::new())
            } else {
                args
            };
            match cmd {
                $(
                    stringify!($sname) => {
                        #[derive(Deserialize)]
                        #[serde(rename_all = "camelCase")]
                        struct Args { $( $sarg: $sty ),* }
                        let Args { $( $sarg ),* } = read_args(cmd, args)?;
                        Ok(Call::$sname { $( $sarg ),* })
                    }
                )*
                $(
                    stringify!($name) => {
                        #[derive(Deserialize)]
                        #[serde(rename_all = "camelCase")]
                        struct Args { $( $arg: $ty ),* }
                        let Args { $( $arg ),* } = read_args(cmd, args)?;
                        Ok(Call::$name { $( $arg ),* })
                    }
                )*
                $(
                    stringify!($dname) => {
                        #[derive(Deserialize)]
                        #[serde(rename_all = "camelCase")]
                        struct Args { $( $darg: $dty ),* }
                        let Args { $( $darg ),* } = read_args(cmd, args)?;
                        if !has_desktop {
                            return Err(NOT_AVAILABLE.to_owned());
                        }
                        Ok(Call::$dname { $( $darg ),* })
                    }
                )*
                _ => Err(format!("command {cmd} not found")),
            }
        }

        async fn run(s: &Services, desktop: Option<&dyn Desktop>, call: Call) -> Result<Value, String> {
            match call {
                $(
                    Call::$sname { $( $sarg ),* } => {
                        finish::<$sret>(crate::engine::commands::$sname($( $sarg ),*).await)
                    }
                )*
                $(
                    Call::$name { $( $arg ),* } => {
                        finish::<$ret>(Box::pin(crate::engine::commands::$name(s, $( $arg ),*)).await)
                    }
                )*
                $(
                    Call::$dname { $( $darg ),* } => {
                        let desktop = desktop.ok_or_else(|| NOT_AVAILABLE.to_owned())?;
                        finish::<$dret>(
                            Box::pin(crate::engine::commands::$dname(s, desktop, $( $darg ),*))
                                .await,
                        )
                    }
                )*
            }
        }
    };
}

/// Run `cmd` with `args`.
///
/// # Errors
///
/// The command's own error, or a message for an unknown command, arguments it
/// can't read, or a desktop command without a desktop.
pub async fn dispatch(
    s: &Services,
    desktop: Option<&dyn Desktop>,
    cmd: &str,
    args: Value,
) -> Result<Value, String> {
    let call = prepare(cmd, args, desktop.is_some())?;
    Box::pin(run(s, desktop, call)).await
}

commands! {
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
        login_oauth(host: String, back: Option<String>, page: Option<String>) -> String;
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
        package_pull_outcome(namespace: String) -> PullPreview;
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
        get_autosync_snapshot() -> WatcherSnapshot;
        get_autopull_activity() -> Option<AutopullActivity>;
        update_fswatcher_settings(enabled: bool) -> ();
        update_log_settings(level: LogLevel) -> ();
        update_experimental_settings(entire_package_sync: Option<bool>, agent_tasks: Option<bool>) -> ();
        update_task_settings(settings: TaskSettings) -> ();
        update_design_settings(design: Design) -> ();
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
        take_navigation(page: String) -> Option<String>;
        dismiss_toast(id: u64) -> ();
        report_ui_panic(message: String) -> ();
        quit_prompt_shown(generation: u64) -> ();
        quit_cancel() -> ();
    }
    desktop {
        open_directory_picker() -> PathBuf;
        copy_to_clipboard(text: String, uri: Option<S3PackageUri>) -> String;
        check_for_update() -> Option<UpdateInfo>;
        download_and_install_update() -> ();
        quit_confirm() -> ();
    }
}

fn read_args<T: serde::de::DeserializeOwned>(cmd: &str, args: Value) -> Result<T, String> {
    serde_json::from_value(args).map_err(|err| format!("invalid args for command `{cmd}`: {err}"))
}

fn finish<T: serde::Serialize>(out: Result<T, String>) -> Result<Value, String> {
    serde_json::to_value(out?).map_err(|err| err.to_string())
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::atomic::{AtomicBool, Ordering};

    use async_trait::async_trait;

    use super::*;
    use crate::engine::Result;

    /// Every command is listed once.
    #[test]
    fn no_command_is_listed_twice() {
        let mut names: Vec<&str> = COMMANDS.iter().map(|(name, _)| *name).collect();
        let listed = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), listed);
    }

    #[test]
    fn every_listed_command_reads_its_arguments() {
        for (name, _) in COMMANDS {
            let Err(err) = prepare(name, Value::String("not args".into()), true) else {
                panic!("{name}: arguments of the wrong shape were accepted");
            };
            assert!(err.starts_with("invalid args"), "{name}: {err}");
        }
    }

    #[test]
    fn an_unknown_command_is_an_error() {
        let Err(err) = prepare("nope", Value::Null, true) else {
            panic!("an unknown command was accepted");
        };
        assert_eq!(err, "command nope not found");
    }

    #[test]
    fn arguments_are_read_by_their_camel_case_names() {
        let call = prepare(
            "update_experimental_settings",
            serde_json::json!({"entirePackageSync": true}),
            false,
        );
        assert!(matches!(
            call,
            Ok(Call::update_experimental_settings {
                entire_package_sync: Some(true),
                agent_tasks: None,
            })
        ));
        let Err(err) = prepare(
            "send_crash_report",
            serde_json::json!({"zip_path": "x"}),
            false,
        ) else {
            panic!("a snake_case argument was accepted");
        };
        assert!(err.contains("zipPath"), "{err}");
    }

    #[test]
    fn a_desktop_command_without_a_desktop_is_not_available() {
        let Err(err) = prepare("quit_confirm", Value::Null, false) else {
            panic!("a desktop command ran without a desktop");
        };
        assert_eq!(err, NOT_AVAILABLE);
    }

    #[derive(Default)]
    struct FakeDesktop {
        quit: AtomicBool,
    }

    #[async_trait]
    impl Desktop for FakeDesktop {
        async fn pick_folder(&self, _start: &Path) -> Result<Option<PathBuf>> {
            Ok(None)
        }

        fn copy_to_clipboard(&self, _text: &str) -> Result {
            Ok(())
        }

        async fn check_for_update(&self) -> std::result::Result<Option<String>, String> {
            Ok(Some("9.9.9".into()))
        }

        async fn install_update(&self) -> std::result::Result<(), String> {
            Ok(())
        }

        fn quit(&self) {
            self.quit.store(true, Ordering::SeqCst);
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_desktop_command_reaches_the_desktop() {
        let data_dir = tempfile::tempdir().expect("temp data dir");
        let s = Services::init(data_dir.path().to_path_buf(), semver::Version::new(0, 0, 0))
            .await
            .expect("services");
        let desktop = FakeDesktop::default();

        dispatch(&s, Some(&desktop), "quit_confirm", Value::Null)
            .await
            .expect("quit");
        assert!(desktop.quit.load(Ordering::SeqCst));

        let update = dispatch(&s, Some(&desktop), "check_for_update", Value::Null).await;
        assert_eq!(update, Ok(serde_json::json!({"version": "9.9.9"})));
    }
}
