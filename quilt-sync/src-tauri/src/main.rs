// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
// `test_log`'s `#[test]` macro injects an init statement before the body, so it
// trips `items_after_statements` on leading `use`s/items in test fns. Enforce
// the lint in production; allow it only under `cfg(test)`.
#![cfg_attr(test, allow(clippy::items_after_statements))]

use std::sync::Arc;

use tauri::Manager;
use tauri_plugin_deep_link::DeepLinkExt;
use tokio::sync;

use crate::telemetry::prelude::*;

mod app;
mod autopull;
mod changelog;
mod commands;
mod commit_message;
mod env;
mod error;
mod experimental_settings;
mod fswatcher;
mod log_settings;
mod model;
mod notify;
mod oauth;
mod publish_settings;
mod quilt;
mod quit;
mod routes;
mod telemetry;
mod toast;
mod tray;
mod uri;

use app::App;
use autopull::StatusReporter;
use autopull::Watcher;
use autopull::reporter::TauriEventReporter;
use error::Error;
use model::Model;

type Result<T = ()> = std::result::Result<T, Error>;

rust_i18n::i18n!("locales");

#[allow(clippy::too_many_lines, reason = "Tauri app builder and setup wiring")]
fn main() {
    env::init();

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            if let Err(err) = match argv.get(1) {
                Some(uri_str) => {
                    info!("Single-instance deep link: {:?}", uri_str);
                    uri::handle_deep_link_url(app, uri_str)
                }
                None => Ok(()),
            } {
                error!("{}", err);
            }
        }))
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            let package_info = app.package_info();

            // Resolved before the sinks are built: the crash client's event hook
            // captures the identity when the client is constructed, so it has to
            // be known first or crashes would go out unattributed.
            let data_dir = app
                .path()
                .app_local_data_dir()
                .expect("Failed to resolve data dir");

            let sinks = telemetry::Sinks::resolve();
            let telemetry = telemetry::Telemetry::new(
                &package_info.version,
                sinks,
                telemetry::InstallId::load(&data_dir),
                Some(telemetry::Buffer::new(&data_dir)),
            );

            // This is for runtime registering
            #[cfg(desktop)]
            for scheme in ["quilt+s3", "quilt"] {
                if let Err(err) = app.deep_link().register(scheme) {
                    error!("Failed to register deep link for {}: {}", scheme, err);
                }
            }

            let logging = telemetry::Telemetry::init_file_logging(&data_dir)?;

            telemetry.init();

            let publish_settings = tauri::async_runtime::block_on(publish_settings::init(
                &data_dir,
            ))
            .unwrap_or_else(|err| {
                error!("Failed to load publish settings, using defaults: {err}");
                std::sync::Arc::new(tokio::sync::RwLock::new(
                    publish_settings::PublishSettings::default(),
                ))
            });

            let autosync_settings = tauri::async_runtime::block_on(autopull::init_settings(
                &data_dir,
            ))
            .unwrap_or_else(|err| {
                error!("Failed to load autosync settings, using defaults: {err}");
                std::sync::Arc::new(tokio::sync::RwLock::new(
                    autopull::AutosyncSettings::default(),
                ))
            });
            let experimental_settings =
                tauri::async_runtime::block_on(experimental_settings::init(&data_dir))
                    .unwrap_or_else(|err| {
                        error!("Failed to load experimental settings, using defaults: {err}");
                        std::sync::Arc::new(tokio::sync::RwLock::new(
                            experimental_settings::ExperimentalSettings::default(),
                        ))
                    });
            let fswatcher_settings = tauri::async_runtime::block_on(fswatcher::init_settings(
                &data_dir,
            ))
            .unwrap_or_else(|err| {
                error!("Failed to load fswatcher settings, using defaults: {err}");
                std::sync::Arc::new(tokio::sync::RwLock::new(
                    fswatcher::FsWatcherSettings::default(),
                ))
            });

            // Only the app opens the package state store; the CLI never does.
            // One store for the last-change times and the walks' stat cache.
            let state_store = std::sync::Arc::new(quilt::state_store::PackageStateStore::new(
                quilt::paths::DomainPaths::new(data_dir.clone()).state_db(),
            ));
            let last_change =
                autopull::last_change::LastChange::shared(std::sync::Arc::clone(&state_store));
            app.manage(Model::create_with_state_store(data_dir, state_store));
            app.manage(commands::WorkflowRulesCache::default());
            app.manage(commands::RoleCache::default());
            app.manage(sync::Mutex::new(app.handle().clone()));
            app.manage(App::new(package_info, logging));
            app.manage(telemetry);
            app.manage(oauth::OAuthState::default());
            app.manage(toast::ToastCenter::new(Box::new(
                toast::TauriToastEmitter::new(app.handle().clone()),
            )));
            // The watcher reads `Model` via `app_handle.state::<Model>()`
            // so it can spawn after `Model` is registered above.
            // Telemetry wraps the UI reporter rather than replacing it: the
            // engine's outcomes reach the window exactly as before, and telemetry
            // observes them on the way past.
            let reporter: Arc<dyn StatusReporter> =
                Arc::new(autopull::reporter::TelemetryReporter::wrapping(
                    Arc::new(TauriEventReporter::new(app.handle().clone())),
                    app.handle().clone(),
                ));
            let (watcher, status_rx) = Watcher::spawn(
                app.handle().clone(),
                autosync_settings.clone(),
                publish_settings.clone(),
                reporter.clone(),
                last_change,
            );
            autopull::activity::spawn_forwarder(app.handle().clone(), watcher.subscribe_activity());
            app.manage(publish_settings);
            app.manage(autosync_settings);
            app.manage(experimental_settings);
            app.manage(watcher);
            // After the watcher is managed: the reactor reads its shared state.
            fswatcher::spawn(app.handle(), fswatcher_settings.clone(), &reporter);
            app.manage(fswatcher_settings);
            app.manage(quit::QuitGate::default());

            // The tray controller needs the autosync settings (for
            // close_to_tray), now in `app.state::<...>()`.
            match tray::TrayController::install(app.handle(), status_rx) {
                Ok(controller) => {
                    app.manage(controller);
                }
                Err(err) => {
                    // On Linux without an SNI/AppIndicator host the
                    // tray won't appear, but the app must still run.
                    warn!("tray icon unavailable: {err}");
                }
            }

            uri::setup_deep_link_handler(app.handle());

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::certify_latest,
            commands::erase_auth,
            commands::get_bucket_workflows,
            commands::get_commit_data,
            commands::get_installed_package_data,
            commands::get_installed_packages_list_data,
            commands::get_login_data,
            commands::get_login_error_data,
            commands::get_merge_data,
            commands::get_roles,
            commands::switch_role,
            commands::load_workflow_rules,
            commands::validate_commit_candidate,
            commands::get_settings_data,
            commands::get_setup_data,
            commands::debug_dot_quilt,
            commands::debug_logs,
            commands::open_data_dir,
            commands::open_home_dir,
            commands::run_gc,
            commands::measure_storage,
            commands::collect_diagnostic_logs,
            commands::send_crash_report,
            commands::login,
            commands::login_oauth,
            commands::open_directory_picker,
            commands::open_in_default_application,
            commands::open_in_file_browser,
            commands::copy_to_clipboard,
            commands::package_file_path,
            commands::open_in_web_browser,
            commands::package_commit,
            commands::package_commit_and_push,
            commands::package_install_paths,
            commands::package_set_sync_scope,
            commands::package_publish,
            commands::package_pull,
            commands::package_pull_outcome,
            commands::package_push,
            commands::update_publish_settings,
            commands::update_autosync_settings,
            commands::set_autosync_direction,
            commands::patch_autosync_settings,
            commands::get_autosync_snapshot,
            commands::get_autopull_activity,
            commands::get_toasts,
            commands::dismiss_toast,
            commands::update_fswatcher_settings,
            commands::update_log_settings,
            commands::update_experimental_settings,
            commands::get_main_page_packages,
            commands::get_main_page_recent_files,
            commands::get_main_page_accounts,
            commands::refresh_main_page_account,
            commands::refresh_main_page_package,
            commands::get_main_page_watcher,
            commands::get_package_page_data,
            commands::get_revision_history,
            commands::remove_revisions,
            commands::package_download_backlog,
            commands::refresh_package_status,
            commands::package_uninstall,
            commands::reset_local,
            commands::undo_commit,
            commands::reveal_in_file_browser,
            commands::set_remote,
            commands::package_create,
            commands::setup,
            commands::add_to_quiltignore,
            commands::test_quiltignore_pattern,
            commands::handle_remote_package,
            commands::get_revision_message,
            commands::check_for_update,
            commands::download_and_install_update,
            commands::report_ui_panic,
            commands::quit_prompt_shown,
            commands::quit_confirm,
            commands::quit_cancel,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            // Runs after every plugin's exit hook, so a plugin's own exit-time
            // logging still reaches the file. A drop hosted in a plugin would not.
            if matches!(event, tauri::RunEvent::Exit)
                && let Some(app) = app_handle.try_state::<App>()
            {
                app.logging.shutdown();
            }
        });
}
