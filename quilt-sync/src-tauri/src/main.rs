// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

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
mod fswatcher;
mod model;
mod notify;
mod oauth;
mod publish_settings;
mod quilt;
mod routes;
mod telemetry;
mod tray;
mod uri;

use app::App;
use autopull::StatusReporter;
use autopull::Watcher;
use autopull::WindowMode;
use autopull::reporter::TauriEventReporter;
use error::Error;
use model::Model;

type Result<T = ()> = std::result::Result<T, Error>;

rust_i18n::i18n!("locales");

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
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            let package_info = app.package_info();
            let enable = if cfg!(debug_assertions) {
                None
            } else {
                Some(())
            };
            let telemetry = telemetry::Telemetry::new(&package_info.version, enable);

            // This is for runtime registering
            #[cfg(desktop)]
            for scheme in ["quilt+s3", "quilt"] {
                if let Err(err) = app.deep_link().register(scheme) {
                    error!("Failed to register deep link for {}: {}", scheme, err);
                }
            }

            let data_dir = app
                .path()
                .app_local_data_dir()
                .expect("Failed to resolve data dir");

            let logs_dir = telemetry.init_file_logging(&data_dir)?;

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
            let fswatcher_settings = tauri::async_runtime::block_on(fswatcher::init_settings(
                &data_dir,
            ))
            .unwrap_or_else(|err| {
                error!("Failed to load fswatcher settings, using defaults: {err}");
                std::sync::Arc::new(tokio::sync::RwLock::new(
                    fswatcher::FsWatcherSettings::default(),
                ))
            });
            let window_mode = autopull::create_window_mode();

            app.manage(Model::create(data_dir));
            app.manage(sync::Mutex::new(app.handle().clone()));
            app.manage(App::new(package_info, logs_dir));
            app.manage(telemetry);
            app.manage(oauth::OAuthState::default());
            // The watcher reads `Model` via `app_handle.state::<Model>()`
            // so it can spawn after `Model` is registered above.
            let reporter: Arc<dyn StatusReporter> =
                Arc::new(TauriEventReporter::new(app.handle().clone()));
            let (watcher, status_rx) = Watcher::spawn(
                app.handle().clone(),
                autosync_settings.clone(),
                window_mode.clone(),
                publish_settings.clone(),
                reporter.clone(),
            );
            fswatcher::spawn(app.handle(), fswatcher_settings.clone(), &reporter);
            app.manage(publish_settings);
            app.manage(autosync_settings);
            app.manage(fswatcher_settings);
            app.manage(window_mode);
            app.manage(watcher);

            // The tray controller needs the autosync settings (for
            // close_to_tray) and the window-mode state — both are now
            // in `app.state::<...>()`.
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
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::Focused(focused) = event {
                // If the window is hidden, leave window_mode at
                // WindowMode::Closed — a focus event during the
                // hide-to-tray transition would otherwise overwrite it.
                if !window.is_visible().unwrap_or(true) {
                    return;
                }
                let mode = if *focused {
                    WindowMode::Focused
                } else {
                    WindowMode::Unfocused
                };
                let handle = window.app_handle().clone();
                tauri::async_runtime::spawn(async move {
                    let watcher = handle.state::<Watcher>();
                    watcher.set_window_mode(mode).await;
                });
            }
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
            commands::get_settings_data,
            commands::get_setup_data,
            commands::debug_dot_quilt,
            commands::debug_logs,
            commands::open_data_dir,
            commands::open_home_dir,
            commands::collect_diagnostic_logs,
            commands::send_crash_report,
            commands::login,
            commands::login_oauth,
            commands::open_directory_picker,
            commands::open_in_default_application,
            commands::open_in_file_browser,
            commands::open_in_web_browser,
            commands::package_commit,
            commands::package_commit_and_push,
            commands::package_install_paths,
            commands::package_publish,
            commands::package_pull,
            commands::package_push,
            commands::update_publish_settings,
            commands::update_autosync_settings,
            commands::get_autosync_snapshot,
            commands::update_fswatcher_settings,
            commands::refresh_package_status,
            commands::package_uninstall,
            commands::reset_local,
            commands::reveal_in_file_browser,
            commands::set_remote,
            commands::package_create,
            commands::setup,
            commands::add_to_quiltignore,
            commands::test_quiltignore_pattern,
            commands::handle_remote_package,
            commands::check_for_update,
            commands::download_and_install_update,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
