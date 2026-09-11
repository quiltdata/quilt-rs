use std::fmt::Write;
use std::time::Duration;

use tauri::AppHandle;
use tauri::Emitter;
use tauri::Manager;
use tauri::WindowEvent;
use tauri::async_runtime;
use tauri::image::Image;
use tauri::menu::Menu;
use tauri::menu::MenuItem;
use tauri::tray::MouseButton;
use tauri::tray::MouseButtonState;
use tauri::tray::TrayIcon;
use tauri::tray::TrayIconBuilder;
use tauri::tray::TrayIconEvent;
use thiserror::Error;
use tokio::sync::watch;

use crate::telemetry::prelude::*;

use crate::autopull::SharedAutosyncSettings;
use crate::autopull::SharedWindowMode;
use crate::autopull::SyncTrayStatus;
use crate::autopull::TrayMode;
use crate::autopull::Watcher;
use crate::autopull::WindowMode;
use crate::quit::QuitAction;
use crate::quit::QuitGate;

const TRAY_ICON_ASSET: &str = "icons/tray/trayicon.png";

#[derive(Debug, Error)]
pub enum TrayError {
    #[error(transparent)]
    Tauri(#[from] tauri::Error),
    #[error("tray icon asset missing: {0}")]
    MissingIcon(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Owns the tray icon, the status-listener task, and the per-window
/// close handler. Hold one of these for the lifetime of the app —
/// dropping it aborts the listener task and removes the tray icon.
pub struct TrayController {
    _tray: TrayIcon,
    _listener: async_runtime::JoinHandle<()>,
}

const MENU_ID_OPEN: &str = "tray-open";
const MENU_ID_QUIT: &str = "tray-quit";

impl TrayController {
    pub fn install(
        app: &AppHandle,
        status_rx: watch::Receiver<SyncTrayStatus>,
    ) -> Result<Self, TrayError> {
        let open_item = MenuItem::with_id(app, MENU_ID_OPEN, "Open Quilt", true, None::<&str>)?;
        let quit_item = MenuItem::with_id(app, MENU_ID_QUIT, "Quit", true, None::<&str>)?;
        let menu = Menu::with_items(app, &[&open_item, &quit_item])?;

        let icon = load_tray_icon(app)?;
        let tray = TrayIconBuilder::new()
            .icon(icon)
            .menu(&menu)
            .tooltip("Quilt — idle")
            .on_menu_event(|app, event| match event.id().as_ref() {
                MENU_ID_OPEN => show_main_window(app),
                MENU_ID_QUIT => request_quit(app),
                _ => {}
            })
            .on_tray_icon_event(|tray, event| {
                // Linux + Windows convention: left-click restores the
                // window. macOS users get the menu on any click and
                // never receive this event in the same shape, so the
                // gate is harmless there.
                if let TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } = event
                {
                    show_main_window(tray.app_handle());
                }
            })
            .build(app)?;

        let listener = spawn_status_listener(tray.clone(), status_rx);
        let _ = open_item;
        let _ = quit_item;

        install_close_handler(app);

        Ok(Self {
            _tray: tray,
            _listener: listener,
        })
    }
}

fn load_tray_icon(app: &AppHandle) -> Result<Image<'static>, TrayError> {
    let path = app
        .path()
        .resolve(TRAY_ICON_ASSET, tauri::path::BaseDirectory::Resource)
        .map_err(|_| TrayError::MissingIcon(TRAY_ICON_ASSET.to_string()))?;
    let bytes = std::fs::read(&path)?;
    let image = Image::from_bytes(&bytes)?.to_owned();
    Ok(image)
}

fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
        #[cfg(target_os = "macos")]
        {
            let _ = app.set_activation_policy(tauri::ActivationPolicy::Regular);
        }
        let handle = app.clone();
        async_runtime::spawn(async move {
            let mode = handle.state::<SharedWindowMode>();
            *mode.write().await = WindowMode::Focused;
        });
    }
}

fn spawn_status_listener(
    tray: TrayIcon,
    mut status_rx: watch::Receiver<SyncTrayStatus>,
) -> async_runtime::JoinHandle<()> {
    async_runtime::spawn(async move {
        // Apply the initial state once before waiting for changes.
        apply_status(&tray, &status_rx.borrow());
        while status_rx.changed().await.is_ok() {
            apply_status(&tray, &status_rx.borrow());
        }
    })
}

fn apply_status(tray: &TrayIcon, status: &SyncTrayStatus) {
    // The tray image is a single Quilt logo; the per-mode signal lives
    // in the tooltip so the icon stays recognisable across themes.
    let label = match status.mode {
        TrayMode::Idle => "Quilt — idle",
        TrayMode::Syncing => "Quilt — syncing",
        TrayMode::Paused => "Quilt — paused",
        TrayMode::Error => "Quilt — error",
    };
    let mut tooltip = label.to_string();
    if status.pending_changes > 0 {
        let _ = write!(
            tooltip,
            "\n{} package(s) with local changes",
            status.pending_changes,
        );
    }
    if let Some(error) = status.error.as_deref() {
        let _ = write!(tooltip, "\n{error}");
    }
    let _ = tray.set_tooltip(Some(&tooltip));
}

/// Event the window listens for to raise the prompt. Kept in lockstep with
/// the UI's `listen(...)` call.
const QUIT_PROMPT_EVENT: &str = "quit-prompt";

/// How long a raised prompt has to report itself on screen before the quit is
/// treated as unanswerable. Long enough for a webview that is merely busy,
/// short enough that a click on Quit never looks ignored.
const PROMPT_GRACE: Duration = Duration::from_millis(1500);

/// Route a quit request through the gate: exit when nothing is being written,
/// otherwise ask and let the answer decide.
fn request_quit(app: &AppHandle) {
    let applying = app
        .try_state::<Watcher>()
        .is_some_and(|watcher| watcher.apply_in_progress());
    let Some(gate) = app.try_state::<QuitGate>() else {
        // Nothing to ask with. A quit that cannot be questioned proceeds.
        app.exit(0);
        return;
    };
    match gate.request(applying) {
        QuitAction::Exit => app.exit(0),
        QuitAction::AlreadyPrompting => {}
        QuitAction::Prompt(generation) => prompt_before_quit(app, generation),
    }
}

/// Raise the window and ask. The tray path usually has it hidden, and an
/// overlay needs something to sit on.
///
/// The failure direction is deliberate: if the ask cannot be delivered, or
/// nothing reports it on screen within [`PROMPT_GRACE`], the quit goes
/// through. A Quit that appears to do nothing is a worse outcome than an
/// interrupted apply — the apply is what the reconcile is being made to
/// survive, whereas an app that will not close has no remedy at all.
fn prompt_before_quit(app: &AppHandle, generation: u64) {
    show_main_window(app);
    if let Err(err) = app.emit(QUIT_PROMPT_EVENT, generation) {
        warn!("quit prompt could not be raised, quitting: {err}");
        app.exit(0);
        return;
    }
    let handle = app.clone();
    async_runtime::spawn(async move {
        tokio::time::sleep(PROMPT_GRACE).await;
        if handle
            .try_state::<QuitGate>()
            .is_some_and(|gate| gate.unanswerable(generation))
        {
            warn!("quit prompt never reported itself on screen, quitting");
            handle.exit(0);
        }
    });
}

fn install_close_handler(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let handle = app.clone();
    window.on_window_event(move |event| {
        if let WindowEvent::CloseRequested { api, .. } = event {
            let close_to_tray = {
                let settings = handle.state::<SharedAutosyncSettings>();
                let snapshot = async_runtime::block_on(settings.read());
                snapshot.close_to_tray
            };
            if !close_to_tray {
                // Closing IS quitting in this configuration, so it gets the
                // same question. Refuse the close either way and let the
                // answer decide: `prevent_close` then exit is the only way to
                // ask, since the event itself cannot be awaited.
                api.prevent_close();
                request_quit(&handle);
                return;
            }
            api.prevent_close();
            if let Some(window) = handle.get_webview_window("main") {
                let _ = window.hide();
            }
            #[cfg(target_os = "macos")]
            {
                let _ = handle.set_activation_policy(tauri::ActivationPolicy::Accessory);
            }
            let mode_handle = handle.clone();
            async_runtime::spawn(async move {
                let mode = mode_handle.state::<SharedWindowMode>();
                *mode.write().await = WindowMode::Closed;
            });
        }
    });
}
