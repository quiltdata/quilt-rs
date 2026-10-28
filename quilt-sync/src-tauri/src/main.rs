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
mod design_settings;
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
mod tasks;
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

/// In a timing run, report the page's errors and the socket's big messages
/// and closes to `bench_report` on a socket of its own.
const BENCH_PROBE: &str = r"(function () {
  var t = new URLSearchParams(location.search).get('t');
  if (!t) { try { t = sessionStorage.getItem('quiltsync-token'); } catch (e) {} }
  if (!t || location.protocol !== 'http:') return;
  var OW = window.WebSocket, q = [], n = 1e9;
  var log = new OW('ws://' + location.host + '/ws?t=' + t);
  function send(m) {
    var s = JSON.stringify({ id: n++, cmd: 'bench_report', args: { text: 'page: ' + m } });
    if (log.readyState === 1) log.send(s); else q.push(s);
  }
  log.onopen = function () { q.forEach(function (s) { log.send(s); }); q = []; };
  ['error', 'warn'].forEach(function (k) {
    var f = console[k];
    console[k] = function () { send(k + ' ' + [].map.call(arguments, String).join(' ')); f.apply(console, arguments); };
  });
  document.addEventListener('visibilitychange', function () { send('visibility hidden=' + document.hidden); });
  window.addEventListener('error', function (ev) { send('onerror ' + ev.message); });
  window.addEventListener('unhandledrejection', function (ev) { send('rejection ' + ev.reason); });
  window.WebSocket = function (u, p) {
    var w = p === undefined ? new OW(u) : new OW(u, p);
    w.addEventListener('message', function (ev) {
      var d = ev.data;
      if (typeof d !== 'string' || d.length > 10000) send('message ' + (d.length || d.size) + ' hidden=' + document.hidden);
    });
    w.addEventListener('close', function (ev) { send('close ' + ev.code + ' ' + ev.reason); });
    return w;
  };
  window.WebSocket.prototype = OW.prototype;
})();";

/// The main window: the UI server's page at `url`, or the bundled page over
/// Tauri IPC with `ipc`, kept for comparing the two.
fn main_window(app: &tauri::App, url: &str, ipc: bool) -> Result {
    let target = if ipc {
        tauri::WebviewUrl::App("index.html".into())
    } else {
        tauri::WebviewUrl::External(url.parse().map_err(Error::ParseUrl)?)
    };
    let mut window = tauri::WebviewWindowBuilder::new(app, "main", target)
        .title("QuiltSync")
        .inner_size(1280.0, 900.0)
        .min_inner_size(640.0, 560.0)
        .resizable(true)
        .fullscreen(false)
        .use_https_scheme(true);
    // A timing run (see WEB-MVP.md): the page reads the namespace to measure.
    if let Ok(namespace) = std::env::var("QUILTSYNC_BENCH") {
        let script = format!(
            "window.__QS_BENCH__ = {};\n{BENCH_PROBE}",
            serde_json::Value::String(namespace)
        );
        // A hidden window (a locked screen, another space) would otherwise
        // suspend the page mid-run.
        window = window
            .initialization_script(&script)
            .background_throttling(tauri::utils::config::BackgroundThrottlingPolicy::Disabled);
    }
    window.build()?;
    Ok(())
}

fn main() {
    env::init();

    if let Some(opts) = parse_headless() {
        run_headless(&opts);
        return;
    }

    let mut builder = tauri::Builder::default();
    // A run on a throwaway data folder is a second copy next to the installed
    // app; it must not hand itself over to that one.
    if services::data_dir_override().is_none() {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            if let Err(err) = match argv.get(1) {
                Some(uri_str) => {
                    info!("Single-instance deep link: {:?}", uri_str);
                    uri::handle_deep_link_url(app, uri_str)
                }
                None => Ok(()),
            } {
                error!("{}", err);
            }
        }));
    }
    builder
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

            // Registers the schemes from tauri.conf.json at runtime. macOS
            // takes them from the bundle's Info.plist, so this only logs there.
            #[cfg(desktop)]
            if let Err(err) = app.deep_link().register_all() {
                error!("Failed to register deep link schemes: {}", err);
            }

            let services =
                tauri::async_runtime::block_on(services::Services::init(data_dir, version))?;
            app.manage(Arc::clone(&services));
            let _ = services.desktop.set(app.handle().clone());

            // The window is a page of the UI server, talking to the backend
            // over the same WebSocket a browser uses.
            let token = server::new_token();
            let addr = tauri::async_runtime::block_on(server::serve(
                Arc::clone(&services),
                std::net::SocketAddr::from(([127, 0, 0, 1], 0)),
                token.clone(),
            ))?;
            let url = format!("http://127.0.0.1:{}/?t={token}", addr.port());
            app.manage(tray::WebUiUrl(url.clone()));
            engine_lock::publish(&format!("http://127.0.0.1:{}", addr.port()), &token);
            let ipc_window = std::env::var_os("QUILTSYNC_IPC_WINDOW").is_some();
            main_window(app, &url, ipc_window)?;

            // Only the bundled page (`QUILTSYNC_IPC_WINDOW`) listens on the
            // Tauri bus.
            let mut events = services.events.subscribe();
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                if !ipc_window {
                    return;
                }
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
