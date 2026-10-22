// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
// `test_log`'s `#[test]` macro injects an init statement before the body, so it
// trips `items_after_statements` on leading `use`s/items in test fns. Enforce
// the lint in production; allow it only under `cfg(test)`.
#![cfg_attr(test, allow(clippy::items_after_statements))]

use std::sync::Arc;

use tauri::Emitter;
use tauri::Manager;
use tauri_plugin_deep_link::DeepLinkExt;
use tokio::sync;

use crate::telemetry::prelude::*;

mod app;
mod autopull;
mod changelog;
mod commands;
mod commit_message;
mod dispatch;
mod env;
mod error;
mod events;
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
mod services;
mod telemetry;
mod toast;
mod tray;
mod uri;

use error::Error;

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
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            let version = app.package_info().version.clone();
            let data_dir = app
                .path()
                .app_local_data_dir()
                .expect("Failed to resolve data dir");

            // This is for runtime registering
            #[cfg(desktop)]
            for scheme in ["quilt+s3", "quilt"] {
                if let Err(err) = app.deep_link().register(scheme) {
                    error!("Failed to register deep link for {}: {}", scheme, err);
                }
            }

            let services =
                tauri::async_runtime::block_on(services::Services::init(data_dir, version))?;
            app.manage(Arc::clone(&services));

            // The window hears every backend event, exactly as when the
            // engine emitted on the Tauri bus itself.
            let mut events = services.events.subscribe();
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    match events.recv().await {
                        Ok((name, payload)) => {
                            if let Err(err) = handle.emit(&name, payload) {
                                warn!("failed to emit {name}: {err}");
                            }
                        }
                        Err(sync::broadcast::error::RecvError::Lagged(missed)) => {
                            warn!("window missed {missed} backend events");
                        }
                        Err(sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
            });

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
        .invoke_handler(dispatch::invoke_handler())
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            // Runs after every plugin's exit hook, so a plugin's own exit-time
            // logging still reaches the file. A drop hosted in a plugin would not.
            if matches!(event, tauri::RunEvent::Exit)
                && let Some(services) = app_handle.try_state::<Arc<services::Services>>()
            {
                services.app.logging.shutdown();
            }
        });
}
