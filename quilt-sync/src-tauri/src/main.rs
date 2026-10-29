// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
// `test_log`'s `#[test]` macro injects an init statement before the body, so it
// trips `items_after_statements` on leading `use`s/items in test fns. Enforce
// the lint in production; allow it only under `cfg(test)`.
#![cfg_attr(test, allow(clippy::items_after_statements))]
// A spawned task run's `Send` check nests the ACP client over quilt-rs's
// package reads, deeper than the default limit of 128.
#![recursion_limit = "256"]

use std::sync::Arc;

use tauri::Manager;
use tauri_plugin_deep_link::DeepLinkExt;

use crate::engine::telemetry::prelude::*;

mod desktop;
mod engine;

use desktop::tray;
use desktop::uri;
use engine::env;
use engine::services::Services;

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
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            // Registers the schemes from tauri.conf.json at runtime. macOS
            // takes them from the bundle's Info.plist, so this only logs there.
            #[cfg(desktop)]
            if let Err(err) = app.deep_link().register_all() {
                error!("Failed to register deep link schemes: {}", err);
            }

            let data_dir = app
                .path()
                .app_local_data_dir()
                .expect("Failed to resolve data dir");
            let version = app.package_info().version.clone();
            let services = tauri::async_runtime::block_on(Services::init(data_dir, version))?;
            desktop::forward_events(app.handle(), &services.events);
            app.manage(Arc::clone(&services));

            match tray::TrayController::install(app.handle(), services.tray_status.clone()) {
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
        .invoke_handler(desktop::commands::invoke_handler())
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            // Runs after every plugin's exit hook, so a plugin's own exit-time
            // logging still reaches the file. A drop hosted in a plugin would not.
            if matches!(event, tauri::RunEvent::Exit)
                && let Some(services) = app_handle.try_state::<Arc<Services>>()
            {
                services.app.logging.shutdown();
            }
        });
}
