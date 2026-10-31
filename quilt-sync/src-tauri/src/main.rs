// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
// `test_log`'s `#[test]` macro injects an init statement before the body, so it
// trips `items_after_statements` on leading `use`s/items in test fns. Enforce
// the lint in production; allow it only under `cfg(test)`.
#![cfg_attr(test, allow(clippy::items_after_statements))]
// A spawned task run's `Send` check nests the ACP client over quilt-rs's
// package reads, deeper than the default limit of 128.
#![recursion_limit = "256"]

use std::process::ExitCode;
use std::sync::Arc;

use tauri::Manager;
use tauri_plugin_deep_link::DeepLinkExt;

use crate::engine::telemetry::prelude::*;

mod desktop;
mod engine;

use desktop::tray;
use desktop::uri;
use engine::env;
use engine::headless;
use engine::services::Services;
use engine::start;
use engine::start::Options;
use engine::start::Started;

rust_i18n::i18n!("locales");

/// Print to the terminal that started us. A Windows release build is a GUI
/// program with no console, so without this headless's link goes nowhere.
#[cfg(windows)]
fn attach_console() {
    // SAFETY: takes no pointers; failing (no parent console) is fine.
    unsafe {
        windows_sys::Win32::System::Console::AttachConsole(
            windows_sys::Win32::System::Console::ATTACH_PARENT_PROCESS,
        );
    }
}

#[cfg(not(windows))]
fn attach_console() {}

fn main() -> ExitCode {
    env::init();

    let args: Vec<String> = std::env::args().skip(1).collect();
    let headless = headless::parse(&args);
    if headless.is_some() {
        attach_console();
    }
    match headless {
        Some(Ok(args)) => return headless::run(args),
        Some(Err(usage)) => {
            eprintln!("{usage}");
            return ExitCode::from(2);
        }
        None => {}
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            if let Some(uri_str) = argv.get(1) {
                info!("Single-instance deep link: {:?}", uri_str);
                if let Err(err) = uri::handle_deep_link_url(app, uri_str) {
                    error!("{}", err);
                }
            } else {
                desktop::show_window(app);
            }
        }))
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            // Registers the schemes from tauri.conf.json at runtime. macOS
            // takes them from the bundle's Info.plist, so this only logs there.
            #[cfg(desktop)]
            if let Err(err) = app.deep_link().register_all() {
                error!("Failed to register deep link schemes: {}", err);
            }

            let data_dir = match headless::data_dir_from_env() {
                Some(dir) => dir,
                None => app.path().app_local_data_dir()?,
            };
            let version = app.package_info().version.clone();
            let desktop = desktop::commands::desktop(app.handle());
            let engine = match tauri::async_runtime::block_on(start::start(Options::desktop(
                data_dir, version, desktop,
            )))? {
                Started::Engine(engine) => engine,
                Started::Running(link) => {
                    // Another process is the engine for this data folder; its
                    // page is this launch's window.
                    match link {
                        Some(link) => {
                            if let Err(err) = opener::open_browser(&link) {
                                eprintln!("QuiltSync is already running at {link}: {err}");
                            }
                        }
                        None => eprintln!("QuiltSync is already running for this data folder"),
                    }
                    std::process::exit(0);
                }
            };
            let services = Arc::clone(&engine.services);
            app.manage(Arc::clone(&services));
            // The window is a client of the engine like any tab: it loads the
            // engine's page with its token.
            desktop::open_window(app.handle(), &engine.link)?;

            match tray::TrayController::install(
                app.handle(),
                services.tray_status.clone(),
                engine.link.clone(),
            ) {
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
            // Holds the data folder's lock for the life of the app.
            app.manage(engine);

            Ok(())
        })
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
    ExitCode::SUCCESS
}
