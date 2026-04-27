// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use tauri::Manager;
use tauri_plugin_deep_link::DeepLinkExt;
use tokio::sync;

use crate::telemetry::prelude::*;

mod app;
mod changelog;
mod commands;
mod commit_message;
mod env;
mod error;
mod model;
mod notify;
mod oauth;
mod publish_settings;
mod quilt;
mod routes;
mod telemetry;
mod uri;

use app::App;
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
            let enable = match cfg!(debug_assertions) {
                true => None,
                false => Some(()),
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

            app.manage(Model::create(data_dir));
            app.manage(sync::Mutex::new(app.handle().clone()));
            app.manage(App::new(package_info, logs_dir));
            app.manage(telemetry);
            app.manage(oauth::OAuthState::default());
            app.manage(publish_settings);

            uri::setup_deep_link_handler(app.handle());

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::certify_latest,
            commands::erase_auth,
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
