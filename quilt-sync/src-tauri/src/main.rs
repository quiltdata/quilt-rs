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
mod engine_lock;
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
mod server;
mod services;
mod telemetry;
mod toast;
mod tray;
mod update_feed;
mod uri;

use error::Error;

type Result<T = ()> = std::result::Result<T, Error>;

rust_i18n::i18n!("locales");

/// Command-line options for a run without a window.
struct Headless {
    bind: std::net::IpAddr,
    port: u16,
    data_dir: Option<std::path::PathBuf>,
}

/// `QUILTSYNC_DATA_DIR`, which replaces the data folder in both modes.
fn data_dir_from_env() -> Option<std::path::PathBuf> {
    std::env::var_os("QUILTSYNC_DATA_DIR")
        .filter(|v| !v.is_empty())
        .map(std::path::PathBuf::from)
}

/// `--headless [--bind <ip>] [--port <n>] [--data-dir <path>]`, or `None`
/// for the desktop app.
fn parse_headless() -> Option<Headless> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if !args.iter().any(|a| a == "--headless") {
        return None;
    }
    let value = |flag: &str| {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
    };
    let bind = value("--bind").map_or(std::net::IpAddr::from([127, 0, 0, 1]), |v| {
        v.parse().expect("--bind takes an IP address")
    });
    let port = value("--port").map_or(7878, |v| v.parse().expect("--port takes a number"));
    let data_dir = value("--data-dir").map(std::path::PathBuf::from);
    Some(Headless {
        bind,
        port,
        data_dir,
    })
}

/// Serve the UI to a browser with no window and no Tauri, until Ctrl-C.
/// Exits with code 2 when another engine already runs on the data folder.
fn run_headless(opts: &Headless) {
    let data_dir = opts
        .data_dir
        .clone()
        .or_else(data_dir_from_env)
        .unwrap_or_else(services::default_data_dir);
    match engine_lock::acquire_or_wait_for_info(&data_dir) {
        Ok(engine_lock::Acquired::Held(lock)) => engine_lock::hold(lock),
        Ok(engine_lock::Acquired::Running(info)) => {
            match info {
                Some(info) => println!("QuiltSync is already running: {}", info.url_with_token()),
                None => println!(
                    "QuiltSync is already running on {} (its address is unknown)",
                    data_dir.display()
                ),
            }
            std::process::exit(2);
        }
        Err(err) => {
            eprintln!("failed to lock {}: {err}", data_dir.display());
            std::process::exit(1);
        }
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to build the tokio runtime");
    runtime.block_on(async {
        let version = semver::Version::parse(env!("CARGO_PKG_VERSION")).expect("crate version");
        let services = services::Services::init(data_dir, version)
            .await
            .expect("failed to start the services");
        let token = server::new_token();
        let addr = server::serve(
            Arc::clone(&services),
            std::net::SocketAddr::new(opts.bind, opts.port),
            token.clone(),
        )
        .await
        .expect("failed to start the server");
        let host = if addr.ip().is_loopback() || addr.ip().is_unspecified() {
            format!("localhost:{}", addr.port())
        } else {
            addr.to_string()
        };
        engine_lock::publish(&format!("http://{host}"), &token);
        println!("QuiltSync headless: http://{host}/?t={token}");
        if !addr.ip().is_loopback() {
            println!("The Host header check is off: the server is not bound to loopback.");
        }
        if addr.ip().is_unspecified() {
            println!(
                "Warning: listening on all interfaces over plain HTTP; the token and your data \
                 travel unencrypted. Prefer the default bind and an SSH tunnel, \
                 e.g. `ssh -L {port}:localhost:{port} <host>`.",
                port = addr.port()
            );
        }
        if let Err(err) = tokio::signal::ctrl_c().await {
            error!("failed to wait for Ctrl-C: {err}");
        }
        services.app.logging.shutdown();
    });
}

/// Become the engine for `data_dir`, or, when another engine (headless) holds
/// it, open the browser at that engine and exit without starting one.
fn take_engine_lock_or_open_browser(data_dir: &std::path::Path) {
    match engine_lock::acquire_or_wait_for_info(data_dir) {
        Ok(engine_lock::Acquired::Held(lock)) => engine_lock::hold(lock),
        Ok(engine_lock::Acquired::Running(info)) => {
            if let Some(info) = info {
                info!("another engine runs on {}; opening it", data_dir.display());
                if let Err(err) = opener::open_browser(info.url_with_token()) {
                    error!("failed to open the browser: {err}");
                }
            } else {
                error!("another engine runs on {}", data_dir.display());
            }
            std::process::exit(0);
        }
        Err(err) => {
            // Starting anyway beats not starting at all.
            error!("failed to lock {}: {err}", data_dir.display());
        }
    }
}

fn main() {
    env::init();

    if let Some(opts) = parse_headless() {
        run_headless(&opts);
        return;
    }

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
            let data_dir = data_dir_from_env().unwrap_or_else(|| {
                app.path()
                    .app_local_data_dir()
                    .expect("Failed to resolve data dir")
            });
            // A second desktop app never gets here (the single-instance
            // plugin stops it); a headless engine on this folder does not
            // stop the plugin, so hand the user to it in the browser.
            take_engine_lock_or_open_browser(&data_dir);

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
